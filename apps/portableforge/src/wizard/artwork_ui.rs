//! Edits and previews for packaged artwork.

use std::path::PathBuf;
#[cfg(all(windows, target_arch = "x86_64"))]
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, TryRecvError};

use eframe::egui;
use portableforge_core::artwork::{
    render_icon, render_sidebar, Artwork, FitMode, IconArtwork, Raster, SidebarArtwork, Transform,
    SIDEBAR_HEIGHT, SIDEBAR_WIDTH,
};

use super::dense;

const DEFAULT_PANE_COLOR: [u8; 3] = [0x15, 0x26, 0x3c];
const ICON_SIZES: [u32; 6] = [16, 32, 48, 75, 128, 256];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    Icon,
    Sidebar,
}

#[derive(Default)]
pub(super) struct Editor {
    applied: Artwork,
    editing: Option<Draft>,
    return_focus: Option<egui::Id>,
}

struct Draft {
    opener: egui::Id,
    copy: &'static EditorCopy,
    tab: Tab,
    icon_size: u32,
    artwork: Artwork,
    base_icon: Option<Raster>,
    captured_load: Option<Receiver<Result<Raster, String>>>,
    preview: Option<egui::TextureHandle>,
    dirty_preview: bool,
    focus_first: bool,
    problem: Option<String>,
}

struct EditorCopy {
    modal_id: &'static str,
    title: &'static str,
    apply: &'static str,
    icon_tab: &'static str,
    sidebar_tab: &'static str,
    icon_help: &'static str,
    restore_icon: &'static str,
    normalize_base_preview: bool,
}

const CAPTURE_COPY: EditorCopy = EditorCopy {
    modal_id: "profile-artwork",
    title: "Package artwork",
    apply: "Apply artwork",
    icon_tab: "App icon",
    sidebar_tab: "Installer left pane",
    icon_help: "Import a picture or use the captured app icon.",
    restore_icon: "Use captured icon",
    normalize_base_preview: false,
};

const FOLDER_COPY: EditorCopy = EditorCopy {
    modal_id: "folder-artwork",
    title: "Package Artwork",
    apply: "Apply Artwork",
    icon_tab: "App Icon",
    sidebar_tab: "Installer Left Pane",
    icon_help: "Import a picture or keep the default app icon.",
    restore_icon: "Use Default App Icon",
    normalize_base_preview: true,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Exit {
    Apply,
    Cancel,
}

impl Editor {
    pub(super) fn is_open(&self) -> bool {
        self.editing.is_some()
    }

    pub(super) fn artwork(&self) -> Option<&Artwork> {
        (self.applied.icon.is_some() || self.applied.sidebar.is_some()).then_some(&self.applied)
    }

    pub(super) fn open(&mut self, opener: egui::Id, captured_icon: Option<Raster>) {
        self.open_with_copy(opener, captured_icon, &CAPTURE_COPY);
    }

    pub(super) fn open_folder(&mut self, opener: egui::Id, base_icon: Raster) {
        self.open_with_copy(opener, Some(base_icon), &FOLDER_COPY);
    }

    fn open_with_copy(
        &mut self,
        opener: egui::Id,
        base_icon: Option<Raster>,
        copy: &'static EditorCopy,
    ) {
        self.editing = Some(Draft {
            opener,
            copy,
            tab: Tab::Icon,
            icon_size: 128,
            artwork: self.applied.clone(),
            base_icon,
            captured_load: None,
            preview: None,
            dirty_preview: true,
            focus_first: true,
            problem: None,
        });
    }

    pub(super) fn load_captured_icon(
        &mut self,
        ctx: &egui::Context,
        session: PathBuf,
        main_exe: PathBuf,
    ) {
        let Some(draft) = &mut self.editing else {
            return;
        };
        #[cfg(all(windows, target_arch = "x86_64"))]
        {
            let (sender, receiver) = mpsc::channel();
            let ctx = ctx.clone();
            let worker = std::thread::Builder::new()
                .name("captured-icon-preview".into())
                .spawn(move || {
                    let result = portableforge_core::wizard_package::captured_icon_preview(
                        &session, &main_exe,
                    );
                    if sender.send(result).is_ok() {
                        ctx.request_repaint();
                    }
                });
            match worker {
                Ok(_) => draft.captured_load = Some(receiver),
                Err(error) => {
                    draft.problem = Some(format!("Could not start the icon preview: {error}"))
                }
            }
        }
        #[cfg(not(all(windows, target_arch = "x86_64")))]
        {
            let _ = (ctx, session, main_exe);
            draft.problem = Some("The captured icon preview needs 64-bit Windows.".into());
        }
    }

    /// Returns true after the user applies the artwork.
    pub(super) fn show(&mut self, ctx: &egui::Context) -> bool {
        let Some(draft) = &mut self.editing else {
            if let Some(opener) = self.return_focus {
                if ctx.memory(|memory| memory.top_modal_layer().is_none()) {
                    ctx.memory_mut(|memory| memory.request_focus(opener));
                    self.return_focus = None;
                }
                ctx.request_repaint();
            }
            return false;
        };
        draft.poll_captured_icon();
        let width = (ctx.content_rect().width() - 52.0).clamp(380.0, 560.0);
        let response = egui::Modal::new(egui::Id::new(draft.copy.modal_id)).show(ctx, |ui| {
            ui.set_width(width);
            ui.heading(draft.copy.title);
            draft.show_tabs(ui);
            draft.show_workspace(ui);
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    return Some(Exit::Cancel);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    dense::primary_action(ui, draft.can_apply(), draft.copy.apply)
                        .clicked()
                        .then_some(Exit::Apply)
                })
                .inner
            })
            .inner
        });
        if let Some(exit) = response
            .inner
            .or_else(|| response.should_close().then_some(Exit::Cancel))
        {
            self.finish(exit);
            ctx.request_repaint();
            return exit == Exit::Apply;
        }
        false
    }

    fn finish(&mut self, exit: Exit) {
        if let Some(draft) = self.editing.take() {
            self.return_focus = Some(draft.opener);
            if exit == Exit::Apply {
                self.applied = draft.artwork;
            }
        }
    }
}

