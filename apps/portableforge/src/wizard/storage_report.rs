//! Shared presentation for file clues and captured data locations.

use eframe::egui;
use portableforge_core::storage::{StorageReport, StorageSource};

use super::dense;

pub(super) const TITLE: &str = "Find Where the App Saves Data";

pub(super) fn show(ui: &mut egui::Ui, report: &StorageReport, page: &mut usize, observed: bool) {
    let (intro, empty, scope) = if observed {
        (
            "Observed during setup or first use.",
            "The capture recorded no data locations.",
            "Shows changes in supported Windows folders and registry keys.",
        )
    } else {
        (
            "Clues from the selected app files.",
            "No storage clues found in the files checked.",
            "Test the app to confirm where it writes.",
        )
    };
    ui.label(intro);
    if report.findings.is_empty() {
        ui.label(empty);
    }
    let per_page = if ui.available_width() < 320.0 { 1 } else { 2 };
    for index in dense::page(ui, page, report.findings.len(), per_page, "Location") {
        let finding = &report.findings[index];
        let source = match finding.source {
            StorageSource::Referenced => "Referenced in",
            StorageSource::Included => "Included file",
            StorageSource::Setup => "Seen during setup",
            StorageSource::FirstRun => "Seen on first use",
        };
        let evidence = if finding.evidence.is_empty() {
            source.to_owned()
        } else {
            format!("{source} · {}", finding.evidence)
        };
        ui.push_id(index, |ui| {
            let width = ui.available_width();
            let detailed = dense::path_needs_view(ui, &finding.location, width)
                || dense::path_needs_view(ui, &evidence, width);
            ui.add(egui::Label::new(egui::RichText::new(&finding.location).strong()).truncate());
            ui.add(egui::Label::new(egui::RichText::new(&evidence).weak()).truncate());
            if detailed {
                let details = format!("{}\n{}", finding.location, evidence);
                dense::view_text(ui, "View Location Details", "Data Location", &details, true);
            }
        });
    }
    ui.weak(scope);
    if report.limited {
        ui.weak("Some locations or files could not be included.");
    }
}
