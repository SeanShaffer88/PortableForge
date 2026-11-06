//! The wizard's verified capture, approval and rebuild controls.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
#[cfg(all(windows, target_arch = "x86_64"))]
use std::time::{SystemTime, UNIX_EPOCH};

use eframe::egui;
#[cfg(all(windows, target_arch = "x86_64"))]
use portableforge_core::orchestrate::{CaptureOutcome, SessionLayout};
use portableforge_core::settings::{self, SavedProfileApproval};
use portableforge_core::SandboxMode;

#[cfg(all(windows, target_arch = "x86_64"))]
use super::sessions_base;
use super::{color, dense, open_folder, picker_start_dir, texture, WizardApp};

#[derive(Default)]
pub(super) struct ProfileFlow {
    pub(super) selected_capture: bool,
    pub(super) captured: bool,
    installed_path: String,
    capture_parent: String,
    capture_setup: String,
    trusted: bool,
    launcher_tools: String,
    profile_destination: String,
    saved: Option<SavedProfileApproval>,
    approval_error: Option<String>,
    pub(super) replay_open: bool,
    selected_profile: usize,
    replay_setup: String,
    installer_tools: String,
    replay_destination: String,
    artifact: Option<PathBuf>,
    job: Option<Receiver<JobEvent>>,
    status: &'static str,
    error: Option<String>,
}

enum JobEvent {
    Stage(&'static str),
    Done(Box<Result<JobResult, String>>),
}

enum JobResult {
    #[cfg(all(windows, target_arch = "x86_64"))]
    Captured(Box<(SessionLayout, CaptureOutcome, String)>),
    #[cfg(all(windows, target_arch = "x86_64"))]
    Saved(SavedProfileApproval),
    #[cfg(all(windows, target_arch = "x86_64"))]
    Rebuilt(PathBuf),
}

impl ProfileFlow {
    #[cfg(test)]
    pub(super) fn set_layout_paths(&mut self, paths: &[String; 3]) {
        self.replay_setup.clone_from(&paths[0]);
        self.installer_tools.clone_from(&paths[1]);
        self.replay_destination.clone_from(&paths[2]);
    }

    #[cfg(test)]
    pub(super) fn populate_layout_fixture(&mut self, busy: bool) -> impl FnOnce() {
        self.replay_setup = format!(r"C:\Setups\{}\app.exe", "long-folder\\".repeat(18));
        self.installer_tools = r"C:\PortableApps\Installer".into();
        self.replay_destination = r"C:\Builds\App".into();
        let sender = if busy {
            let (sender, receiver) = mpsc::channel();
            self.job = Some(receiver);
            self.status = "Building your installer…";
            Some(sender)
        } else {
            self.error = Some("The test detected an error in a file.\n".repeat(100));
            None
        };
        move || drop(sender)
    }

    pub(super) fn profile_saved(&self) -> bool {
        self.saved.is_some()
    }

    pub(super) fn busy(&self) -> bool {
        self.job.is_some()
    }

    pub(super) fn review_fix_supported(
        &self,
        finding: &portableforge_core::classify::Finding,
    ) -> bool {
        #[cfg(all(windows, target_arch = "x86_64"))]
        {
            portableforge_core::wizard_package::supports_review_fix(
                finding,
                Some(Path::new(self.installed_path.trim())),
            )
        }
        #[cfg(not(all(windows, target_arch = "x86_64")))]
        {
            let _ = finding;
            false
        }
    }

