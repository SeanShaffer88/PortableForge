//! Shared heading and source row for package review.

use std::path::Path;

use eframe::egui;

use super::dense;

pub(super) const TITLE: &str = "Review Your Package";

pub(super) fn overview(ui: &mut egui::Ui, source_type: &str, display_name: &str, path: &Path) {
    dense::heading(ui, TITLE, "");
    let path = path.to_string_lossy();
    let label = if display_name.is_empty() {
        source_type.to_owned()
    } else {
        format!("{source_type} · {display_name}")
    };
    ui.horizontal(|ui| {
        let show_path = !path.is_empty() && path != display_name;
        let width = if show_path {
            dense::width_with_button(ui, "View Source")
        } else {
            ui.available_width()
        };
        ui.allocate_ui_with_layout(
            egui::vec2(width.max(0.0), ui.spacing().interact_size.y),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add(egui::Label::new(egui::RichText::new(&label).weak()).truncate())
                    .on_hover_text(path.as_ref());
            },
        );
        dense::view_text(ui, "View Source", "Package Source", &path, show_path);
    });
}
