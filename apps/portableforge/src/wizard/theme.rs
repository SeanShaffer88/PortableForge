//! Native egui mapping of the TesselGUI revision 2 Rust export.

use eframe::egui;
use portableforge_core::settings::Appearance;

// The pinned upstream export is generated. Keep its bytes intact.
#[rustfmt::skip]
#[path = "theme/tessel.rs"]
mod tessel;
use tessel::{Density, Palette};

#[derive(Default)]
pub(super) struct State {
    last_appearance: Option<Appearance>,
    observed_system: Option<Option<egui::Theme>>,
    focused: bool,
    high_contrast: Option<Palette>,
    applied: Option<(Palette, Density)>,
}

const RADIUS: u8 = 4;
const SEMIBOLD: &str = "Segoe UI Semibold";

pub(super) fn rgb([red, green, blue]: [u8; 3]) -> egui::Color32 {
    egui::Color32::from_rgb(red, green, blue)
}

fn palette_id() -> egui::Id {
    egui::Id::new("portableforge-tessel-palette")
}

pub(super) fn palette(ctx: &egui::Context) -> Palette {
    ctx.data(|data| data.get_temp::<Palette>(palette_id()))
        .unwrap_or(tessel::DARK)
}

pub(super) fn success(ui: &egui::Ui) -> egui::Color32 {
    rgb(palette(ui.ctx()).success)
}

pub(super) fn info(ui: &egui::Ui) -> egui::Color32 {
    rgb(palette(ui.ctx()).info)
}

pub(super) fn focus_outline(ui: &egui::Ui, response: &egui::Response) {
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect.expand(tessel::FOCUS_OFFSET),
            RADIUS,
            egui::Stroke::new(tessel::FOCUS_OUTLINE, rgb(palette(ui.ctx()).focus)),
            egui::StrokeKind::Middle,
        );
    }
}

pub(super) fn compact_row_height() -> f32 {
    tessel::COMPACT.row_min_height
}

impl State {
    pub(super) fn apply(&mut self, ctx: &egui::Context, appearance: Appearance) {
        let system = ctx.system_theme();
        let focused = ctx.input(|input| input.viewport().focused.unwrap_or(true));
        if self.observed_system != Some(system) || (focused && !self.focused) {
            self.high_contrast = native_high_contrast();
        }
        self.observed_system = Some(system);
        self.focused = focused;
        if self.last_appearance != Some(appearance) {
            ctx.set_theme(match appearance {
                Appearance::System => egui::ThemePreference::System,
                Appearance::Dark => egui::ThemePreference::Dark,
                Appearance::Light => egui::ThemePreference::Light,
            });
            self.last_appearance = Some(appearance);
        }
        let dark = match appearance {
            Appearance::System => system != Some(egui::Theme::Light),
            Appearance::Dark => true,
            Appearance::Light => false,
        };
        let chosen = self
            .high_contrast
            .unwrap_or(if dark { tessel::DARK } else { tessel::LIGHT });
        let size = ctx.content_rect().size();
        let density = if size.x >= 700.0 && size.y >= 620.0 {
            tessel::COMFORTABLE
        } else {
            tessel::COMPACT
        };
        if self.applied != Some((chosen, density))
            || ctx.style().visuals.panel_fill != rgb(chosen.background)
        {
            ctx.set_style(style(chosen, density, dark));
            ctx.data_mut(|data| data.insert_temp(palette_id(), chosen));
            self.applied = Some((chosen, density));
        }
    }
}

