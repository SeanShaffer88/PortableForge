use std::path::{Path, PathBuf};

use eframe::egui;
use portableforge_core::folder::IssueSeverity;
use portableforge_core::guide::folder_flow as copy;
use portableforge_core::paf::CATEGORIES;

#[cfg(windows)]
use super::super::artwork_ui::ArtworkPage;
use super::super::{dense, file_picker, package_review, shell, theme, WizardApp};
use super::draft::{suggested_id, Language};
use super::saved_job;
use super::{Flow, Page};

const ROADMAP: [(&str, &[Page], &str); 7] = [
    (
        "Package Review",
        &[Page::Review],
        "Check the files you selected for this package.",
    ),
    (
        "Start File",
        &[Page::Program],
        "Choose the file that starts the app.",
    ),
    (
        "App Details",
        &[Page::App, Page::About, Page::Package, Page::Rights],
        "Review the app name, publisher, package details, and sharing permission.",
    ),
    (
        "File Findings",
        &[Page::Findings],
        "Read what PortableForge found in your source files.",
    ),
    (
        "Package Artwork",
        &[Page::Icon, Page::InstallerArtwork],
        "Edit the app icon and installer image.",
    ),
    (
        "Build & Output",
        &[Page::Tools, Page::Output, Page::Build],
        "Choose the tools and save location, then build the installer.",
    ),
    (
        "Build Result",
        &[Page::Done, Page::Test],
        "Find and test the installer you built.",
    ),
];

impl Page {
    fn roadmap_position(self) -> (usize, usize) {
        ROADMAP
            .iter()
            .enumerate()
            .find_map(|(group, (_, pages, _))| {
                pages
                    .iter()
                    .position(|page| *page == self)
                    .map(|step| (group, step))
            })
            .expect("each folder page has a roadmap row")
    }
}

