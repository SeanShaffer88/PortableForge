//! One logical canvas, scaled with the native client area.

pub(super) const CANVAS: [f32; 2] = [1060.0, 580.0];

pub(super) fn fit(client: [f32; 2]) -> Option<f32> {
    let scale = (client[0] / CANVAS[0]).min(client[1] / CANVAS[1]);
    (scale.is_finite() && scale > 0.0).then_some(scale)
}

#[cfg(windows)]
pub(super) use native::install;

#[cfg(windows)]
mod native {
    use super::{fit, CANVAS};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::{io, mem::size_of, ptr::null_mut};
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        },
        UI::{
            HiDpi::GetDpiForWindow,
            Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
            WindowsAndMessaging::{
                GetClientRect, GetWindowRect, SetWindowPos, HTBORDER, HTBOTTOM, HTLEFT, HTRIGHT,
                HTTOP, SC_MAXIMIZE, SWP_NOACTIVATE, SWP_NOZORDER, WMSZ_BOTTOMLEFT,
                WMSZ_BOTTOMRIGHT, WMSZ_TOPLEFT, WMSZ_TOPRIGHT, WM_NCDESTROY, WM_NCHITTEST,
                WM_SIZING, WM_SYSCOMMAND,
            },
        },
    };

    const SUBCLASS_ID: usize = 1;
    const MIN_SCALE: f32 = 0.75;

    pub(in crate::wizard) fn install(context: &eframe::CreationContext<'_>) -> io::Result<()> {
        let handle = context.window_handle().map_err(io::Error::other)?;
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return Err(io::Error::other(
                "PortableForge needs a Win32 window handle",
            ));
        };
        let hwnd = handle.hwnd.get() as HWND;
        // The handle belongs to this event-loop thread. The subclass owns no app data.
        if unsafe { SetWindowSubclass(hwnd, Some(window_proc), SUBCLASS_ID, 0) } == 0 {
            return Err(io::Error::other(
                "Could not enable proportional window resize",
            ));
        }
        if let Some(metrics) = Metrics::read(hwnd) {
            let target = metrics.initial_rect();
            // Resize during window creation, before handing control to the UI.
            unsafe {
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    target.left,
                    target.top,
                    width(target),
                    height(target),
                    SWP_NOACTIVATE | SWP_NOZORDER,
                );
            }
            if let Some(metrics) = Metrics::read(hwnd) {
                if let Some(scale) = fit([
                    metrics.client[0] as f32 / metrics.dpi,
                    metrics.client[1] as f32 / metrics.dpi,
                ]) {
                    context.egui_ctx.set_zoom_factor(scale);
                }
            }
        }
        Ok(())
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _: usize,
        _: usize,
    ) -> LRESULT {
        match message {
            WM_NCHITTEST => {
                // Preserve the native title bar, caption buttons and corner hit targets.
                let hit = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
                if matches!(hit as u32, HTLEFT | HTRIGHT | HTTOP | HTBOTTOM) {
                    return HTBORDER as LRESULT;
                }
                return hit;
            }
            WM_SIZING if lparam != 0 => {
                // WM_SIZING supplies an in/out RECT for this callback only.
                let proposed = unsafe { &mut *(lparam as *mut RECT) };
                if let Some(metrics) = Metrics::read(hwnd) {
                    *proposed = metrics.drag_rect(*proposed, wparam as u32);
                    return 1;
                }
            }
            WM_SYSCOMMAND if wparam & 0xfff0 == SC_MAXIMIZE as usize => return 0,
            WM_NCDESTROY => {
                // Keep the parent/default procedure in the destruction chain.
                unsafe { RemoveWindowSubclass(hwnd, Some(window_proc), SUBCLASS_ID) };
            }
            _ => {}
        }
        unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
    }

    #[derive(Clone, Copy)]
    struct Metrics {
        window: RECT,
        work: RECT,
        client: [i32; 2],
        frame: [i32; 2],
        dpi: f32,
    }

    impl Metrics {
        fn read(hwnd: HWND) -> Option<Self> {
            let mut window = RECT::default();
            let mut client = RECT::default();
            let mut monitor = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            // These calls only inspect the live window on its creation thread.
            let dpi = unsafe {
                let display = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
                if GetWindowRect(hwnd, &mut window) == 0
                    || GetClientRect(hwnd, &mut client) == 0
                    || GetMonitorInfoW(display, &mut monitor) == 0
                {
                    return None;
                }
                GetDpiForWindow(hwnd)
            };
            Some(Self {
                window,
                work: monitor.rcWork,
                client: [width(client), height(client)],
                frame: [
                    (width(window) - width(client)).max(0),
                    (height(window) - height(client)).max(0),
                ],
                dpi: dpi.max(96) as f32 / 96.0,
            })
        }

        fn inner_scale(self, outer: [i32; 2]) -> [f32; 2] {
            [
                (outer[0] - self.frame[0]).max(1) as f32 / (CANVAS[0] * self.dpi),
                (outer[1] - self.frame[1]).max(1) as f32 / (CANVAS[1] * self.dpi),
            ]
        }

        fn outer_size(self, scale: f32) -> [i32; 2] {
            // Ceil both axes to preserve at least the canonical logical canvas.
            [
                (CANVAS[0] * self.dpi * scale).ceil() as i32 + self.frame[0],
                (CANVAS[1] * self.dpi * scale).ceil() as i32 + self.frame[1],
            ]
        }

        fn work_scale(self) -> f32 {
            let maximum = self.inner_scale([width(self.work), height(self.work)]);
            maximum[0].min(maximum[1])
        }

        fn initial_rect(self) -> RECT {
            let [width, height] = self.outer_size(self.work_scale().min(1.0));
            self.keep_visible(RECT {
                left: self.window.left,
                top: self.window.top,
                right: self.window.left + width,
                bottom: self.window.top + height,
            })
        }

        fn drag_rect(self, proposed: RECT, edge: u32) -> RECT {
            let (left, top) = match edge {
                WMSZ_TOPLEFT => (true, true),
                WMSZ_TOPRIGHT => (false, true),
                WMSZ_BOTTOMLEFT => (true, false),
                WMSZ_BOTTOMRIGHT => (false, false),
                // The keyboard Size command can still request straight-edge sizing.
                _ => return self.window,
            };
            let [x, y] = self.inner_scale([width(proposed), height(proposed)]);
            // Project the dragged corner onto the aspect-ratio diagonal. The same
            // pointer position now gives the same size, even after a live repaint.
            let width_weight = CANVAS[0] * CANVAS[0];
            let height_weight = CANVAS[1] * CANVAS[1];
            let requested = (x * width_weight + y * height_weight) / (width_weight + height_weight);
            let anchor = [
                if left {
                    self.window.right
                } else {
                    self.window.left
                },
                if top {
                    self.window.bottom
                } else {
                    self.window.top
                },
            ];
            let available = [
                if left {
                    anchor[0] - self.work.left
                } else {
                    self.work.right - anchor[0]
                },
                if top {
                    anchor[1] - self.work.top
                } else {
                    self.work.bottom - anchor[1]
                },
            ];
            let [max_x, max_y] = self.inner_scale(available);
            let maximum = self.work_scale();
            let minimum = MIN_SCALE.min(maximum);
            let anchor_max = max_x.min(max_y);
            let [width, height] =
                self.outer_size(requested.clamp(minimum, maximum.min(anchor_max).max(minimum)));
            let x = if left { anchor[0] - width } else { anchor[0] };
            let y = if top { anchor[1] - height } else { anchor[1] };
            self.keep_visible(RECT {
                left: x,
                top: y,
                right: x + width,
                bottom: y + height,
            })
        }

        fn keep_visible(self, rect: RECT) -> RECT {
            let w = width(rect).min(width(self.work));
            let h = height(rect).min(height(self.work));
            let x = rect.left.clamp(self.work.left, self.work.right - w);
            let y = rect.top.clamp(self.work.top, self.work.bottom - h);
            RECT {
                left: x,
                top: y,
                right: x + w,
                bottom: y + h,
            }
        }
    }

    fn width(rect: RECT) -> i32 {
        rect.right - rect.left
    }
    fn height(rect: RECT) -> i32 {
        rect.bottom - rect.top
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
            RECT {
                left,
                top,
                right,
                bottom,
            }
        }

        #[test]
        fn corner_drag_preserves_the_opposite_corner_at_each_scale() {
            let metrics = Metrics {
                window: rect(1000, 800, 2076, 1419),
                work: rect(-1000, -400, 3700, 2600),
                client: CANVAS.map(|dimension| dimension as i32),
                frame: [16, 39],
                dpi: 1.0,
            };
            for scale in [0.8, 1.0, 1.25, 1.5] {
                let [w, h] = metrics.outer_size(scale);
                let window = metrics.window;
                for (edge, left, top) in [
                    (WMSZ_TOPLEFT, true, true),
                    (WMSZ_TOPRIGHT, false, true),
                    (WMSZ_BOTTOMLEFT, true, false),
                    (WMSZ_BOTTOMRIGHT, false, false),
                ] {
                    let x = if left { window.right - w } else { window.left };
                    let y = if top { window.bottom - h } else { window.top };
                    let expected = rect(x, y, x + w, y + h);
                    let adjusted = metrics.drag_rect(expected, edge);
                    assert_eq!(adjusted.left, x);
                    assert_eq!(adjusted.top, y);
                    assert!((width(adjusted) - w).abs() <= 1);
                    assert!((height(adjusted) - h).abs() <= 1);
                }
            }
        }

        #[test]
        fn initial_and_corner_sizes_fit_a_work_area_smaller_than_the_nominal_minimum() {
            let metrics = Metrics {
                window: rect(-380, -210, 695, 409),
                work: rect(-400, -240, 240, 160),
                client: CANVAS.map(|dimension| dimension as i32),
                frame: [16, 39],
                dpi: 1.25,
            };
            let initial = metrics.initial_rect();
            let dragged = metrics.drag_rect(rect(-380, -210, 1500, 1000), WMSZ_BOTTOMRIGHT);
            for size in [initial, dragged] {
                assert!(size.left >= metrics.work.left && size.top >= metrics.work.top);
                assert!(size.right <= metrics.work.right && size.bottom <= metrics.work.bottom);
                let [x, y] = metrics.inner_scale([width(size), height(size)]);
                assert!((x - y).abs() < 0.002);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scales_width_height_and_text_together() {
        for scale in [0.8, 1.0, 1.25, 1.5] {
            let client = CANVAS.map(|dimension| dimension * scale);
            assert_eq!(fit(client), Some(scale));
            assert_eq!(fit([client[0] + 1.0, client[1]]), Some(scale));
        }
        assert_eq!(fit([0.0, 0.0]), None);
    }

    #[test]
    fn wizard_layout_converges_on_one_canvas_across_native_dpi_and_resize_changes() {
        use crate::wizard::WizardApp;
        use eframe::egui;

        let ctx = egui::Context::default();
        let mut wizard = WizardApp {
            proportional_window: true,
            ..WizardApp::default()
        };
        wizard.settings.text_scale = 1.5;
        for dpi in [1.0, 1.25, 1.5] {
            for scale in [0.8, 1.0, 1.25, 1.5] {
                let physical = egui::Vec2::from(CANVAS) * (dpi * scale);
                for _ in 0..4 {
                    let logical = physical / (dpi * ctx.zoom_factor());
                    let mut input = egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, logical)),
                        ..Default::default()
                    };
                    input
                        .viewports
                        .get_mut(&egui::ViewportId::ROOT)
                        .unwrap()
                        .native_pixels_per_point = Some(dpi);
                    let output = ctx.run(input, |ctx| wizard.apply_layout_zoom(ctx));
                    assert!(output.viewport_output[&egui::ViewportId::ROOT]
                        .commands
                        .is_empty());
                }
                assert!((ctx.zoom_factor() - scale).abs() < 0.0001);
                let size = ctx.content_rect().size();
                assert!(
                    (size.x - CANVAS[0]).abs() < 1.0,
                    "dpi={dpi} scale={scale} size={size:?}"
                );
                assert!(
                    (size.y - CANVAS[1]).abs() < 1.0,
                    "dpi={dpi} scale={scale} size={size:?}"
                );
            }
        }
    }
}
