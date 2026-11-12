use std::path::{Path, PathBuf};

use eframe::egui;
use portableforge_core::folder::IssueSeverity;
use portableforge_core::guide::folder_flow as copy;
use portableforge_core::paf::CATEGORIES;

use super::super::{dense, open_folder, shell, theme, WizardApp};
use super::draft::{suggested_id, Language};
use super::{Failure, Flow, Page};

impl WizardApp {
    pub(in crate::wizard) fn show_folder_flow(&mut self, ctx: &egui::Context) {
        let previous = (
            self.folder_flow.page,
            self.folder_flow.busy(),
            self.folder_flow.build_ready(),
        );
        self.show_header(ctx, copy::HEADER, None);
        let visuals = &ctx.style().visuals;
        let frame = egui::Frame::new()
            .fill(visuals.window_fill)
            .stroke(visuals.widgets.noninteractive.bg_stroke)
            .inner_margin(egui::Margin::symmetric(12, 4));
        egui::TopBottomPanel::bottom("folder-actions")
            .frame(frame)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let flow = &mut self.folder_flow;
                    let back = ui.add_enabled(
                        !flow.busy() && !flow.artwork_open(),
                        egui::Button::new(if flow.page == Page::Done {
                            "Choose Another App"
                        } else {
                            "Back"
                        }),
                    );
                    if back.clicked() {
                        flow.back();
                    }
                    if !flow.busy() && !flow.artwork_open() {
                        shell::register_region(ui, shell::Region::Footer, back.id);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if flow.page == Page::Done {
                            let open = dense::primary_action(ui, true, "Open Output Folder");
                            shell::register_region(ui, shell::Region::Footer, open.id);
                            if open.clicked() {
                                if let Some(folder) = flow
                                    .result
                                    .as_ref()
                                    .and_then(|delivery| delivery.artifact.parent())
                                {
                                    if let Err(error) = open_folder(folder) {
                                        flow.error = Some(Failure::new(
                                            "Could not open the output folder.",
                                            error,
                                        ));
                                    }
                                }
                            }
                        } else if flow.can_stop() {
                            let stop = ui.button(copy::STOP);
                            shell::register_region(ui, shell::Region::Footer, stop.id);
                            if stop.clicked() {
                                flow.stop();
                            }
                        } else if flow.show_build_action() {
                            let build = dense::primary_action(ui, flow.build_ready(), copy::BUILD);
                            if flow.build_ready() {
                                shell::register_region(ui, shell::Region::Footer, build.id);
                            }
                            if build.clicked() {
                                flow.build(ctx);
                            }
                        }
                    });
                });
            });
        if !self.folder_flow.open {
            ctx.request_repaint();
            return;
        }
        egui::CentralPanel::default()
            .frame(dense::content_frame(ctx))
            .show(ctx, |ui| {
                let choose = ui
                    .add_enabled_ui(!self.folder_flow.artwork_open(), |ui| {
                        self.folder_flow.show_body(ui)
                    })
                    .inner;
                if choose {
                    self.choose_app_folder(ctx);
                }
            });
        if previous
            != (
                self.folder_flow.page,
                self.folder_flow.busy(),
                self.folder_flow.build_ready(),
            )
        {
            ctx.request_repaint();
        }
    }
}