    fn start(
        &mut self,
        ctx: &egui::Context,
        status: &'static str,
        task: impl FnOnce(&dyn Fn(&'static str)) -> Result<JobResult, String> + Send + 'static,
    ) {
        if self.busy() || ctx.input(|input| input.viewport().close_requested()) {
            return;
        }
        self.status = status;
        self.error = None;
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        match std::thread::Builder::new()
            .name("portableforge-wizard-job".into())
            .spawn(move || {
                let result = task(&|status| {
                    let _ = tx.send(JobEvent::Stage(status));
                    ctx.request_repaint();
                });
                let _ = tx.send(JobEvent::Done(Box::new(result)));
                ctx.request_repaint();
            }) {
            Ok(_) => self.job = Some(rx),
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn poll(&mut self) -> Option<Result<JobResult, String>> {
        loop {
            match self.job.as_ref()?.try_recv() {
                Ok(JobEvent::Stage(status)) => self.status = status,
                Ok(JobEvent::Done(result)) => {
                    self.job = None;
                    return Some(*result);
                }
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => {
                    self.job = None;
                    return Some(Err("The work stopped before reporting a result.".into()));
                }
            }
        }
    }

    fn show_feedback(&self, ui: &mut egui::Ui) {
        if self.busy() {
            ui.horizontal_wrapped(|ui| {
                ui.spinner();
                ui.label(self.status);
            });
            ui.label("Keep this window open until the operation finishes.");
        }
        if let Some(error) = &self.error {
            ui.colored_label(
                color(super::ALERT_TEXT),
                "The operation failed. Open the details for the reason.",
            );
            dense::view_text(ui, "View error details", "Problem details", error, true);
        }
    }
}

fn path_field(ui: &mut egui::Ui, label: &str, value: &mut String, kind: FieldKind<'_>) -> bool {
    let heading = ui.label(label);
    let browse = match kind {
        FieldKind::Typed => None,
        FieldKind::NewFolder(_) => Some("Choose parent…"),
        _ => Some("Browse…"),
    };
    let mut changed = false;
    ui.horizontal(|ui| {
        let width = browse.map_or(ui.available_width() - 12.0, |name| {
            dense::width_with_button(ui, name)
        });
        let (response, view) = dense::path_edit(ui, value, width);
        changed = response.labelled_by(heading.id).changed();
        if let Some(browse) = browse {
            if ui.button(browse).on_hover_text(label).clicked() {
                let mut dialog = rfd::FileDialog::new().set_title(label);
                if let Some(dir) = picker_start_dir(value) {
                    dialog = dialog.set_directory(dir);
                }
                let picked = match kind {
                    FieldKind::File => dialog.add_filter("Setup files", &["exe"]).pick_file(),
                    FieldKind::Folder => dialog.pick_folder(),
                    FieldKind::NewFolder(name) => dialog.pick_folder().map(|path| path.join(name)),
                    FieldKind::Typed => None,
                };
                if let Some(path) = picked {
                    *value = path.to_string_lossy().into_owned();
                    changed = true;
                }
            }
        }
        dense::view_text(ui, "View path", label, value, view);
    });
    changed
}

enum FieldKind<'a> {
    File,
    Folder,
    NewFolder(&'a str),
    Typed,
}

fn entered(value: &str) -> PathBuf {
    PathBuf::from(value.trim())
}

impl WizardApp {
    pub(super) fn verified_metadata_error(&self) -> Option<&'static str> {
        if !self.use_verified_capture() {
            return None;
        }
        #[cfg(all(windows, target_arch = "x86_64"))]
        {
            if self.publisher.trim().is_empty() {
                return Some("Enter the app publisher.");
            }
            let homepage = self.homepage.trim();
            let is_web_url = homepage.split_once("://").is_some_and(|(scheme, _)| {
                scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
            });
            if !is_web_url || !portableforge_core::wizard_package::valid_homepage(homepage) {
                return Some("Enter the app homepage as a full HTTP or HTTPS address.");
            }
            let description = self.description.trim();
            if description.is_empty() {
                return Some("Enter a short app description.");
            }
            if description.chars().count() > 512 {
                return Some("Keep the description to 512 characters or fewer.");
            }
        }
        None
    }

    pub(super) fn use_verified_capture(&self) -> bool {
        self.profile_flow.selected_capture
            || self.session_mode == Some(SandboxMode::Sandboxie)
            || self
                .preflight
                .as_ref()
                .is_some_and(|report| report.best_mode == SandboxMode::Sandboxie)
    }

    pub(super) fn enter_verified_capture(&mut self) {
        self.profile_flow.selected_capture = true;
        self.step = 2;
        self.show_details = false;
    }

    pub(super) fn poll_profile_work(&mut self) {
        let Some(result) = self.profile_flow.poll() else {
            return;
        };
        match result {
            #[cfg(all(windows, target_arch = "x86_64"))]
            Ok(JobResult::Captured(capture)) => {
                let (layout, outcome, installed_name) = *capture;
                self.layout = Some(layout);
                self.session_mode = Some(SandboxMode::Sandboxie);
                self.profile_flow.captured = true;
                self.main_exe = installed_name;
                self.hints_applied = false;
                self.apply_capture(outcome);
                for item in &mut self.review.items {
                    if !self
                        .profile_flow
                        .review_fix_supported(&item.classification.finding)
                    {
                        if let portableforge_core::review::Decision::Fix { on } = &mut item.decision
                        {
                            *on = false;
                        }
                    }
                }
                self.step = 6;
                self.show_details = false;
                self.log
                    .push("Capture complete. The dedicated box was removed.".into());
                self.mark_dirty();
            }
            #[cfg(all(windows, target_arch = "x86_64"))]
            Ok(JobResult::Saved(approval)) => {
                self.profile_flow.saved = Some(approval);
                self.settings.draft = None;
                self.settings_dirty = true;
                self.step = 11;
                self.show_details = false;
                self.save_approval();
            }
            #[cfg(all(windows, target_arch = "x86_64"))]
            Ok(JobResult::Rebuilt(artifact)) => self.profile_flow.artifact = Some(artifact),
            Err(error) => self.profile_flow.error = Some(error),
        }
    }

    fn save_approval(&mut self) {
        let Some(approval) = self.profile_flow.saved.clone() else {
            return;
        };
        match settings::save_approval(&self.settings_path, &mut self.settings, approval) {
            Ok(()) => {
                self.settings_dirty = false;
                self.last_save = std::time::Instant::now();
                self.profile_flow.approval_error = None;
                self.log
                    .push("Saved the profile and remembered its approval.".into());
            }
            Err(error) => {
                self.settings_dirty = false;
                self.profile_flow.approval_error = Some(error);
            }
        }
    }

    pub(super) fn show_verified_capture(&mut self, ui: &mut egui::Ui) {
        if self.profile_flow.captured {
            ui.label("Capture complete. The dedicated box was removed.");
            return;
        }
        let busy = self.profile_flow.busy();
        ui.add_enabled_ui(!busy, |ui| {
            if path_field(ui, "Setup file", &mut self.installer_path, FieldKind::File) {
                self.profile_flow.trusted = false;
                self.mark_dirty();
            }
            if path_field(
                ui,
                "Installed app path (.exe)",
                &mut self.profile_flow.installed_path,
                FieldKind::Typed,
            ) {
                self.profile_flow.trusted = false;
            }
            ui.label(r"Example: C:\Program Files\Example\Example.exe");
            ui.collapsing("Capture location", |ui| {
                path_field(
                    ui,
                    "Folder for capture sessions",
                    &mut self.profile_flow.capture_parent,
                    FieldKind::Folder,
                );
            });
            ui.checkbox(
                &mut self.profile_flow.trusted,
                "I trust this setup file and the installed app.",
            );
        });
        self.profile_flow.show_feedback(ui);
    }

    pub(super) fn capture_ready(&self) -> bool {
        !self.profile_flow.busy()
            && self.profile_flow.trusted
            && !self.installer_path.trim().is_empty()
            && !self.profile_flow.installed_path.trim().is_empty()
    }

    pub(super) fn start_verified_capture(&mut self, ctx: &egui::Context) {
        if !self.capture_ready() {
            return;
        }
        let setup = entered(&self.installer_path);
        let installed = entered(&self.profile_flow.installed_path);
        let parent = (!self.profile_flow.capture_parent.trim().is_empty())
            .then(|| entered(&self.profile_flow.capture_parent));
        self.profile_flow.capture_setup = self.installer_path.trim().to_owned();
        self.profile_flow
            .start(ctx, "Preparing the dedicated box…", move |stage| {
                capture_job(&setup, &installed, parent.as_deref(), stage)
            });
    }

    pub(super) fn show_profile_save(&mut self, ui: &mut egui::Ui) {
        if self.profile_flow.saved.is_some() {
            ui.label("The reviewed profile is ready.");
            return;
        }
        let busy = self.profile_flow.busy();
        ui.add_enabled_ui(!busy, |ui| {
            path_field(
                ui,
                "PortableApps.com Launcher Generator folder (2.2.9)",
                &mut self.profile_flow.launcher_tools,
                FieldKind::Folder,
            );
            ui.label("Select the folder where the Launcher Generator is installed.");
            path_field(
                ui,
                "New profile folder",
                &mut self.profile_flow.profile_destination,
                FieldKind::NewFolder("PortableForge Profile"),
            );
        });
        self.profile_flow.show_feedback(ui);
    }

    pub(super) fn profile_save_ready(&self) -> bool {
        !self.profile_flow.busy()
            && self.profile_flow.captured
            && !self.profile_flow.launcher_tools.trim().is_empty()
            && !self.profile_flow.profile_destination.trim().is_empty()
    }

    pub(super) fn start_profile_save(&mut self, ctx: &egui::Context) {
        if !self.profile_save_ready() {
            return;
        }
        let Some(layout) = &self.layout else {
            return;
        };
        let meta = match self.package_metadata() {
            Ok(meta) => meta,
            Err(error) => {
                self.profile_flow.error = Some(error);
                return;
            }
        };
        let session = layout.root.clone();
        let review = self.review.clone();
        let tools = entered(&self.profile_flow.launcher_tools);
        let destination = entered(&self.profile_flow.profile_destination);
        self.profile_flow
            .start(ctx, "Preparing and saving your profile…", move |_| {
                save_job(&session, &meta, &review, &tools, &destination)
            });
    }

    pub(super) fn show_saved_profile(&mut self, ui: &mut egui::Ui) {
        if let Some(approval) = &self.profile_flow.saved {
            let path = &approval.folder;
            ui.label(path.display().to_string());
            if ui.button("Open the profile folder").clicked() {
                if let Err(error) = open_folder(path) {
                    self.profile_flow.error = Some(error);
                }
            }
        }
        if let Some(error) = self.profile_flow.approval_error.clone() {
            ui.colored_label(
                color(super::ALERT_TEXT),
                "The profile was saved. This computer could not remember its approval.",
            );
            if ui.button("Retry remembering the approval").clicked() {
                self.save_approval();
            }
            dense::view_text(ui, "View error details", "Approval problem", &error, true);
            if let Some(approval) = &self.profile_flow.saved {
                if ui.button("Copy the approval fingerprint").clicked() {
                    ui.ctx().copy_text(approval.manifest_sha256.clone());
                }
            }
        }
        if ui
            .add_enabled(
                self.profile_flow.approval_error.is_none(),
                dense::primary("Build installer"),
            )
            .clicked()
        {
            self.open_profile_replay(true);
        }
        self.profile_flow.show_feedback(ui);
    }

    pub(super) fn open_profile_replay(&mut self, from_capture: bool) {
        self.settings.saved_profiles = settings::load(&self.settings_path).saved_profiles;
        self.profile_flow.replay_open = true;
        self.profile_flow.selected_profile = self
            .profile_flow
            .saved
            .as_ref()
            .and_then(|saved| {
                self.settings
                    .saved_profiles
                    .iter()
                    .position(|profile| profile == saved)
            })
            .unwrap_or(0);
        self.profile_flow.replay_setup = if from_capture {
            self.profile_flow.capture_setup.clone()
        } else {
            self.installer_path.clone()
        };
        self.profile_flow.replay_destination.clear();
        self.profile_flow.artifact = None;
        self.profile_flow.error = None;
    }

    pub(super) fn show_profile_replay(&mut self, ctx: &egui::Context) {
        self.show_header(ctx, "Rebuild", None);
        let busy = self.profile_flow.busy();
        let selected = self.profile_flow.selected_profile;
        let approval = self.settings.saved_profiles.get(selected).cloned();
        let ready = !busy
            && self.profile_flow.artifact.is_none()
            && approval.is_some()
            && !self.profile_flow.replay_setup.trim().is_empty()
            && !self.profile_flow.installer_tools.trim().is_empty()
            && !self.profile_flow.replay_destination.trim().is_empty();
        egui::TopBottomPanel::bottom("profile-replay-back").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.add_enabled(!busy, egui::Button::new("Back")).clicked() {
                    self.profile_flow.replay_open = false;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.profile_flow.artifact.is_some() {
                        if ui.button("Build another").clicked() {
                            self.profile_flow.replay_destination.clear();
                            self.profile_flow.artifact = None;
                        }
                    } else if ui
                        .add_enabled(ready, dense::primary("Build installer"))
                        .clicked()
                    {
                        if let Some(approval) = approval {
                            let setup = entered(&self.profile_flow.replay_setup);
                            let tools = entered(&self.profile_flow.installer_tools);
                            let destination = entered(&self.profile_flow.replay_destination);
                            self.profile_flow.artifact = None;
                            self.profile_flow
                                .start(ctx, "Building your installer…", move |_| {
                                    rebuild_job(&setup, &approval, &tools, &destination)
                                });
                        }
                    }
                });
            });
        });
        egui::CentralPanel::default()
            .frame(dense::content_frame(ctx))
            .show(ctx, |ui| {
                texture::paint(ui);
                if let Some(artifact) = &self.profile_flow.artifact {
                    dense::heading(
                        ui,
                        "Your installer is ready",
                        "Open the folder to access the installer.",
                    );
                    let path = artifact.display().to_string();
                    if dense::path_needs_view(ui, &path, ui.available_width()) {
                        dense::view_text(ui, "View path", "Installer path", &path, true);
                    } else {
                        ui.label(path);
                    }
                    if ui.button("Open the installer folder").clicked() {
                        if let Some(folder) = artifact.parent() {
                            if let Err(error) = open_folder(folder) {
                                self.profile_flow.error = Some(error);
                            }
                        }
                    }
                    self.profile_flow.show_feedback(ui);
                    return;
                }
                dense::heading(
                    ui,
                    "Rebuild an installer",
                    "PortableForge verifies the setup file's SHA-256 and the saved approval before building.",
                );
                if self.settings.saved_profiles.is_empty() {
                    ui.label("Capture, review and save a profile before rebuilding.");
                    return;
                }
                ui.add_enabled_ui(!busy, |ui| {
                    let label = ui.label("Saved profile");
                    let selected = &mut self.profile_flow.selected_profile;
                    for index in dense::page(
                        ui,
                        selected,
                        self.settings.saved_profiles.len(),
                        1,
                        "Profile",
                    ) {
                        let approval = &self.settings.saved_profiles[index];
                        ui.label(&approval.label).labelled_by(label.id);
                        let path = approval.folder.display().to_string();
                        if dense::path_needs_view(ui, &path, ui.available_width()) {
                            dense::view_text(ui, "View path", "Profile path", &path, true);
                        } else {
                            ui.label(path);
                        }
                    }
                    path_field(
                        ui,
                        "Matching setup file",
                        &mut self.profile_flow.replay_setup,
                        FieldKind::File,
                    );
                    path_field(
                        ui,
                        "PortableApps.com Installer folder (3.9.19)",
                        &mut self.profile_flow.installer_tools,
                        FieldKind::Folder,
                    );
                    path_field(
                        ui,
                        "New build folder",
                        &mut self.profile_flow.replay_destination,
                        FieldKind::NewFolder("PortableForge Installer"),
                    );
                });
                self.profile_flow.show_feedback(ui);
            });
    }
}