fn style(palette: Palette, density: Density, dark: bool) -> egui::Style {
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    let fg = rgb(palette.foreground);
    let border = egui::Stroke::new(1.0_f32, rgb(palette.border));
    let edge = egui::Stroke::new(1.0_f32, rgb(palette.edge));
    let focus = egui::Stroke::new(tessel::FOCUS_OUTLINE, rgb(palette.focus));
    visuals.panel_fill = rgb(palette.background);
    visuals.window_fill = rgb(palette.surface);
    visuals.window_stroke = edge;
    visuals.window_shadow = egui::epaint::Shadow::NONE;
    visuals.popup_shadow = egui::epaint::Shadow::NONE;
    visuals.extreme_bg_color = rgb(palette.raised);
    visuals.text_edit_bg_color = Some(rgb(palette.surface));
    visuals.faint_bg_color = rgb(palette.surface);
    visuals.override_text_color = Some(fg);
    visuals.weak_text_color = Some(rgb(palette.muted));
    visuals.hyperlink_color = rgb(palette.info);
    visuals.warn_fg_color = rgb(palette.warning);
    visuals.error_fg_color = rgb(palette.danger);
    visuals.selection.bg_fill = rgb(palette.selected);
    visuals.selection.stroke = egui::Stroke::new(2.0_f32, rgb(palette.selection_indicator));
    for widget in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
        &mut visuals.widgets.noninteractive,
    ] {
        widget.corner_radius = egui::CornerRadius::same(RADIUS);
        widget.fg_stroke.color = fg;
        widget.expansion = 0.0;
    }
    visuals.widgets.noninteractive.bg_stroke = edge;
    visuals.widgets.inactive.bg_stroke = border;
    visuals.widgets.hovered.bg_stroke = border;
    visuals.widgets.open.bg_stroke = border;
    visuals.widgets.active.bg_stroke = focus;
    visuals.widgets.inactive.bg_fill = rgb(palette.raised);
    visuals.widgets.inactive.weak_bg_fill = rgb(palette.raised);
    for widget in [
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.open,
        &mut visuals.widgets.active,
    ] {
        widget.bg_fill = rgb(palette.selected);
        widget.weak_bg_fill = rgb(palette.selected);
    }
    let mut style = egui::Style {
        visuals,
        animation_time: 0.0,
        ..Default::default()
    };
    style.spacing.item_spacing = egui::vec2(tessel::CONTROL_GAP, tessel::CONTROL_GAP_MIN);
    style.spacing.button_padding = egui::vec2(8.0, 4.0);
    style.spacing.interact_size = egui::vec2(32.0, density.control_min_height);
    style.spacing.window_margin = egui::Margin::same(density.panel_padding as i8);
    style.text_styles = [
        (
            egui::TextStyle::Heading,
            egui::FontId::new(
                tessel::FONT_SIZE_TITLE,
                egui::FontFamily::Name(SEMIBOLD.into()),
            ),
        ),
        (
            egui::TextStyle::Body,
            egui::FontId::proportional(tessel::FONT_SIZE_BODY),
        ),
        (
            egui::TextStyle::Button,
            egui::FontId::proportional(tessel::FONT_SIZE_LABEL),
        ),
        (
            egui::TextStyle::Small,
            egui::FontId::proportional(tessel::FONT_SIZE_CAPTION),
        ),
        (
            egui::TextStyle::Monospace,
            egui::FontId::monospace(tessel::FONT_SIZE_LABEL),
        ),
    ]
    .into();
    style
}

pub(super) fn install_fonts(ctx: &egui::Context) {
    let mut definitions = egui::FontDefinitions::default();
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        let fonts = std::path::PathBuf::from(root).join("Fonts");
        for (file, name, family) in [
            ("segoeui.ttf", "Segoe UI", egui::FontFamily::Proportional),
            (
                "CascadiaMono.ttf",
                "Cascadia Mono",
                egui::FontFamily::Monospace,
            ),
            ("consola.ttf", "Consolas", egui::FontFamily::Monospace),
            (
                "seguisb.ttf",
                SEMIBOLD,
                egui::FontFamily::Name(SEMIBOLD.into()),
            ),
        ] {
            if let Ok(bytes) = std::fs::read(fonts.join(file)) {
                definitions
                    .font_data
                    .insert(name.into(), egui::FontData::from_owned(bytes).into());
                let font_list = definitions.families.entry(family).or_default();
                let rank = usize::from(
                    name == "Consolas"
                        && font_list
                            .first()
                            .is_some_and(|font| font == "Cascadia Mono"),
                );
                font_list.insert(rank, name.into());
            }
        }
        if let Ok(bytes) = std::fs::read(fonts.join("seguisym.ttf")) {
            let name = "Segoe UI Symbol";
            definitions
                .font_data
                .insert(name.into(), egui::FontData::from_owned(bytes).into());
            let proportional = definitions
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default();
            let after_segoe =
                usize::from(proportional.first().is_some_and(|font| font == "Segoe UI"));
            proportional.insert(after_segoe, name.into());
            definitions
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .push(name.into());
        }
    }
    let fallbacks = definitions.families[&egui::FontFamily::Proportional].clone();
    definitions
        .families
        .entry(egui::FontFamily::Name(SEMIBOLD.into()))
        .or_default()
        .extend(fallbacks);
    ctx.set_fonts(definitions);
}

