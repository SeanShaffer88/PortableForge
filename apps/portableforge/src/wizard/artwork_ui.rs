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

use super::{dense, file_picker};

const DEFAULT_PANE_COLOR: [u8; 3] = [0x15, 0x26, 0x3c];
const ICON_SIZES: [u32; 6] = [16, 32, 48, 75, 128, 256];
const ARTWORK_HELP: &str = "Point to a control or focus it with Tab to read what it does.";

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    Icon,
    Sidebar,
}

impl Tab {
    fn index(self) -> usize {
        match self {
            Self::Icon => 0,
            Self::Sidebar => 1,
        }
    }
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
    previews: [Option<egui::TextureHandle>; 2],
    dirty_previews: [bool; 2],
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

    pub(super) fn set_applied(&mut self, artwork: Artwork) {
        self.applied = artwork;
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
            previews: [None, None],
            dirty_previews: [true; 2],
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
        let viewport = ctx.content_rect();
        let paired = viewport.width() >= 760.0 && viewport.height() >= 560.0;
        let width = (viewport.width() - 52.0).clamp(380.0, if paired { 860.0 } else { 560.0 });
        let response = egui::Modal::new(egui::Id::new(draft.copy.modal_id)).show(ctx, |ui| {
            ui.set_width(width);
            let heading = ui.add(
                egui::Label::new(egui::RichText::new(draft.copy.title).heading())
                    .sense(egui::Sense::focusable_noninteractive()),
            );
            if draft.focus_first {
                heading.request_focus();
                draft.focus_first = false;
            }
            if paired {
                draft.show_paired_workspace(ui);
            } else {
                draft.show_tabs(ui);
                draft.show_workspace(ui);
            }
            ui.separator();
            let exit = ui.horizontal(|ui| {
                if help(ui.button("Cancel"), "Close the artwork editor and discard edits made since you opened it. Your previously applied artwork stays in the package.").clicked() {
                    return Some(Exit::Cancel);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    help(dense::primary_action(ui, draft.can_apply(), draft.copy.apply),
                        "Use these previews in the package. You can revisit and adjust the artwork before building.")
                        .clicked()
                        .then_some(Exit::Apply)
                })
                .inner
            }).inner;
            ui.separator();
            ui.scope(|ui| dense::help_strip(ui, ARTWORK_HELP));
            exit
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
                self.dirty_previews = [true; 2];
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
            help(
                ui.selectable_value(&mut self.tab, Tab::Icon, self.copy.icon_tab),
                "Edit the icon shown for the portable app. Check its preview at different sizes before applying.",
            );
            help(
                ui.selectable_value(&mut self.tab, Tab::Sidebar, self.copy.sidebar_tab),
                "Edit the vertical picture shown to the left of the installation steps. You can import a picture or use the app icon as its starting point.",
            );
        });
        if before != self.tab {
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
                        Tab::Icon => self.show_icon_controls(ui, false),
                        Tab::Sidebar => self.show_sidebar_controls(ui, false),
                    }
                    self.show_problem(ui);
                },
            );
            self.update_preview(ui.ctx(), self.tab);
            ui.allocate_ui_with_layout(
                egui::vec2(preview_width, 0.0),
                egui::Layout::top_down(egui::Align::Center),
                |ui| self.show_preview(ui, preview_width, self.tab, false),
            );
        });
    }

    fn show_paired_workspace(&mut self, ui: &mut egui::Ui) {
        ui.columns(2, |columns| {
            for (column, tab) in columns.iter_mut().zip([Tab::Icon, Tab::Sidebar]) {
                let heading = match tab {
                    Tab::Icon => self.copy.icon_tab,
                    Tab::Sidebar => self.copy.sidebar_tab,
                };
                column.label(egui::RichText::new(heading).strong());
                self.update_preview(column.ctx(), tab);
                self.show_preview(column, column.available_width(), tab, true);
                column.add_space(8.0);
                match tab {
                    Tab::Icon => self.show_icon_controls(column, true),
                    Tab::Sidebar => self.show_sidebar_controls(column, true),
                }
            }
        });
        self.show_problem(ui);
        if self.dirty_previews.iter().any(|dirty| *dirty) {
            ui.ctx().request_repaint();
        }
    }

    fn show_problem(&self, ui: &mut egui::Ui) {
        if let Some(problem) = &self.problem {
            ui.add(
                egui::Label::new(egui::RichText::new(problem).color(ui.visuals().error_fg_color))
                    .truncate(),
            );
            dense::view_text(ui, "View artwork error", "Artwork problem", problem, true);
        }
    }

    fn show_icon_controls(&mut self, ui: &mut egui::Ui, paired: bool) {
        if !paired {
            ui.label(self.copy.icon_help);
        }
        let import_label = if paired {
            "Import Icon…"
        } else {
            "Import image…"
        };
        ui.horizontal_wrapped(|ui| {
            let import = help(ui.button(import_label), "Choose a PNG or ICO image for the app icon. A square image with a transparent background usually reads best at small sizes.");
            self.import_image(&import, Tab::Icon);
            if help(
                ui.add_enabled(self.artwork.icon.is_some(), egui::Button::new(self.copy.restore_icon)),
                "Remove the custom icon and use the icon PortableForge found for this app. You can import another picture at any time.",
            ).clicked()
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
                if help(
                    ui.selectable_value(&mut self.icon_size, size, format!("{size} px")),
                    "Change the preview size to check the artwork. The package still includes all required icon sizes.",
                ).changed()
                {
                    self.dirty_previews[Tab::Icon.index()] = true;
                }
            }
        });
    }

    fn show_sidebar_controls(&mut self, ui: &mut egui::Ui, paired: bool) {
        if !paired {
            ui.label("Use a picture or create a panel from the app icon.");
        }
        let import_label = if paired {
            "Import Left Pane…"
        } else {
            "Import image…"
        };
        ui.horizontal_wrapped(|ui| {
            let import = help(ui.button(import_label), "Choose a PNG or ICO picture for the installer’s left pane. PortableForge prepares it at 328 by 628 pixels. Use Fit to show the whole image or Crop to fill the pane.");
            self.import_image(&import, Tab::Sidebar);
            let icon_available = self.artwork.icon.is_some() || self.base_icon.is_some();
            if help(ui.add_enabled(icon_available, egui::Button::new("Use app icon")),
                "Use the current app icon as the picture for the installer’s left pane. Set the background color below.").clicked()
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
        if help(
            ui.add_enabled(self.artwork.sidebar.is_some(), egui::Button::new("Use default left pane")),
            "Remove your custom artwork for the left pane and use the installer’s built-in artwork.",
        ).clicked()
        {
            self.artwork.sidebar = None;
            self.changed();
        }
        if let Some(pane) = &mut self.artwork.sidebar {
            let transformed = show_transform(ui, &mut pane.transform);
            let recolored = ui
                .horizontal(|ui| {
                    let label = ui.label("Background");
                    let picker = help(
                        ui.color_edit_button_srgb(&mut pane.background).labelled_by(label.id),
                        "Choose the color behind transparent areas and around pictures that do not fill the pane.",
                    );
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

    fn import_image(&mut self, opener: &egui::Response, tab: Tab) {
        let Some(path) = file_picker::choose(
            opener,
            "Choose artwork",
            file_picker::Mode::File(&["png", "ico"]),
            None,
        )
        .and_then(|paths| paths.into_iter().next()) else {
            return;
        };
        match Raster::load(&path) {
            Ok(source) => {
                match tab {
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
        self.dirty_previews = [true; 2];
        self.problem = None;
    }

    fn update_preview(&mut self, ctx: &egui::Context, tab: Tab) {
        let index = tab.index();
        if !self.dirty_previews[index] {
            return;
        }
        self.dirty_previews[index] = false;
        let rendered = match tab {
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
        self.previews[index] = match rendered {
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

    fn show_preview(&self, ui: &mut egui::Ui, width: f32, tab: Tab, paired: bool) {
        if !paired {
            ui.label(egui::RichText::new("Preview").strong());
        }
        let pane_height = match (paired, ui.ctx().content_rect().height()) {
            (true, height) if height < 620.0 => 112.0,
            (true, _) => 144.0,
            (false, _) => 184.0,
        };
        let size = match tab {
            Tab::Icon => egui::Vec2::splat(128.0_f32.min(pane_height)),
            Tab::Sidebar => egui::vec2(
                pane_height * SIDEBAR_WIDTH as f32 / SIDEBAR_HEIGHT as f32,
                pane_height,
            ),
        };
        let (panel, response) = ui.allocate_exact_size(
            egui::vec2(width, if paired { pane_height } else { size.y }),
            egui::Sense::hover(),
        );
        let image = egui::Rect::from_center_size(panel.center(), size);
        let painter = ui.painter_at(panel);
        painter.rect_filled(image, 0.0, ui.visuals().extreme_bg_color);
        painter.rect_stroke(
            image,
            0.0,
            ui.visuals().widgets.noninteractive.bg_stroke,
            egui::StrokeKind::Inside,
        );
        if let Some(preview) = &self.previews[tab.index()] {
            let label = match tab {
                Tab::Icon => format!("App icon preview at {} pixels", self.icon_size),
                Tab::Sidebar => "Installer left pane preview".into(),
            };
            response
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Image, true, &label));
            let size = preview.size_vec2();
            let mut ratio = (image.width() / size.x).min(image.height() / size.y);
            if tab == Tab::Icon {
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
            let label = match tab {
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
        if paired || tab == Tab::Sidebar {
            let dimensions = match tab {
                Tab::Icon => format!("{} × {} px", self.icon_size, self.icon_size),
                Tab::Sidebar => "328 × 628 px".into(),
            };
            ui.label(egui::RichText::new(dimensions).small().weak());
        }
    }
}

fn help(response: egui::Response, explanation: &str) -> egui::Response {
    dense::explained(response, explanation)
}

fn show_transform(ui: &mut egui::Ui, transform: &mut Transform) -> bool {
    let before = *transform;
    ui.horizontal_wrapped(|ui| {
        if help(
            ui.button("Rotate 90°"),
            "Turn the image one quarter clockwise. Select this again to keep rotating.",
        )
        .clicked()
        {
            transform.quarter_turns = transform.quarter_turns.wrapping_add(1) % 4;
        }
        if help(
            ui.selectable_label(transform.flip_horizontal, "Flip horizontal"),
            "Mirror the picture from left to right. Select again to undo the mirror.",
        )
        .clicked()
        {
            transform.flip_horizontal = !transform.flip_horizontal;
        }
        if help(
            ui.selectable_label(transform.flip_vertical, "Flip vertical"),
            "Mirror the picture from top to bottom. Select again to undo the mirror.",
        )
        .clicked()
        {
            transform.flip_vertical = !transform.flip_vertical;
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.label("Placement");
        help(ui.selectable_value(&mut transform.fit, FitMode::Fit, "Fit"),
            "Show the entire image. The empty area around it stays transparent for the icon or uses the background color for the pane.");
        help(ui.selectable_value(&mut transform.fit, FitMode::Crop, "Crop"),
            "Fill the available space by trimming the image edges. Check the preview to make sure the subject stays visible.");
        if help(
            ui.add_enabled(*transform != Transform::default(), egui::Button::new("Reset adjustments")),
            "Undo rotation, mirroring and cropping and keep the imported picture.",
        ).clicked()
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
        assert_artwork_fits(dense::MIN_CONTENT, false);
    }

    #[test]
    fn paired_artwork_previews_fit_at_each_text_size() {
        assert_artwork_fits([760.0, 560.0], true);
        assert_artwork_fits([980.0, 620.0], true);
    }

    fn assert_artwork_fits(size: [f32; 2], paired: bool) {
        let bounds = egui::Rect::from_min_size(egui::Pos2::ZERO, size.into());
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
                let mut expected = vec![
                    copy.apply,
                    "Cancel",
                    ARTWORK_HELP,
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
                ];
                if paired {
                    expected.extend(["Import Icon…", "Import Left Pane…", "Background", "256 px"]);
                    let previews = &editor.editing.as_ref().unwrap().previews;
                    let preview_bounds = previews.each_ref().map(|texture| {
                        let id = texture.as_ref().unwrap().id();
                        let shape = output.shapes.iter().find(|shape| {
                            matches!(&shape.shape, egui::Shape::Mesh(mesh) if mesh.texture_id == id)
                        }).expect("preview painted");
                        let rect = shape.shape.visual_bounding_rect();
                        assert!(bounds
                            .intersect(shape.clip_rect)
                            .expand(1.0)
                            .contains_rect(rect));
                        rect
                    });
                    assert!(preview_bounds[0].right() < preview_bounds[1].left());
                }
                for expected in expected {
                    let shapes: Vec<_> = output.shapes.iter().filter(|clipped| {
                        matches!(&clipped.shape, egui::Shape::Text(text) if text.galley.text() == expected)
                    }).collect();
                    assert!(
                        !shapes.is_empty(),
                        "Missing {expected:?} at {scale}. Text: {:?}",
                        output
                            .shapes
                            .iter()
                            .filter_map(|s| match &s.shape {
                                egui::Shape::Text(text) => Some((text.galley.text(), text.pos)),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                    );
                    for shape in shapes {
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
}
