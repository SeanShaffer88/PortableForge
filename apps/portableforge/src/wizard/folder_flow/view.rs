use std::path::{Path, PathBuf};

use eframe::egui;
use portableforge_core::folder::IssueSeverity;
use portableforge_core::guide::folder_flow as copy;
use portableforge_core::paf::CATEGORIES;

use super::super::{dense, open_folder, package_review, shell, theme, WizardApp};
use super::draft::{suggested_id, Language};
use super::saved_job;
use super::{Failure, Flow, IntakeMode, Page};

enum SourcePicker {
    Files,
    Folder,
}

impl WizardApp {
    pub(in crate::wizard) fn show_app_intake_picker(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let guided =
                ui.selectable_value(&mut self.folder_flow.mode, IntakeMode::Guided, copy::GUIDED);
            theme::focus_outline(ui, &guided);
            let autonomous = dense::explained(
                ui.selectable_value(
                    &mut self.folder_flow.mode,
                    IntakeMode::Autonomous,
                    copy::AUTONOMOUS,
                ),
                copy::AUTONOMOUS_HELP,
            );
            theme::focus_outline(ui, &autonomous);
            ui.separator();
            if ui.button(copy::CHOOSE_FILES).clicked() {
                self.choose_app_files(ui.ctx());
            }
            if ui.button(copy::CHOOSE_FOLDER).clicked() {
                self.choose_app_folder(ui.ctx());
            }
        });
        let saved_path = saved_job::path(&self.settings_path);
        if saved_job::revision(&saved_path).is_ok_and(|slot| slot.is_some()) {
            ui.horizontal_wrapped(|ui| {
                if dense::explained(
                    ui.button(saved_job::RESUME),
                    "Check the original files again and reopen the saved choices.",
                )
                .clicked()
                {
                    self.resume_app_job(ui.ctx());
                }
                if ui.button(saved_job::REMOVE).clicked() {
                    self.remove_app_job();
                }
            });
        }
        if let Some(error) = &self.folder_flow.error {
            ui.colored_label(ui.visuals().error_fg_color, error.message);
        } else if let Some(status) = self.folder_flow.status {
            status_label(ui, status);
        }
    }

    pub(in crate::wizard) fn show_folder_flow(&mut self, ctx: &egui::Context) {
        let previous = (
            self.folder_flow.page,
            self.folder_flow.busy(),
            self.folder_flow.build_ready(),
        );
        let heading = if self.folder_flow.mode == IntakeMode::Autonomous {
            copy::AUTONOMOUS_HEADER
        } else {
            copy::HEADER
        };
        self.show_header(ctx, heading, None);
        dense::show_help_bar(
            ctx,
            self.folder_flow.status.unwrap_or("Review the source and choose how to build this package. Move over a control or press Tab for help."),
        );
        let visuals = &ctx.style().visuals;
        let frame = egui::Frame::new()
            .fill(visuals.window_fill)
            .stroke(visuals.widgets.noninteractive.bg_stroke)
            .inner_margin(egui::Margin::symmetric(12, 4));
        egui::TopBottomPanel::bottom("folder-actions")
            .frame(frame)
            .show(ctx, |ui| {
                let mut save = false;
                ui.horizontal(|ui| {
                    let flow = &mut self.folder_flow;
                    let back = ui.add_enabled(
                        !flow.busy() && !flow.modal_open(),
                        egui::Button::new(if flow.page == Page::Done {
                            "Choose Another App"
                        } else if flow.details_next_step().is_some() {
                            "Back to Review"
                        } else {
                            "Back"
                        }),
                    );
                    if back.clicked() {
                        flow.back();
                    }
                    if !flow.busy() && !flow.modal_open() {
                        shell::register_region(ui, shell::Region::Footer, back.id);
                    }
                    if flow.can_save_for_later() {
                        let save_job = dense::explained(ui.button(saved_job::SAVE), "Keep your choices for later. You’ll confirm sharing permission when you resume.");
                        shell::register_region(ui, shell::Region::Footer, save_job.id);
                        save = save_job.clicked();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if flow.test_in_progress() {
                            #[cfg(all(windows, target_arch = "x86_64"))]
                            {
                                let stop = ui.add_enabled(
                                    !flow.runtime.stopping,
                                    egui::Button::new(copy::TEST_CANCEL),
                                );
                                if !flow.runtime.stopping {
                                    shell::register_region(ui, shell::Region::Footer, stop.id);
                                }
                                if stop.clicked() {
                                    flow.cancel_test();
                                    ctx.request_repaint();
                                }
                            }
                        } else if flow.page == Page::Done {
                            let enabled = !flow.busy() && !flow.modal_open();
                            let open = dense::primary_action(ui, enabled, "Open Output Folder");
                            if enabled {
                                shell::register_region(ui, shell::Region::Footer, open.id);
                            }
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
                            let stop = ui.button(if flow.intake_stop.is_some() {
                                copy::STOP_INTAKE
                            } else {
                                copy::STOP
                            });
                            shell::register_region(ui, shell::Region::Footer, stop.id);
                            if stop.clicked() {
                                flow.stop();
                            }
                        } else if let Some((next, label)) = flow.details_next_step() {
                            let enabled = !flow.busy() && !flow.modal_open();
                            let action = dense::primary_action(ui, enabled, label);
                            if enabled {
                                shell::register_region(ui, shell::Region::Footer, action.id);
                            }
                            if action.clicked() {
                                flow.page = next;
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
                if save {
                    self.save_app_job(ctx);
                }
            });
        if !self.folder_flow.open {
            ctx.request_repaint();
            return;
        }
        dense::task_sidebar(ctx, |ui| self.folder_flow.show_roadmap(ui));
        self.update_heading_focus(ctx);
        egui::CentralPanel::default()
            .frame(dense::content_frame(ctx))
            .show(ctx, |ui| {
                theme::paint_work_area(ui);
                dense::compact_body(ui);
                let choose = ui
                    .add_enabled_ui(!self.folder_flow.modal_open(), |ui| {
                        self.folder_flow.show_body(ui)
                    })
                    .inner;
                match choose {
                    Some(SourcePicker::Files) => self.choose_app_files(ctx),
                    Some(SourcePicker::Folder) => self.choose_app_folder(ctx),
                    None => {}
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
    fn details_next_step(&self) -> Option<(Page, &'static str)> {
        match self.page {
            Page::App => Some((Page::About, "Next: About")),
            Page::About => Some((Page::Package, "Next: Package")),
            Page::Package => Some((Page::Rights, "Next: License")),
            Page::Rights => Some((Page::Review, "Return to Review")),
            _ => None,
        }
    }

    pub(in crate::wizard) fn show_compact_roadmap(&mut self, ui: &mut egui::Ui) {
        let finished = self.page == Page::Done;
        let review = self.page == Page::Review;
        let can_review = !self.busy() && !self.modal_open() && !finished;
        ui.horizontal(|ui| {
            ui.spacing_mut().interact_size.y = theme::compact_row_height();
            let review_item = ui.add_enabled(
                can_review, egui::Button::selectable(review, "Review package"),
            );
            dense::field_help(&review_item, "Review the source files and the choices needed to build the package.");
            if review_item.clicked() {
                self.page = Page::Review;
            }
            ui.label(egui::RichText::new("›").weak());
            let current = match self.page {
                Page::Review => "Configuration",
                Page::Program => "Start file",
                Page::App | Page::About | Page::Package | Page::Rights => "App details",
                Page::Files => "Tools and output",
                Page::Findings => "File findings",
                Page::Done => "Configuration",
            };
            if review || finished {
                ui.weak(current);
            } else {
                let step = ui.selectable_label(true, current);
                dense::field_help(&step, "You are editing this part of the package. Review your choices before building.");
            }
            ui.label(egui::RichText::new("›").weak());
            if finished {
                let _result = ui.selectable_label(true, "Result");
            } else {
                ui.weak("Result");
            }
        });
    }

    fn show_roadmap(&mut self, ui: &mut egui::Ui) {
        let ready = !self.busy()
            && !self.modal_open()
            && self.scan.as_ref().is_some_and(|scan| !scan.is_prepared())
            && self.page != Page::Done;
        let selected_file = ready && self.selected.is_some();
        let finished = self.page == Page::Done;
        for (label, page, active, enabled, help) in [
            (
                "Package Review",
                Page::Review,
                self.page == Page::Review,
                ready || self.page == Page::Review,
                "Check the source, start file and required choices before creating your installer.",
            ),
            (
                "Start File",
                Page::Program,
                self.page == Page::Program,
                ready && self.scan.as_ref().is_some_and(|scan| !scan.candidates.is_empty()),
                "Choose which program to open when someone runs the portable app. Use the file you would normally open to start this app.",
            ),
            (
                "App Details",
                Page::App,
                matches!(self.page, Page::App | Page::About | Page::Package | Page::Rights),
                selected_file,
                "Set the name, publisher, version, website, and license information that appear in the portable package.",
            ),
            (
                "File Findings",
                Page::Findings,
                self.page == Page::Findings,
                ready,
                "Read what PortableForge found in the source files. Resolve anything marked to fix before building.",
            ),
        ] {
            let item = dense::roadmap_item(ui, label, active, enabled, help);
            if active && enabled {
                shell::register_region(ui, shell::Region::Sidebar, item.id);
            }
            if item.clicked() && !active {
                if page == Page::Findings {
                    self.toggle_findings();
                } else {
                    self.page = page;
                }
            }
        }
        #[cfg(windows)]
        {
            let item = dense::roadmap_item(
                ui,
                "Package Artwork",
                self.artwork.is_open(),
                selected_file,
                "Preview the app icon and the image shown on the left side of the installer. Apply changes to include them in this package.",
            );
            if item.clicked() {
                self.open_artwork(item.id);
            }
        }
        let item = dense::roadmap_item(
            ui,
            "Build & Output",
            self.page == Page::Files,
            selected_file,
            "Choose the installed PortableApps.com tools and the folder that will receive your new installer.",
        );
        if self.page == Page::Files && selected_file {
            shell::register_region(ui, shell::Region::Sidebar, item.id);
        }
        if item.clicked() {
            self.page = Page::Files;
        }
        let item = dense::roadmap_item(
            ui,
            "Build Result",
            finished,
            finished,
            "Find the installer and its build receipt after the package has been created.",
        );
        if finished {
            shell::register_region(ui, shell::Region::Sidebar, item.id);
        }
    }

    fn show_body(&mut self, ui: &mut egui::Ui) -> Option<SourcePicker> {
        if self.busy() {
            let title = if self.test_in_progress() {
                copy::TEST_RUNNING
            } else if self.status == Some(saved_job::SAVING) {
                "Save App Job"
            } else if self.discard_stage.is_some() || self.status == Some(copy::CLEANING_SOURCE) {
                copy::CLEANING_SOURCE_TITLE
            } else if self.scan.is_none() || self.status == Some(saved_job::LOADING) {
                copy::CHECKING_SOURCE
            } else if self.page == Page::Program {
                "Reading App Details"
            } else {
                copy::BUILDING
            };
            dense::heading(ui, title, "");
            if let Some(status) = self.status {
                status_label(ui, status);
            }
            if self.test_in_progress() {
                ui.label(copy::TEST_SETUP_HELP);
                ui.label(copy::TEST_LAUNCH_HELP);
                if let Some(error) = &self.error {
                    ui.colored_label(ui.visuals().error_fg_color, error.message);
                }
            } else if !self.source.as_os_str().is_empty() {
                path_summary(ui, "Source", &self.source);
            }
            return None;
        }

        if self.page == Page::Findings {
            self.show_findings(ui);
            return None;
        }

        let choose = match self.page {
            Page::Review => self.show_review(ui),
            Page::Program => {
                self.show_programs(ui);
                None
            }
            Page::Files => {
                self.show_files(ui);
                None
            }
            Page::Done => {
                self.show_delivery(ui);
                None
            }
            _ => {
                self.show_editor(ui);
                None
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

    fn show_review(&mut self, ui: &mut egui::Ui) -> Option<SourcePicker> {
        let Some(scan) = self.scan.clone() else {
            dense::heading(ui, copy::CHOOSE_SOURCE, "");
            if !self.source.as_os_str().is_empty() {
                path_summary(ui, "Source", &self.source);
            }
            return ui
                .horizontal_wrapped(|ui| {
                    if ui.button(copy::CHOOSE_FILES).clicked() {
                        Some(SourcePicker::Files)
                    } else if ui.button(copy::CHOOSE_FOLDER).clicked() {
                        Some(SourcePicker::Folder)
                    } else {
                        None
                    }
                })
                .inner;
        };
        if scan.is_prepared() {
            dense::heading(ui, copy::PREPARED, copy::PREPARED_HELP);
            path_summary(ui, "Source", &self.source);
            return ui
                .button(copy::CHOOSE_FOLDER)
                .clicked()
                .then_some(SourcePicker::Folder);
        }

        package_review::overview(ui, "App Files", &self.source_label, &self.source);
        if self.selected.is_some() {
            self.show_app_summary(ui);
            let selected = self.selected.as_deref().unwrap_or_else(|| Path::new(""));
            path_summary(ui, "Starts With", selected);
        }
        if !self.draft.id.is_empty() && !self.output_parent.as_os_str().is_empty() {
            path_summary(ui, "Save To", &self.output());
        }
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
            if ui.button(copy::CHANGE_SOURCE).clicked() {
                self.back();
                ui.ctx().request_repaint();
            }
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
            let rights = ui.checkbox(&mut self.draft.rights, copy::RIGHTS);
            dense::field_help(
                &rights,
                "Confirm that the app’s license or publisher lets you distribute these files in your installer. PortableForge needs your confirmation before it can include them.",
            );
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
        None
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
                dense::explained(
                    ui.add(
                        egui::Label::new(egui::RichText::new(&self.draft.name).strong()).truncate(),
                    ),
                    &self.draft.name,
                );
                let details = match (self.draft.publisher.trim(), self.draft.version.trim()) {
                    ("", "") if self.inferred_name => Some("Suggested app name".into()),
                    ("", "") => None,
                    ("", version) => Some(format!("Version {version}")),
                    (publisher, "") => Some(publisher.to_owned()),
                    (publisher, version) => Some(format!("{publisher} · Version {version}")),
                };
                if let Some(details) = details {
                    dense::explained(
                        ui.add(egui::Label::new(egui::RichText::new(&details).weak()).truncate()),
                        details,
                    );
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
                let name_help = "Enter the name people will see in the installer and the PortableApps.com menu. A recognizable name helps people find the portable app later.";
                let publisher_help = "Enter the person or organization that publishes the original app. Use the name on its website or About screen.";
                let changed = if ui.available_width() >= 480.0 {
                    ui.columns(2, |columns| {
                        let changed = dense::explained_text_field(
                            &mut columns[0],
                            "App Name",
                            &mut self.draft.name,
                            name_help,
                        );
                        dense::explained_text_field(
                            &mut columns[1],
                            "Publisher",
                            &mut self.draft.publisher,
                            publisher_help,
                        );
                        changed
                    })
                } else {
                    let changed = dense::explained_text_field(
                        ui,
                        "App Name",
                        &mut self.draft.name,
                        name_help,
                    );
                    dense::explained_text_field(
                        ui,
                        "Publisher",
                        &mut self.draft.publisher,
                        publisher_help,
                    );
                    changed
                };
                if changed {
                    self.inferred_name = false;
                    if old_id == self.draft.id {
                        self.draft.id = suggested_id(&self.draft.name);
                    }
                }
                dense::explained_text_field(
                    ui, "Version Number", &mut self.draft.version,
                    "Enter the version of this app in numbers, such as 2.4 or 2.4.1. The platform uses this to recognize newer packages.",
                );
            }
            Page::About => {
                dense::explained_text_field(
                    ui, "App Website", &mut self.draft.homepage,
                    "Enter the app’s official website, starting with https:// or http://. This lets people learn about the original app.",
                );
                dense::explained_text_field(
                    ui, "Short Description", &mut self.draft.description,
                    "Write a brief explanation of what the app does. People may read it to decide whether they want to install the package.",
                );
            }
            Page::Package => {
                dense::explained_text_field(
                    ui, "Package Name", &mut self.draft.id,
                    "This name identifies the package and its install folder. Use a short name without spaces, such as WidgetPortable.",
                );
                ui.label("Category");
                egui::Grid::new("folder-package-categories")
                    .num_columns(3)
                    .show(ui, |ui| {
                        for (index, &category) in CATEGORIES.iter().enumerate() {
                            if ui
                                .radio(self.draft.category == category, category)
                                .clicked()
                            {
                                self.draft.category = category.to_owned();
                            }
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
                for (value, label, help) in [
                    (&mut self.draft.open_source, "Open source", "Select this when the publisher makes the source code available under an open-source license."),
                    (&mut self.draft.freeware, "Free of charge", "Select this if people can use the app without paying. Check the publisher’s license for any limits."),
                    (&mut self.draft.commercial_use, "Commercial use allowed", "Select this when the publisher’s license allows people to use the app for work or business."),
                    (&mut self.draft.requires_admin, "Requires administrator access", "Select this if the app itself needs administrator access to run. People without that access may be unable to use it."),
                ] {
                    let choice = ui.checkbox(value, label);
                    dense::field_help(&choice, help);
                }
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
            "Choose the folder containing the PortableApps.com Launcher Generator. PortableForge uses it to create the file that starts the portable app.",
        );
        folder_field(
            ui,
            "PortableApps.com Installer",
            &mut self.installer_tools,
            "Choose Installer Folder…",
            "Choose the folder containing the PortableApps.com Installer tool. It combines the portable app and its settings into one installable file.",
        );
        folder_field(
            ui,
            "Save In",
            &mut self.output_parent,
            "Change Save Location…",
            "Choose an existing folder. PortableForge creates a new output folder there for the installer and its build receipt.",
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

    fn show_delivery(&mut self, ui: &mut egui::Ui) {
        #[cfg(all(windows, target_arch = "x86_64"))]
        let tested = self.show_test_outcome(ui);
        #[cfg(not(all(windows, target_arch = "x86_64")))]
        let tested = false;
        if !tested {
            dense::heading(ui, copy::BUILT, copy::UNVERIFIED);
        }
        if let Some(delivery) = &self.result {
            if let Some(name) = delivery.artifact.file_name() {
                ui.label(egui::RichText::new(name.to_string_lossy()).strong());
            }
            path_summary(ui, "Installer", &delivery.artifact);
            path_summary(ui, "Build Receipt", &delivery.manifest);
        }
        #[cfg(all(windows, target_arch = "x86_64"))]
        if self.result.is_some() {
            let test = ui
                .push_id("folder-test-action", |ui| ui.button(copy::TEST_INSTALLER))
                .inner;
            if test.clicked() {
                self.open_test_confirmation();
            } else {
                self.restore_test_focus(ui, &test);
            }
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
                dense::explained(
                    ui.add(egui::Label::new(value.as_ref()).truncate()),
                    value.as_ref(),
                );
            },
        );
        dense::view_text(ui, "View Path", label, &value, view);
    });
}

fn folder_field(ui: &mut egui::Ui, label: &str, folder: &mut PathBuf, action: &str, help: &str) {
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
                dense::explained(
                    ui.add(egui::Label::new(value.as_ref()).truncate()),
                    value.as_ref(),
                );
            },
        );
        let choose = ui.button(action);
        dense::field_help(&choose, help);
        if choose.clicked() {
            let dialog = rfd::FileDialog::new().set_title(label);
            if let Some(path) = dialog.pick_folder() {
                *folder = path;
            }
        }
    });
}