#[cfg(windows)]
fn native_high_contrast() -> Option<Palette> {
    #[repr(C)]
    struct HighContrast {
        size: u32,
        flags: u32,
        scheme: *mut u16,
    }
    #[link(name = "user32")]
    extern "system" {
        fn SystemParametersInfoW(
            action: u32,
            param: u32,
            value: *mut HighContrast,
            flags: u32,
        ) -> i32;
        fn GetSysColor(index: i32) -> u32;
    }
    const SPI_GETHIGHCONTRAST: u32 = 0x0042;
    const HCF_HIGHCONTRASTON: u32 = 1;
    const WINDOW: i32 = 5;
    const WINDOW_TEXT: i32 = 8;
    const HIGHLIGHT: i32 = 13;
    const HIGHLIGHT_TEXT: i32 = 14;
    const BUTTON_FACE: i32 = 15;
    const BUTTON_TEXT: i32 = 18;
    let mut value = HighContrast {
        size: std::mem::size_of::<HighContrast>() as u32,
        flags: 0,
        scheme: std::ptr::null_mut(),
    };
    // SPI_GETHIGHCONTRAST writes the fixed Win32 structure into this buffer.
    if unsafe { SystemParametersInfoW(SPI_GETHIGHCONTRAST, value.size, &mut value, 0) } == 0
        || value.flags & HCF_HIGHCONTRASTON == 0
    {
        return None;
    }
    let system_color = |index| {
        // All indexes are documented system colors and GetSysColor takes no pointers.
        let [red, green, blue, _] = unsafe { GetSysColor(index) }.to_le_bytes();
        [red, green, blue]
    };
    let background = system_color(WINDOW);
    let foreground = system_color(WINDOW_TEXT);
    let highlight = system_color(HIGHLIGHT);
    let selected_text = system_color(HIGHLIGHT_TEXT);
    let raised = system_color(BUTTON_FACE);
    let edge = system_color(BUTTON_TEXT);
    Some(Palette {
        background,
        surface: background,
        raised,
        selected: background,
        foreground,
        muted: foreground,
        border: edge,
        accent: highlight,
        accent_foreground: selected_text,
        selection_indicator: highlight,
        focus: foreground,
        action: highlight,
        action_hover: highlight,
        action_pressed: highlight,
        action_foreground: selected_text,
        progress: foreground,
        info: foreground,
        warning: foreground,
        success: foreground,
        danger: foreground,
        material_rim: edge,
        material_shade: background,
        edge,
        grid: background,
        surround: background,
    })
}

#[cfg(not(windows))]
fn native_high_contrast() -> Option<Palette> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn installed_windows_symbol_font_resolves_status_marks() {
        let Some(root) = std::env::var_os("SystemRoot") else {
            return;
        };
        if !std::path::PathBuf::from(root)
            .join("Fonts/seguisym.ttf")
            .is_file()
        {
            return;
        }
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            ctx.fonts_mut(|fonts| {
                assert!(fonts.has_glyphs(&egui::FontId::proportional(13.0), "✓⚠✗"));
            });
        });
    }

    fn contrast(a: [u8; 3], b: [u8; 3]) -> f64 {
        fn luminance(color: [u8; 3]) -> f64 {
            let channels = color.map(|channel| {
                let value = f64::from(channel) / 255.0;
                if value <= 0.04045 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
            });
            channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722
        }
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn github_palettes_keep_text_action_and_focus_readable() {
        assert_eq!(tessel::DESIGN_REVISION, 2);
        for (palette, dark) in [(tessel::DARK, true), (tessel::LIGHT, false)] {
            let visuals = style(palette, tessel::COMPACT, dark).visuals;
            assert_eq!(visuals.text_color(), rgb(palette.foreground));
            assert_eq!(visuals.weak_text_color(), rgb(palette.muted));
            assert_eq!(
                visuals.widgets.noninteractive.bg_stroke.color,
                rgb(palette.edge)
            );
            assert_eq!(
                visuals.widgets.inactive.bg_stroke.color,
                rgb(palette.border)
            );
            assert_eq!(visuals.widgets.active.bg_stroke.color, rgb(palette.focus));
            assert_eq!(
                visuals.selection.stroke.color,
                rgb(palette.selection_indicator)
            );
            for surface in [
                palette.background,
                palette.surface,
                palette.raised,
                palette.selected,
            ] {
                assert!(contrast(palette.foreground, surface) >= 4.5);
                assert!(contrast(palette.muted, surface) >= 4.5);
            }
            for fill in [palette.action, palette.action_hover, palette.action_pressed] {
                assert!(contrast(palette.action_foreground, fill) >= 4.5);
            }
            assert!(contrast(palette.focus, palette.surface) >= 3.0);
            assert!(contrast(palette.selection_indicator, palette.surface) >= 3.0);
        }
    }
}