fn capture_job(
    setup: &Path,
    installed: &Path,
    selected_parent: Option<&Path>,
    stage: &dyn Fn(&'static str),
) -> Result<JobResult, String> {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        use portableforge_core::sbie_verified::{capture_with_progress, CaptureStage};

        let default_parent = sessions_base();
        let parent = match selected_parent {
            Some(parent) => parent,
            None => {
                std::fs::create_dir_all(&default_parent).map_err(|error| error.to_string())?;
                &default_parent
            }
        };
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let output = parent.join(format!("Verified-{unique}"));
        let installed_name = installed
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("The installed app path needs a file name.")?
            .to_string();
        let root = capture_with_progress(
            setup,
            installed,
            &output,
            &[],
            std::time::Duration::from_secs(600),
            |state| {
                stage(match state {
                    CaptureStage::Preparing => "Preparing the dedicated box…",
                    CaptureStage::Installing => "Complete setup in its window.",
                    CaptureStage::Exercising => "Try the app, then close it.",
                    CaptureStage::CleaningUp => "Checking capture and removing the dedicated box…",
                })
            },
        )?;
        let layout = crate::session_layout_for(&root);
        let outcome = portableforge_core::orchestrate::read_capture(&layout)?;
        Ok(JobResult::Captured(Box::new((
            layout,
            outcome,
            installed_name,
        ))))
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        let _ = (setup, installed, selected_parent, stage);
        Err("Verified quick capture needs 64-bit Windows on an x64 computer.".into())
    }
}