impl Flow {
    /// The return value asks the wizard to open the shared input folder picker.
    fn show_body(&mut self, ui: &mut egui::Ui) -> bool {
        if self.busy() {
            let title = if self.scan.is_none() {
                "Checking Your Folder"
            } else if self.page == Page::Program {
                "Reading App Details"
            } else {
                copy::BUILDING
            };
            dense::heading(ui, title, "");
            if let Some(status) = self.status {
                status_label(ui, status);
            }
            path_summary(ui, "Folder", &self.source);
            return false;
        }

        if self.page == Page::Findings {
            self.show_findings(ui);
            return false;
        }

        let choose = match self.page {
            Page::Review => self.show_review(ui),
            Page::Program => {
                self.show_programs(ui);
                false
            }
            Page::Files => {
                self.show_files(ui);
                false
            }
            Page::Done => {
                self.show_delivery(ui);
                false
            }
            _ => {
                self.show_editor(ui);
                false
            }
        };
        if let Some(error) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, error.message);
            dense::view_text(
                ui,
                "Show Details",
                "Folder Task Details",
                &error.detail,
                true,
            );
        } else if let Some(status) = self.status {
            status_label(ui, status);
        }
        choose
    }

    fn show_review(&mut self, ui: &mut egui::Ui) -> bool {
        let Some(scan) = self.scan.clone() else {
            dense::heading(ui, "Choose an App Folder", "");
            path_summary(ui, "Folder", &self.source);
            return ui.button(copy::CHOOSE_FOLDER).clicked();
        };
        if scan.is_prepared() {
            dense::heading(ui, copy::PREPARED, copy::PREPARED_HELP);
            path_summary(ui, "Folder", &self.source);
            return ui.button(copy::CHOOSE_FOLDER).clicked();
        }

        dense::heading(ui, copy::REVIEW, "");
        if self.selected.is_some() {
            self.show_app_summary(ui);
            let selected = self.selected.as_deref().unwrap_or_else(|| Path::new(""));
            path_summary(ui, "Starts With", selected);
        } else {
            path_summary(ui, "Folder", &self.source);
        }
        if !self.draft.id.is_empty() && !self.output_parent.as_os_str().is_empty() {
            path_summary(ui, "Save To", &self.output());
        }
        let mut choose = false;
        if self.selected.is_some() {
            ui.horizontal_wrapped(|ui| {
                if ui.button("Edit App Details").clicked() {
                    self.page = Page::App;
                }
                #[cfg(windows)]
                {
                    let edit = ui.button("Edit Package Artwork…");
                    if edit.clicked() {
                        self.open_artwork(edit.id);
                    }
                }
            });
        }
        ui.horizontal_wrapped(|ui| {
            if self.selected.is_some() && ui.button("Build Tools and Output").clicked() {
                self.page = Page::Files;
            }
            choose = ui.button("Change App Folder").clicked();
        });
        ui.horizontal_wrapped(|ui| {
            if !scan.candidates.is_empty() {
                let label = if self.selected.is_some() {
                    "Change Start File"
                } else {
                    "Choose Start File"
                };
                if ui.button(label).clicked() {
                    self.page = Page::Program;
                }
            }
            let findings = scan.issues.len() + self.issues.len();
            if ui.button(format!("Folder Findings ({findings})")).clicked() {
                self.toggle_findings();
            }
        });
        if self.selected.is_some() {
            ui.checkbox(&mut self.draft.rights, copy::RIGHTS);
        }
        if let Some((page, message)) = self.blocker() {
            ui.horizontal_wrapped(|ui| {
                ui.label(message);
                if page != Page::Review
                    && (page != Page::Program || !scan.candidates.is_empty())
                    && ui.button("Review").clicked()
                {
                    if page == Page::Findings {
                        self.toggle_findings();
                    } else {
                        self.page = page;
                    }
                }
            });
        } else {
            ui.label("Ready to build. Your original folder stays as it is.");
        }
        choose
    }

    fn show_app_summary(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(icon) = &self.icon {
                let image = egui::Image::new(icon)
                    .fit_to_exact_size(egui::vec2(32.0, 32.0))
                    .alt_text("App icon");
                ui.add(image);
            }
            ui.vertical(|ui| {
                ui.add(egui::Label::new(egui::RichText::new(&self.draft.name).strong()).truncate())
                    .on_hover_text(&self.draft.name);
                let details = match (self.draft.publisher.trim(), self.draft.version.trim()) {
                    ("", "") if self.inferred_name => Some("Name suggested from the folder".into()),
                    ("", "") => None,
                    ("", version) => Some(format!("Version {version}")),
                    (publisher, "") => Some(publisher.to_owned()),
                    (publisher, version) => Some(format!("{publisher} · Version {version}")),
                };
                if let Some(details) = details {
                    ui.add(egui::Label::new(egui::RichText::new(&details).weak()).truncate())
                        .on_hover_text(details);
                }
            });
        });
    }

    fn show_programs(&mut self, ui: &mut egui::Ui) {
        dense::heading(
            ui,
            copy::CHOOSE_PROGRAM,
            "Choose the file you normally open.",
        );
        let Some(scan) = &self.scan else {
            return;
        };
        let mut chosen = None;
        for index in dense::page(ui, &mut self.program_page, scan.candidates.len(), 1, "File") {
            let candidate = &scan.candidates[index];
            path_summary(ui, "File", &candidate.relative_path);
            if self.selected.as_ref() == Some(&candidate.relative_path) {
                ui.label("Currently selected");
            } else if dense::primary_action(ui, true, "Use This File").clicked() {
                chosen = Some(candidate.relative_path.clone());
            }
        }
        if let Some(relative) = chosen {
            self.select(ui.ctx(), relative);
        }
    }

    fn show_editor(&mut self, ui: &mut egui::Ui) {
        dense::heading(ui, "Edit App Details", "");
        ui.horizontal_wrapped(|ui| {
            for (page, label) in [
                (Page::App, "Name and Version"),
                (Page::About, "About"),
                (Page::Package, "Package"),
                (Page::Rights, "License"),
            ] {
                let response = ui.selectable_value(&mut self.page, page, label);
                theme::focus_outline(ui, &response);
            }
        });
        match self.page {
            Page::App => {
                let old_id = suggested_id(&self.draft.name);
                if dense::text_field(ui, "App Name", &mut self.draft.name) {
                    self.inferred_name = false;
                    if old_id == self.draft.id {
                        self.draft.id = suggested_id(&self.draft.name);
                    }
                }
                dense::text_field(ui, "Publisher", &mut self.draft.publisher);
                dense::text_field(ui, "Version Number", &mut self.draft.version);
            }
            Page::About => {
                dense::text_field(ui, "App Website", &mut self.draft.homepage);
                dense::text_field(ui, "Short Description", &mut self.draft.description);
            }
            Page::Package => {
                dense::text_field(ui, "Package Name", &mut self.draft.id);
                ui.label("Category");
                egui::Grid::new("folder-package-categories")
                    .num_columns(3)
                    .show(ui, |ui| {
                        for (index, &category) in CATEGORIES.iter().enumerate() {
                            ui.radio_value(&mut self.draft.category, category, category);
                            if (index + 1) % 3 == 0 {
                                ui.end_row();
                            }
                        }
                    });
                ui.horizontal_wrapped(|ui| {
                    ui.label("Installer Language");
                    for language in [Language::English, Language::Multilingual] {
                        ui.radio_value(&mut self.draft.language, Some(language), language.label());
                    }
                });
            }
            Page::Rights => {
                ui.label("Select what applies to this app.");
                ui.checkbox(&mut self.draft.open_source, "Open source");
                ui.checkbox(&mut self.draft.freeware, "Free of charge");
                ui.checkbox(&mut self.draft.commercial_use, "Commercial use allowed");
                ui.checkbox(
                    &mut self.draft.requires_admin,
                    "Requires administrator access",
                );
            }
            _ => {}
        }
        if let Some((page, message)) = self.draft.blocker() {
            if page == self.page {
                ui.label(message);
            }
        }
    }

    fn show_files(&mut self, ui: &mut egui::Ui) {
        dense::heading(ui, "Build Tools and Output", "");
        folder_field(
            ui,
            "Launcher Generator",
            &mut self.launcher_tools,
            "Choose Launcher Folder…",
        );
        folder_field(
            ui,
            "PortableApps.com Installer",
            &mut self.installer_tools,
            "Choose Installer Folder…",
        );
        folder_field(
            ui,
            "Save In",
            &mut self.output_parent,
            "Change Save Location…",
        );
        if !self.output_parent.as_os_str().is_empty() {
            path_summary(ui, "New Folder", &self.output());
        }
    }

    fn show_findings(&mut self, ui: &mut egui::Ui) {
        dense::heading(ui, "Folder Findings", "");
        let Some(scan) = &self.scan else {
            if let Some(error) = &self.error {
                dense::diagnostic(ui, &error.detail, &mut self.issue_page);
            } else {
                ui.label("Choose an app folder to see findings.");
            }
            return;
        };
        let count = scan.issues.len() + self.issues.len();
        if count == 0 {
            ui.label("No folder problems found.");
            ui.weak(copy::UNVERIFIED);
            return;
        }
        for index in dense::page(ui, &mut self.issue_page, count, 1, "Finding") {
            let issue = scan
                .issues
                .get(index)
                .unwrap_or_else(|| &self.issues[index - scan.issues.len()]);
            let (label, color) = match issue.severity {
                IssueSeverity::Error => ("Fix before building", theme::palette(ui.ctx()).danger),
                IssueSeverity::Warning => (
                    "Review before moving the app",
                    theme::palette(ui.ctx()).warning,
                ),
                IssueSeverity::Info => ("Note", theme::palette(ui.ctx()).info),
            };
            ui.colored_label(theme::rgb(color), label);
            ui.label(&issue.message);
            if let Some(path) = &issue.relative_path {
                path_summary(ui, "File", path);
            }
        }
    }

    fn show_delivery(&self, ui: &mut egui::Ui) {
        dense::heading(ui, copy::BUILT, copy::UNVERIFIED);
        if let Some(delivery) = &self.result {
            if let Some(name) = delivery.artifact.file_name() {
                ui.label(egui::RichText::new(name.to_string_lossy()).strong());
            }
            path_summary(ui, "Installer", &delivery.artifact);
            path_summary(ui, "Build Receipt", &delivery.manifest);
        }
    }
}