impl Draft {
    fn poll_captured_icon(&mut self) {
        let Some(receiver) = &self.captured_load else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("The captured icon preview stopped.".into()),
        };
        self.captured_load = None;
        match result {
            Ok(icon) => {
                self.base_icon = Some(icon);
                self.dirty_preview = true;
            }
            Err(error) => self.problem = Some(error),
        }
    }

    fn can_apply(&self) -> bool {
        self.artwork
            .sidebar
            .as_ref()
            .is_none_or(|pane| pane.source.is_some())
            || self.artwork.icon.is_some()
            || self.base_icon.is_some()
    }

    fn show_tabs(&mut self, ui: &mut egui::Ui) {
        let before = self.tab;
        ui.horizontal(|ui| {
            let icon = ui.selectable_value(&mut self.tab, Tab::Icon, self.copy.icon_tab);
            ui.selectable_value(&mut self.tab, Tab::Sidebar, self.copy.sidebar_tab);
            if self.focus_first {
                icon.request_focus();
                self.focus_first = false;
            }
        });
        if before != self.tab {
            self.dirty_preview = true;
            self.problem = None;
        }
    }

    fn show_workspace(&mut self, ui: &mut egui::Ui) {
        let preview_width = 146.0;
        let controls_width =
            (ui.available_width() - preview_width - ui.spacing().item_spacing.x).max(180.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(controls_width, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(controls_width);
                    match self.tab {
                        Tab::Icon => self.show_icon_controls(ui),
                        Tab::Sidebar => self.show_sidebar_controls(ui),
                    }
                    if let Some(problem) = &self.problem {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(problem).color(ui.visuals().error_fg_color),
                            )
                            .truncate(),
                        )
                        .on_hover_text(problem);
                        dense::view_text(
                            ui,
                            "View artwork error",
                            "Artwork problem",
                            problem,
                            true,
                        );
                    }
                },
            );
            self.update_preview(ui.ctx());
            ui.allocate_ui_with_layout(
                egui::vec2(preview_width, 0.0),
                egui::Layout::top_down(egui::Align::Center),
                |ui| self.show_preview(ui, preview_width),
            );
        });
    }

    fn show_icon_controls(&mut self, ui: &mut egui::Ui) {
        ui.label(self.copy.icon_help);
        ui.horizontal_wrapped(|ui| {
            if ui.button("Import image…").clicked() {
                self.import_image();
            }
            if ui
                .add_enabled(
                    self.artwork.icon.is_some(),
                    egui::Button::new(self.copy.restore_icon),
                )
                .clicked()
            {
                self.artwork.icon = None;
                self.changed();
            }
        });
        if let Some(icon) = &mut self.artwork.icon {
            if show_transform(ui, &mut icon.transform) {
                self.changed();
            }
        }
        ui.label(egui::RichText::new("Preview icon size").strong());
        ui.horizontal_wrapped(|ui| {
            for size in ICON_SIZES {
                if ui
                    .selectable_value(&mut self.icon_size, size, format!("{size} px"))
                    .changed()
                {
                    self.dirty_preview = true;
                }
            }
        });
    }

    fn show_sidebar_controls(&mut self, ui: &mut egui::Ui) {
        ui.label("Use a picture or create a panel from the app icon.");
        ui.horizontal_wrapped(|ui| {
            if ui.button("Import image…").clicked() {
                self.import_image();
            }
            let icon_available = self.artwork.icon.is_some() || self.base_icon.is_some();
            if ui
                .add_enabled(icon_available, egui::Button::new("Use app icon"))
                .clicked()
            {
                let background = self
                    .artwork
                    .sidebar
                    .as_ref()
                    .map_or(DEFAULT_PANE_COLOR, |pane| pane.background);
                self.artwork.sidebar = Some(SidebarArtwork {
                    source: None,
                    transform: Transform::default(),
                    background,
                });
                self.changed();
            }
        });
        if ui
            .add_enabled(
                self.artwork.sidebar.is_some(),
                egui::Button::new("Use default left pane"),
            )
            .clicked()
        {
            self.artwork.sidebar = None;
            self.changed();
        }
        if let Some(pane) = &mut self.artwork.sidebar {
            let transformed = show_transform(ui, &mut pane.transform);
            let recolored = ui
                .horizontal(|ui| {
                    let label = ui.label("Background");
                    let picker = ui
                        .color_edit_button_srgb(&mut pane.background)
                        .labelled_by(label.id);
                    let [r, g, b] = pane.background;
                    ui.monospace(format!("#{r:02X}{g:02X}{b:02X}"));
                    picker.changed()
                })
                .inner;
            if transformed || recolored {
                self.changed();
            }
        }
    }

    fn import_image(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Choose artwork")
            .add_filter("PNG or ICO image", &["png", "ico"])
            .pick_file()
        else {
            return;
        };
        match Raster::load(&path) {
            Ok(source) => {
                match self.tab {
                    Tab::Icon => {
                        self.artwork.icon = Some(IconArtwork {
                            source,
                            transform: Transform::default(),
                        })
                    }
                    Tab::Sidebar => {
                        let background = self
                            .artwork
                            .sidebar
                            .as_ref()
                            .map_or(DEFAULT_PANE_COLOR, |pane| pane.background);
                        self.artwork.sidebar = Some(SidebarArtwork {
                            source: Some(source),
                            transform: Transform::default(),
                            background,
                        });
                    }
                }
                self.changed();
            }
            Err(error) => self.problem = Some(error),
        }
    }

    fn changed(&mut self) {
        self.dirty_preview = true;
        self.problem = None;
    }

    fn update_preview(&mut self, ctx: &egui::Context) {
        if !self.dirty_preview {
            return;
        }
        self.dirty_preview = false;
        let rendered = match self.tab {
            Tab::Icon => self.icon_preview(),
            Tab::Sidebar => self.artwork.sidebar.as_ref().map_or(Ok(None), |pane| {
                let icon = if pane.source.is_none() {
                    self.effective_icon(256)?
                } else {
                    None
                };
                render_sidebar(pane, icon.as_ref()).map(Some)
            }),
        };
        self.preview = match rendered {
            Ok(Some(raster)) => Some(ctx.load_texture(
                self.copy.modal_id,
                egui::ColorImage::from_rgba_unmultiplied(
                    [raster.width() as usize, raster.height() as usize],
                    raster.rgba(),
                ),
                egui::TextureOptions::LINEAR,
            )),
            Ok(None) => None,
            Err(error) => {
                self.problem = Some(error);
                None
            }
        };
    }

    fn effective_icon(&self, size: u32) -> Result<Option<Raster>, String> {
        self.artwork.icon.as_ref().map_or_else(
            || Ok(self.base_icon.clone()),
            |icon| render_icon(icon, size).map(Some),
        )
    }

    fn icon_preview(&self) -> Result<Option<Raster>, String> {
        if self.copy.normalize_base_preview && self.artwork.icon.is_none() {
            if let Some(source) = &self.base_icon {
                return render_icon(
                    &IconArtwork {
                        source: source.clone(),
                        transform: Transform::default(),
                    },
                    self.icon_size,
                )
                .map(Some);
            }
        }
        self.effective_icon(self.icon_size)
    }

    fn show_preview(&self, ui: &mut egui::Ui, width: f32) {
        ui.label(egui::RichText::new("Preview").strong());
        let size = match self.tab {
            Tab::Icon => egui::vec2(128.0, 128.0),
            Tab::Sidebar => egui::vec2(184.0 * SIDEBAR_WIDTH as f32 / SIDEBAR_HEIGHT as f32, 184.0),
        };
        let (panel, response) =
            ui.allocate_exact_size(egui::vec2(width, size.y), egui::Sense::hover());
        let image = egui::Rect::from_center_size(panel.center(), size);
        let painter = ui.painter_at(panel);
        painter.rect_filled(image, 0.0, ui.visuals().extreme_bg_color);
        painter.rect_stroke(
            image,
            0.0,
            ui.visuals().widgets.noninteractive.bg_stroke,
            egui::StrokeKind::Inside,
        );
        if let Some(preview) = &self.preview {
            let label = match self.tab {
                Tab::Icon => format!("App icon preview at {} pixels", self.icon_size),
                Tab::Sidebar => "Installer left pane preview".into(),
            };
            response
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Image, true, &label));
            let size = preview.size_vec2();
            let mut ratio = (image.width() / size.x).min(image.height() / size.y);
            if self.tab == Tab::Icon {
                ratio = ratio.min(self.icon_size as f32 / size.x.max(size.y));
            }
            let bounds = egui::Rect::from_center_size(image.center(), size * ratio);
            painter.image(
                preview.id(),
                bounds,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        } else {
            let label = match self.tab {
                Tab::Icon if self.captured_load.is_some() => "Loading icon…",
                Tab::Icon => "No preview",
                Tab::Sidebar => "Default pane",
            };
            response
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label));
            painter.text(
                image.center(),
                egui::Align2::CENTER_CENTER,
                label,
                egui::TextStyle::Small.resolve(ui.style()),
                ui.visuals().weak_text_color(),
            );
        }
        if self.tab == Tab::Sidebar {
            ui.label(egui::RichText::new("328 × 628 px").small().weak());
        }
    }
}