fn save_job(
    session: &Path,
    meta: &portableforge_core::PafMetadata,
    review: &portableforge_core::review::Review,
    tools: &Path,
    destination: &Path,
) -> Result<JobResult, String> {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        let result = portableforge_core::wizard_package::save_reviewed_profile(
            session,
            meta,
            review,
            tools,
            destination,
        )?;
        let folder = result
            .manifest
            .parent()
            .ok_or("The profile folder is missing.")?;
        Ok(JobResult::Saved(SavedProfileApproval {
            folder: folder.to_path_buf(),
            label: meta.name.clone(),
            manifest_sha256: result.manifest_sha256,
        }))
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        let _ = (session, meta, review, tools, destination);
        Err("Saving this capture needs 64-bit Windows on an x64 computer.".into())
    }
}

fn rebuild_job(
    setup: &Path,
    approval: &SavedProfileApproval,
    tools: &Path,
    destination: &Path,
) -> Result<JobResult, String> {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        let result = portableforge_core::wizard_package::build_reviewed_profile(
            setup,
            &approval.folder,
            &approval.manifest_sha256,
            tools,
            destination,
        )?;
        Ok(JobResult::Rebuilt(result.artifact))
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        let _ = (setup, approval, tools, destination);
        Err("Making this installer needs 64-bit Windows on an x64 computer.".into())
    }
}