fn status_label(ui: &mut egui::Ui, status: &str) {
    let response = ui.label(status);
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_role(egui::accesskit::Role::Status);
        node.set_live(egui::accesskit::Live::Polite);
    });
}

fn path_summary(ui: &mut egui::Ui, label: &str, path: &Path) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).weak());
        let value = path.to_string_lossy();
        let view = dense::path_needs_view(ui, &value, ui.available_width());
        let width = if view {
            dense::width_with_button(ui, "View Path")
        } else {
            ui.available_width()
        };
        ui.allocate_ui_with_layout(
            egui::vec2(width.max(0.0), ui.spacing().interact_size.y),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add(egui::Label::new(value.as_ref()).truncate())
                    .on_hover_text(value.as_ref());
            },
        );
        dense::view_text(ui, "View Path", label, &value, view);
    });
}

fn folder_field(ui: &mut egui::Ui, label: &str, folder: &mut PathBuf, action: &str) {
    ui.label(label);
    ui.horizontal(|ui| {
        let value = if folder.as_os_str().is_empty() {
            "Choose a folder".into()
        } else {
            folder.to_string_lossy()
        };
        let width = dense::width_with_button(ui, action);
        ui.allocate_ui_with_layout(
            egui::vec2(width, ui.spacing().interact_size.y),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add(egui::Label::new(value.as_ref()).truncate())
                    .on_hover_text(value.as_ref());
            },
        );
        if ui.button(action).clicked() {
            let dialog = rfd::FileDialog::new().set_title(label);
            if let Some(path) = dialog.pick_folder() {
                *folder = path;
            }
        }
    });
}