fn show_transform(ui: &mut egui::Ui, transform: &mut Transform) -> bool {
    let before = *transform;
    ui.horizontal_wrapped(|ui| {
        if ui.button("Rotate 90°").clicked() {
            transform.quarter_turns = transform.quarter_turns.wrapping_add(1) % 4;
        }
        if ui
            .selectable_label(transform.flip_horizontal, "Flip horizontal")
            .clicked()
        {
            transform.flip_horizontal = !transform.flip_horizontal;
        }
        if ui
            .selectable_label(transform.flip_vertical, "Flip vertical")
            .clicked()
        {
            transform.flip_vertical = !transform.flip_vertical;
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.label("Placement");
        ui.selectable_value(&mut transform.fit, FitMode::Fit, "Fit");
        ui.selectable_value(&mut transform.fit, FitMode::Crop, "Crop");
        if ui
            .add_enabled(
                *transform != Transform::default(),
                egui::Button::new("Reset adjustments"),
            )
            .clicked()
        {
            *transform = Transform::default();
        }
    });
    before != *transform
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_icon() -> Raster {
        let screenshot = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/screenshots/wizard-01-pick.png");
        Raster::load(&screenshot).expect("load an existing image for preview")
    }

    #[test]
    fn artwork_only_changes_the_applied_draft_after_apply() {
        let opener = egui::Id::new("edit-artwork");
        let icon = fixture_icon();
        for copy in [&CAPTURE_COPY, &FOLDER_COPY] {
            let mut editor = Editor::default();
            editor.open_with_copy(opener, Some(icon.clone()), copy);
            editor.editing.as_mut().unwrap().artwork.icon = Some(IconArtwork {
                source: icon.clone(),
                transform: Transform::default(),
            });
            assert!(editor.artwork().is_none());
            editor.finish(Exit::Apply);
            assert!(editor.artwork().unwrap().icon.is_some());

            editor.open_with_copy(opener, Some(icon.clone()), copy);
            editor.editing.as_mut().unwrap().artwork.icon = None;
            editor.finish(Exit::Cancel);
            assert!(editor.artwork().unwrap().icon.is_some());
            assert_eq!(editor.return_focus, Some(opener));
        }
    }

    #[test]
    fn folder_normalizes_the_icon_preview_but_keeps_the_raw_sidebar_source() {
        let mut editor = Editor::default();
        let base = fixture_icon();
        editor.open_folder(egui::Id::new("folder-artwork"), base.clone());
        let draft = editor.editing.as_mut().unwrap();
        draft.icon_size = 32;
        let preview = draft.icon_preview().unwrap().unwrap();
        assert_eq!((preview.width(), preview.height()), (32, 32));
        assert_eq!(
            draft.effective_icon(256).unwrap().unwrap().rgba(),
            base.rgba()
        );
    }

    #[test]
    fn edited_artwork_tabs_fit_at_each_text_size() {
        let bounds =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::from(dense::MIN_CONTENT));
        let icon = fixture_icon();
        for scale in [1.0, 1.25, 1.5] {
            for (tab, copy) in [
                (Tab::Icon, &CAPTURE_COPY),
                (Tab::Sidebar, &CAPTURE_COPY),
                (Tab::Icon, &FOLDER_COPY),
                (Tab::Sidebar, &FOLDER_COPY),
            ] {
                let ctx = egui::Context::default();
                super::super::theme::install_fonts(&ctx);
                ctx.set_zoom_factor(scale);
                let mut theme = super::super::theme::State::default();
                let mut editor = Editor::default();
                editor.open_with_copy(egui::Id::new("edit-artwork"), Some(icon.clone()), copy);
                let draft = editor.editing.as_mut().unwrap();
                draft.tab = tab;
                draft.artwork = Artwork {
                    icon: Some(IconArtwork {
                        source: icon.clone(),
                        transform: Transform::default(),
                    }),
                    sidebar: Some(SidebarArtwork {
                        source: None,
                        transform: Transform::default(),
                        background: DEFAULT_PANE_COLOR,
                    }),
                };
                draft.problem = Some("This image could not be read.".into());
                let mut draw = || {
                    ctx.run(
                        egui::RawInput {
                            screen_rect: Some(bounds),
                            ..Default::default()
                        },
                        |ctx| {
                            theme.apply(ctx, portableforge_core::settings::Appearance::Dark);
                            editor.show(ctx);
                        },
                    )
                };
                draw();
                draw();
                let output = draw();
                for expected in [
                    copy.apply,
                    "Cancel",
                    "Rotate 90°",
                    "Flip horizontal",
                    "Flip vertical",
                    "Reset adjustments",
                    "View artwork error",
                    if tab == Tab::Icon {
                        "256 px"
                    } else {
                        "Background"
                    },
                ] {
                    let shape = output.shapes.iter().find(|clipped| {
                        matches!(&clipped.shape, egui::Shape::Text(text) if text.galley.text() == expected)
                    }).unwrap_or_else(|| panic!("Missing {expected:?} at {scale}"));
                    let visible = bounds.intersect(shape.clip_rect).expand(1.0);
                    assert!(
                        visible.contains_rect(shape.shape.visual_bounding_rect()),
                        "{expected:?} clipped at {scale}: {:?} versus {visible:?}",
                        shape.shape.visual_bounding_rect(),
                    );
                }
            }
        }
    }
}