impl WizardApp {
    pub(in crate::wizard) fn show_app_intake_picker(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("Archive or program files").strong());
        ui.horizontal_wrapped(|ui| {
            let files = dense::explained(
                ui.button(copy::CHOOSE_FILES),
                "Choose a ZIP or select the program and its companion files together. For another archive format, extract it first and choose the extracted folder.",
            );
            self.choose_app_files(&files);
            let folder = dense::explained(
                ui.button(copy::CHOOSE_FOLDER),
                "Choose the folder containing the program and its files. PortableForge will suggest the file that opens the program.",
            );
            self.choose_app_folder(&folder);
            dense::view_text(
                ui,
                "Help me choose",
                "Which source should I choose?",
                "Choose Setup file below if the download page calls the file an installer or setup. If you downloaded a ZIP containing the app, choose the ZIP here. For other archive formats, extract the files first and choose the extracted folder.",
                true,
            );
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
                let remove = ui.button(saved_job::REMOVE);
                self.remove_app_job(&remove);
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
        let heading = copy::HEADER;
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
                let mut save = None;
                ui.horizontal(|ui| {
                    let flow = &mut self.folder_flow;
                    let back = ui.add_enabled(
                        !flow.busy() && !flow.modal_open(),
                        egui::Button::new(if flow.page == Page::Done {
                            "Choose Another App"
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
                        save = Some(save_job);
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
                        } else if flow.page.is_result() {
                            let enabled = !flow.busy() && !flow.modal_open();
                            if let Some(folder) = flow.result.as_ref().and_then(|delivery| delivery.artifact.parent()) {
                                ui.add_enabled_ui(enabled, |ui| {
                                    dense::view_text(ui, "View Output Folder", "Output folder", &folder.display().to_string(), true);
                                });
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
                        } else if flow.show_build_action() {
                            let build = dense::primary_action(ui, flow.build_ready(), copy::BUILD);
                            if flow.build_ready() {
                                shell::register_region(ui, shell::Region::Footer, build.id);
                            }
                            if build.clicked() {
                                flow.build(ctx);
                            }
                        }
                        if let Some((_, label)) = flow.next_step().filter(|_| !flow.busy()) {
                            let enabled = flow.can_continue();
                            let action = dense::primary_action(ui, enabled, label);
                            if enabled {
                                shell::register_region(ui, shell::Region::Footer, action.id);
                            }
                            if action.clicked() {
                                flow.continue_page(ctx);
                            }
                        }
                    });
                });
                if let Some(save) = save {
                    self.choose_save_location(&save);
                }
            });
        if !self.folder_flow.open {
            ctx.request_repaint();
            return;
        }
        dense::task_sidebar(ctx, |ui| self.folder_flow.show_roadmap(ui));
        self.update_heading_focus(ctx);
        self.show_activity_side(ctx);
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
                if let Some(paths) = choose {
                    self.inspect_app_inputs(ctx, paths, false);
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
    fn next_step(&self) -> Option<(Page, &'static str)> {
        let next = match self.page {
            Page::Review
                if self
                    .scan
                    .as_ref()
                    .is_some_and(|scan| !scan.is_prepared() && !scan.candidates.is_empty()) =>
            {
                Page::Program
            }
            Page::Program => Page::App,
            Page::App => Page::About,
            Page::About => Page::Package,
            Page::Package => Page::Rights,
            Page::Rights => Page::Findings,
            Page::Findings if cfg!(windows) => Page::Icon,
            Page::Findings => Page::Tools,
            Page::Icon => Page::InstallerArtwork,
            Page::InstallerArtwork => return Some((Page::Tools, "Apply Artwork")),
            Page::Tools => Page::Output,
            Page::Output => Page::Build,
            Page::Done if cfg!(all(windows, target_arch = "x86_64")) => Page::Test,
            _ => return None,
        };
        Some((next, "Next"))
    }

    pub(in crate::wizard) fn can_continue(&self) -> bool {
        !self.busy()
            && !self.modal_open()
            && self.next_step().is_some()
            && match self.page {
                Page::Program | Page::Findings => self.selected.is_some(),
                Page::InstallerArtwork => self.artwork.can_apply_folder_page(),
                Page::Done => self.result.is_some(),
                _ => true,
            }
    }

    pub(in crate::wizard) fn continue_page(&mut self, ctx: &egui::Context) {
        if !self.can_continue() {
            return;
        }
        if let Some((next, _)) = self.next_step() {
            if self.page == Page::InstallerArtwork && self.artwork.apply_folder_page() {
                self.refresh_review_icon(ctx);
            }
            self.go_to(next);
            ctx.request_repaint();
        }
    }

    pub(in crate::wizard) fn show_compact_roadmap(&mut self, ui: &mut egui::Ui) {
        let finished = self.page.is_result();
        let review = self.page == Page::Review;
        let can_review = !self.busy() && !self.modal_open() && !finished;
        ui.horizontal(|ui| {
            ui.spacing_mut().interact_size.y = theme::compact_row_height();
            let review_item = ui.add_enabled(
                can_review, egui::Button::selectable(review, "Review package"),
            );
            dense::field_help(&review_item, "Review the source files and the choices needed to build the package.");
            if review_item.clicked() {
                self.go_to(Page::Review);
            }
            ui.label(egui::RichText::new("›").weak());
            let (group, _) = self.page.roadmap_position();
            let current = if review || finished { "Configuration" } else { ROADMAP[group].0 };
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
            && self.scan.as_ref().is_some_and(|scan| !scan.is_prepared());
        let finished = self.result.is_some();
        let editable = ready && !finished;
        let selected = editable && self.selected.is_some();
        let (current, substep) = self.page.roadmap_position();
        self.roadmap_positions[current] = Some(substep);
        for (group, (label, pages, help)) in ROADMAP.into_iter().enumerate() {
            let active = group == current;
            let enabled = match pages[0] {
                Page::Review => editable || (active && !self.busy()),
                Page::Program => {
                    editable
                        && self
                            .scan
                            .as_ref()
                            .is_some_and(|scan| !scan.candidates.is_empty())
                }
                Page::App | Page::Tools => selected,
                Page::Findings => editable,
                Page::Icon => selected && cfg!(windows),
                Page::Done => finished && !self.busy() && !self.modal_open(),
                _ => unreachable!("roadmap rows start with an entry page"),
            };
            let last = self.roadmap_positions[group];
            let progress = (pages.len() > 1).then_some((last, pages.len()));
            let item =
                dense::roadmap_item_with_progress(ui, label, active, enabled, help, progress);
            if active && enabled {
                shell::register_region(ui, shell::Region::Sidebar, item.id);
            }
            if item.clicked() && !active {
                self.go_to(pages[last.unwrap_or_default()]);
            }
        }
    }

    fn show_body(&mut self, ui: &mut egui::Ui) -> Option<Vec<PathBuf>> {
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
            Page::Tools => {
                self.show_tools(ui);
                None
            }
            Page::Output => {
                self.show_output(ui);
                None
            }
            Page::Build => {
                self.show_build(ui);
                None
            }
            Page::Done => {
                self.show_delivery(ui);
                None
            }
            Page::Test => {
                self.show_test_page(ui);
                None
            }
            Page::Icon | Page::InstallerArtwork => {
                self.show_artwork_page(ui);
                None
            }
            Page::App | Page::About | Page::Package | Page::Rights => {
                self.show_editor(ui);
                None
            }
            Page::Findings => unreachable!("findings render before other pages"),
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

    fn show_review(&mut self, ui: &mut egui::Ui) -> Option<Vec<PathBuf>> {
        let Some(scan) = self.scan.clone() else {
            dense::heading(ui, copy::CHOOSE_SOURCE, "");
            if !self.source.as_os_str().is_empty() {
                path_summary(ui, "Source", &self.source);
            }
            return ui
                .horizontal_wrapped(|ui| {
                    let files = ui.button(copy::CHOOSE_FILES);
                    let folder = ui.button(copy::CHOOSE_FOLDER);
                    file_picker::choose(
                        &files,
                        "Choose a ZIP or files for one app",
                        file_picker::Mode::Files(&[]),
                        None,
                    )
                    .or_else(|| {
                        file_picker::choose(
                            &folder,
                            "Choose the unpacked app folder",
                            file_picker::Mode::Folder,
                            None,
                        )
                    })
                })
                .inner;
        };
        if scan.is_prepared() {
            dense::heading(ui, copy::PREPARED, copy::PREPARED_HELP);
            path_summary(ui, "Source", &self.source);
            return file_picker::choose(
                &ui.button(copy::CHOOSE_FOLDER),
                "Choose the unpacked app folder",
                file_picker::Mode::Folder,
                None,
            );
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
        if scan.candidates.is_empty() {
            ui.label("Choose a folder containing the file that starts the app.");
        }
        if ui.button(copy::CHANGE_SOURCE).clicked() {
            self.back();
            ui.ctx().request_repaint();
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

    fn show_substep_heading(&self, ui: &mut egui::Ui, title: &str) {
        let (group, step) = self.page.roadmap_position();
        let (name, pages, _) = ROADMAP[group];
        dense::heading(
            ui,
            title,
            &format!("{name} · {} of {}", step + 1, pages.len()),
        );
    }

    fn show_editor(&mut self, ui: &mut egui::Ui) {
        match self.page {
            Page::App => {
                self.show_substep_heading(ui, "Edit App Details");
                let old_id = suggested_id(&self.draft.name);
                let name_help = "Enter the name people will see in the installer and the PortableApps.com menu. A recognizable name helps people find the portable app later.";
                let version_help = "Enter the version of this app in numbers, such as 2.4 or 2.4.1. The platform uses this to recognize newer packages.";
                let changed = ui.columns(2, |columns| {
                    let changed = dense::explained_text_field(
                        &mut columns[0],
                        "App Name",
                        &mut self.draft.name,
                        name_help,
                    );
                    dense::explained_text_field(
                        &mut columns[1],
                        "Version Number",
                        &mut self.draft.version,
                        version_help,
                    );
                    changed
                });
                if changed {
                    self.inferred_name = false;
                    if old_id == self.draft.id {
                        self.draft.id = suggested_id(&self.draft.name);
                    }
                }
            }
            Page::About => {
                self.show_substep_heading(ui, "Describe the App");
                dense::explained_text_field(
                    ui, "Publisher", &mut self.draft.publisher,
                    "Enter the person or organization that publishes the original app. Use the name on its website or About screen.",
                );
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
                self.show_substep_heading(ui, "Configure the Package");
                dense::explained_text_field(
                    ui, "Package Name", &mut self.draft.id,
                    "This name identifies the package and its install folder. Use a short name without spaces, such as WidgetPortable.",
                );
                ui.horizontal(|ui| {
                    ui.label("Category");
                    let category = egui::ComboBox::from_id_salt("folder-package-category")
                        .selected_text(&self.draft.category)
                        .show_ui(ui, |ui| {
                            for &category in CATEGORIES {
                                if ui.selectable_label(self.draft.category == category, category).clicked() {
                                    self.draft.category = category.to_owned();
                                }
                            }
                        });
                    dense::field_help(&category.response, "Choose the category where people should find the app in the PortableApps.com menu.");
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label("Installer Language");
                    for language in [Language::English, Language::Multilingual] {
                        ui.radio_value(&mut self.draft.language, Some(language), language.label());
                    }
                });
            }
            Page::Rights => {
                self.show_substep_heading(ui, "Confirm Sharing Permission");
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
                let rights = ui.checkbox(&mut self.draft.rights, copy::RIGHTS);
                dense::field_help(
                    &rights,
                    "Confirm that the app’s license or publisher lets you distribute these files in your installer.",
                );
            }
            _ => unreachable!("only app details pages use this editor"),
        }
        if let Some((page, message)) = self.draft.blocker() {
            if page == self.page {
                ui.label(message);
            }
        }
    }

    fn show_tools(&mut self, ui: &mut egui::Ui) {
        self.show_substep_heading(ui, "Choose Build Tools");
        folder_field(
            ui,
            "Launcher Generator",
            &mut self.launcher_tools,
            "Choose Launcher Folder…",
            "Choose the Launcher Generator folder. It builds the file that starts your portable app and applies its settings rules.",
        );
        folder_field(
            ui,
            "PortableApps.com Installer",
            &mut self.installer_tools,
            "Choose Installer Folder…",
            "Choose the Installer tool folder. It packages your app and launcher into the .paf.exe you can install or share.",
        );
    }

    fn show_output(&mut self, ui: &mut egui::Ui) {
        self.show_substep_heading(ui, "Choose Where to Save");
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

    fn show_build(&mut self, ui: &mut egui::Ui) {
        self.show_substep_heading(ui, copy::BUILD);
        if let Some((page, message)) = self.blocker() {
            ui.label(message);
            if ui.button("Review Required Choice").clicked() {
                self.go_to(page);
            }
            return;
        }
        self.show_app_summary(ui);
        path_summary(ui, "Save To", &self.output());
        ui.label("Ready to build. Your original files stay in place.");
    }

    fn show_artwork_page(&mut self, ui: &mut egui::Ui) {
        #[cfg(windows)]
        {
            let (title, artwork) = if self.page == Page::Icon {
                ("Edit App Icon", ArtworkPage::Icon)
            } else {
                ("Edit Installer Artwork", ArtworkPage::Sidebar)
            };
            self.show_substep_heading(ui, title);
            if !self.artwork.is_open() {
                let icon = self
                    .base_icon
                    .clone()
                    .unwrap_or_else(super::default_application_icon);
                self.artwork.begin_folder_page(icon);
            }
            self.artwork.show_folder_page(ui, artwork);
            if ui.button("Discard Artwork Changes").clicked() {
                self.go_to(Page::Findings);
            }
        }
        #[cfg(not(windows))]
        let _ = ui;
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
        self.show_substep_heading(ui, copy::BUILT);
        ui.weak(copy::UNVERIFIED);
        if let Some(delivery) = &self.result {
            if let Some(name) = delivery.artifact.file_name() {
                ui.label(egui::RichText::new(name.to_string_lossy()).strong());
            }
            path_summary(ui, "Installer", &delivery.artifact);
            path_summary(ui, "Build Receipt", &delivery.manifest);
        }
    }

    fn show_test_page(&mut self, ui: &mut egui::Ui) {
        #[cfg(all(windows, target_arch = "x86_64"))]
        {
            if !self.show_test_outcome(ui) {
                self.show_substep_heading(ui, "Test the Installer");
                ui.label("Check installation and see whether changes survive two launches.");
            }
            if self.result.is_none() {
                return;
            }
            let test = ui
                .push_id("folder-test-action", |ui| ui.button(copy::TEST_INSTALLER))
                .inner;
            if test.clicked() {
                self.open_test_confirmation();
            } else {
                self.restore_test_focus(ui, &test);
            }
        }
        #[cfg(not(all(windows, target_arch = "x86_64")))]
        let _ = ui;
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
        let name = ui.label(egui::RichText::new(label).weak());
        if label == "Build Receipt" {
            dense::field_help(&name, "Compare the receipt’s SHA-256 with the installer’s SHA-256 to check whether the file has changed.");
        }
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
        let start = choose
            .clicked()
            .then(|| super::super::picker_start_dir(&folder.to_string_lossy()))
            .flatten();
        if let Some(path) = file_picker::choose(&choose, label, file_picker::Mode::Folder, start)
            .and_then(|paths| paths.into_iter().next())
        {
            *folder = path;
        }
    });
}
