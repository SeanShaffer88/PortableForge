//! The wizard's verified capture, approval and rebuild controls.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
#[cfg(all(windows, target_arch = "x86_64"))]
use std::time::{SystemTime, UNIX_EPOCH};

use eframe::egui;
#[cfg(all(windows, target_arch = "x86_64"))]
use portableforge_core::orchestrate::{CaptureOutcome, SessionLayout};
#[cfg(all(windows, target_arch = "x86_64"))]
use portableforge_core::sbie_verified::CancellationToken;
use portableforge_core::settings::{self, CleanedCapture, SavedBuild, SavedProfileApproval};
use portableforge_core::SandboxMode;

#[cfg(all(windows, target_arch = "x86_64"))]
use super::sessions_base;
use super::{color, dense, open_folder, picker_start_dir, texture, WizardApp};

#[derive(Default)]
pub(super) struct ProfileFlow {
    pub(super) selected_capture: bool,
    pub(super) captured: bool,
    installed_path: String,
    setup_args: String,
    companions: Vec<PathBuf>,
    setup_options_open: bool,
    capture_parent: String,
    capture_setup: String,
    trusted: bool,
    prerequisites: Option<Prerequisites>,
    pub(super) msi_ready: Option<Result<(), String>>,
    pub(super) cleaned_capture: Option<CleanedCapture>,
    #[cfg(all(windows, target_arch = "x86_64"))]
    cancellation: Option<CancellationToken>,
    #[cfg(all(windows, target_arch = "x86_64"))]
    candidate: Option<CandidateChoice>,
    launcher_tools: String,
    profile_destination: String,
    saved: Option<SavedProfileApproval>,
    approval_error: Option<String>,
    pub(super) replay_open: bool,
    selected_profile: usize,
    profile_page: usize,
    profile_search: String,
    profile_actions_open: bool,
    relocate_destination: String,
    pub(super) update_base: Option<SavedProfileApproval>,
    update_details: Option<String>,
    replay_setup: String,
    installer_tools: String,
    replay_destination: String,
    build: Option<SavedBuild>,
    library_tab: LibraryTab,
    result_page: usize,
    queue: Vec<QueuedBuild>,
    queue_page: usize,
    active_queue: Option<usize>,
    queue_running: bool,
    build_details: Option<(&'static str, String)>,
    runtime_confirm: Option<SavedBuild>,
    job: Option<Receiver<JobEvent>>,
    status: &'static str,
    error: Option<String>,
}

enum JobEvent {
    Stage(&'static str),
    #[cfg(all(windows, target_arch = "x86_64"))]
    Choose(CandidateChoice),
    Done(Box<Result<JobResult, String>>),
}

#[cfg(all(windows, target_arch = "x86_64"))]
struct CandidateChoice {
    paths: Vec<PathBuf>,
    page: usize,
    reply: mpsc::Sender<PathBuf>,
}

struct Prerequisites {
    capture: Result<(), String>,
    launcher: Option<Result<(), String>>,
    installer: Option<Result<(), String>>,
}

enum JobResult {
    #[cfg(all(windows, target_arch = "x86_64"))]
    Captured(Box<CaptureComplete>),
    Checked(Prerequisites),
    MsiChecked(Result<(), String>),
    #[cfg(windows)]
    MsiStarted(portableforge_core::orchestrate::SessionLayout),
    #[cfg(all(windows, target_arch = "x86_64"))]
    Saved(SavedProfileApproval),
    #[cfg(all(windows, target_arch = "x86_64"))]
    Rebuilt(Box<(SavedProfileApproval, SavedBuild)>),
    #[cfg(all(windows, target_arch = "x86_64"))]
    Relocated(SavedProfileApproval, PathBuf),
    #[cfg(all(windows, target_arch = "x86_64"))]
    UpdateCompared(String),
    #[cfg(all(windows, target_arch = "x86_64"))]
    BuildDetails(&'static str, String),
}

#[cfg(all(windows, target_arch = "x86_64"))]
struct CaptureComplete {
    layout: SessionLayout,
    outcome: CaptureOutcome,
    installed: PathBuf,
    receipt: CleanedCapture,
    reopened: bool,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum LibraryTab {
    #[default]
    Profiles,
    Queue,
    Results,
}

#[derive(Clone)]
struct ReplayRequest {
    setup: PathBuf,
    approval: SavedProfileApproval,
    tools: PathBuf,
    destination: PathBuf,
}

struct QueuedBuild {
    request: ReplayRequest,
    state: QueueState,
}

enum QueueState {
    Waiting,
    Building,
    #[cfg(all(windows, target_arch = "x86_64"))]
    Built(SavedBuild),
    Failed(String),
}

impl ProfileFlow {
    #[cfg(test)]
    pub(super) fn populate_prerequisites_fixture(&mut self) {
        self.prerequisites = Some(Prerequisites {
            capture: Err("The dedicated box is unavailable on this computer.".into()),
            launcher: Some(Ok(())),
            installer: None,
        });
    }

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

    pub(super) fn exclusion_reason(
        &self,
        finding: &portableforge_core::classify::Finding,
    ) -> &'static str {
        use portableforge_core::classify::Finding;
        match finding {
            Finding::AssociationCandidate { .. } => {
                "Excluded because it would change which app opens files on this computer."
            }
            Finding::RegistryKey { .. } => {
                "Excluded because the setting is outside the supported app-specific settings area."
            }
            Finding::DirectoriesMove { .. } | Finding::FilesMove { .. } => {
                "Excluded because this location cannot be moved safely into the portable copy."
            }
            Finding::RedFlag { .. } => {
                "This system change needs review. The portable copy does not apply a fix for it."
            }
            _ => "This change has no supported portable fix.",
        }
    }

    fn start(
        &mut self,
        ctx: &egui::Context,
        status: &'static str,
        task: impl FnOnce(&dyn Fn(&'static str)) -> Result<JobResult, String> + Send + 'static,
    ) -> bool {
        self.start_with_events(ctx, status, move |tx, ctx| {
            task(&|status| {
                let _ = tx.send(JobEvent::Stage(status));
                ctx.request_repaint();
            })
        })
    }

    fn start_with_events(
        &mut self,
        ctx: &egui::Context,
        status: &'static str,
        task: impl FnOnce(&mpsc::Sender<JobEvent>, &egui::Context) -> Result<JobResult, String>
            + Send
            + 'static,
    ) -> bool {
        if self.busy() || ctx.input(|input| input.viewport().close_requested()) {
            return false;
        }
        self.status = status;
        self.error = None;
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        match std::thread::Builder::new()
            .name("portableforge-wizard-job".into())
            .spawn(move || {
                let result = task(&tx, &ctx);
                let _ = tx.send(JobEvent::Done(Box::new(result)));
                ctx.request_repaint();
            }) {
            Ok(_) => {
                self.job = Some(rx);
                true
            }
            Err(error) => {
                self.error = Some(error.to_string());
                false
            }
        }
    }

    fn poll(&mut self) -> Option<Result<JobResult, String>> {
        loop {
            match self.job.as_ref()?.try_recv() {
                Ok(JobEvent::Stage(status)) => self.status = status,
                #[cfg(all(windows, target_arch = "x86_64"))]
                Ok(JobEvent::Choose(choice)) => {
                    if !self
                        .cancellation
                        .as_ref()
                        .is_some_and(CancellationToken::is_cancelled)
                    {
                        self.candidate = Some(choice);
                    }
                }
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

    pub(super) fn reset_trust(&mut self) {
        self.trusted = false;
        if !self.captured {
            self.cleaned_capture = None;
        }
    }

    pub(super) fn detach_capture(&mut self) {
        self.captured = false;
        self.reset_trust();
        self.saved = None;
        self.update_details = None;
        self.installed_path.clear();
        self.capture_setup.clear();
        self.setup_args.clear();
        self.companions.clear();
    }

    #[cfg(all(windows, target_arch = "x86_64"))]
    fn cancel_capture(&mut self) {
        if let Some(cancellation) = &self.cancellation {
            match cancellation.cancel() {
                Ok(()) => {
                    self.candidate = None;
                    self.status = "Stopping and removing the dedicated box…";
                }
                Err(error) => self.error = Some(error),
            }
        }
    }

    #[cfg(all(windows, target_arch = "x86_64"))]
    fn show_candidate(&mut self, ui: &mut egui::Ui) {
        let Some(choice) = &mut self.candidate else {
            return;
        };
        ui.label("Choose the app that should open when you use the portable copy.");
        let mut picked = false;
        for i in dense::page(ui, &mut choice.page, choice.paths.len(), 1, "App") {
            let path = choice.paths[i].display().to_string();
            dense::view_text(ui, "View app path", "Installed app path", &path, true);
            if ui
                .button(format!(
                    "Use {}",
                    choice.paths[i]
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                ))
                .clicked()
            {
                picked = true;
                if choice.reply.send(choice.paths[i].clone()).is_err() {
                    self.error = Some("The capture stopped before receiving your choice.".into());
                }
            }
        }
        if picked {
            self.candidate = None;
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

    fn show_prerequisites(&self, ui: &mut egui::Ui) {
        let Some(checks) = &self.prerequisites else {
            ui.label("Check that the dedicated box is available.");
            return;
        };
        let mut errors = Vec::new();
        for (tool, result) in [
            ("Sandboxie-Plus 1.18.8", Some(&checks.capture)),
            ("Launcher Generator", checks.launcher.as_ref()),
            ("Installer maker", checks.installer.as_ref()),
        ] {
            let status = match result {
                Some(Ok(())) => "Ready",
                Some(Err(error)) => {
                    errors.push(format!("{tool}\n{error}"));
                    "Needs attention"
                }
                None => "Choose folder",
            };
            ui.label(format!("{tool} · {status}"));
        }
        if !errors.is_empty() {
            dense::view_text(
                ui,
                "View prerequisite details",
                "Prerequisites",
                &errors.join("\n\n"),
                true,
            );
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
            || (!self.uses_guided_msi()
                && (self.session_mode == Some(SandboxMode::Sandboxie)
                    || self
                        .preflight
                        .as_ref()
                        .is_some_and(|report| report.best_mode == SandboxMode::Sandboxie)))
    }

    pub(super) fn enter_verified_capture(&mut self) {
        self.profile_flow.selected_capture = true;
        self.step = 2;
        self.show_details = false;
    }

    pub(super) fn poll_profile_work(&mut self, ctx: &egui::Context) {
        let Some(result) = self.profile_flow.poll() else {
            return;
        };
        #[cfg(all(windows, target_arch = "x86_64"))]
        {
            self.profile_flow.cancellation = None;
            self.profile_flow.candidate = None;
        }
        let active_queue = self.profile_flow.active_queue.take();
        match result {
            #[cfg(all(windows, target_arch = "x86_64"))]
            Ok(JobResult::Captured(capture)) => {
                let CaptureComplete {
                    layout,
                    outcome,
                    installed,
                    receipt,
                    reopened,
                } = *capture;
                self.layout = Some(layout);
                self.session_mode = Some(SandboxMode::Sandboxie);
                self.profile_flow.captured = true;
                self.profile_flow.cleaned_capture = Some(receipt);
                self.main_exe = installed
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into();
                self.profile_flow.installed_path = installed.to_string_lossy().into();
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
                self.review_needs_attention = !self.review.all_acknowledged();
                self.log.push(if reopened {
                    "Reopened the cleaned capture. Review the choices again before saving.".into()
                } else {
                    "Capture complete. The dedicated box was removed.".into()
                });
                self.mark_dirty();
                self.persist_now();
                if self.settings_dirty {
                    self.log.push("The capture is ready. This computer could not save the draft for reopening.".into());
                }
                self.start_update_comparison(ctx);
            }
            Ok(JobResult::Checked(prerequisites)) => {
                self.profile_flow.prerequisites = Some(prerequisites);
            }
            Ok(JobResult::MsiChecked(result)) => self.profile_flow.msi_ready = Some(result),
            #[cfg(windows)]
            Ok(JobResult::MsiStarted(layout)) => {
                self.layout = Some(layout);
                self.session_mode = Some(SandboxMode::WindowsSandbox);
                self.log.push(
                    "Windows Sandbox is starting. Wait for the clean note before opening setup."
                        .into(),
                );
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
            Ok(JobResult::Rebuilt(result)) => {
                let (approval, build) = *result;
                if let Err(error) = settings::save_build(
                    &self.settings_path,
                    &mut self.settings,
                    &approval,
                    build.clone(),
                ) {
                    self.profile_flow.error = Some(format!(
                        "The installer is ready. The build could not be remembered: {error}"
                    ));
                }
                if let Some(index) = active_queue {
                    self.profile_flow.queue[index].state = QueueState::Built(build);
                } else {
                    self.profile_flow.build = Some(build);
                    self.profile_flow.library_tab = LibraryTab::Results;
                }
            }
            #[cfg(all(windows, target_arch = "x86_64"))]
            Ok(JobResult::Relocated(approval, destination)) => {
                if let Err(error) = settings::relocate_approval(
                    &self.settings_path,
                    &mut self.settings,
                    &approval,
                    destination.clone(),
                ) {
                    self.profile_flow.error = Some(format!("The profile was copied to {}. Its new location could not be remembered: {error}", destination.display()));
                }
            }
            #[cfg(all(windows, target_arch = "x86_64"))]
            Ok(JobResult::UpdateCompared(details)) => {
                self.profile_flow.update_details = Some(details)
            }
            #[cfg(all(windows, target_arch = "x86_64"))]
            Ok(JobResult::BuildDetails(title, details)) => {
                self.profile_flow.build_details = Some((title, details))
            }
            Err(error) => {
                if let Some(index) = active_queue {
                    self.profile_flow.queue[index].state = QueueState::Failed(error);
                } else {
                    self.profile_flow.error = Some(error);
                }
            }
        }
        self.advance_queue(ctx);
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
        if busy {
            #[cfg(all(windows, target_arch = "x86_64"))]
            {
                self.profile_flow.show_candidate(ui);
                if self.profile_flow.cancellation.is_some() && ui.button("Cancel capture").clicked()
                {
                    self.profile_flow.cancel_capture();
                }
            }
            self.profile_flow.show_feedback(ui);
            return;
        }
        ui.scope(|ui| {
            if path_field(ui, "Setup file", &mut self.installer_path, FieldKind::File) {
                self.setup_changed();
            }
            if path_field(
                ui,
                "Installed app path (optional)",
                &mut self.profile_flow.installed_path,
                FieldKind::Typed,
            ) {
                self.profile_flow.reset_trust();
            }
            ui.horizontal_wrapped(|ui| {
                if ui.button("Setup options…").clicked() {
                    self.profile_flow.setup_options_open = true;
                }
                ui.label(if self.profile_flow.installed_path.trim().is_empty() {
                    "Choose an installed app after setup."
                } else {
                    "The app path is set."
                });
            });
            if ui.button("Check prerequisites").clicked() {
                self.check_prerequisites(ui.ctx());
            }
            let trust_label = if self.profile_flow.companions.is_empty() {
                "I trust this setup file and the installed app."
            } else {
                "I trust setup, the chosen companion files and the installed app."
            };
            ui.checkbox(&mut self.profile_flow.trusted, trust_label);
        });
        self.profile_flow.show_prerequisites(ui);
        self.profile_flow.show_feedback(ui);
        if self.profile_flow.setup_options_open {
            self.show_setup_options(ui.ctx());
        }
    }

    pub(super) fn capture_ready(&self) -> bool {
        !self.profile_flow.busy()
            && self.profile_flow.trusted
            && entered(&self.installer_path)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
            && self
                .profile_flow
                .prerequisites
                .as_ref()
                .is_some_and(|checks| checks.capture.is_ok())
    }

    pub(super) fn check_prerequisites(&mut self, ctx: &egui::Context) {
        let launcher = entered(&self.profile_flow.launcher_tools);
        let installer = entered(&self.profile_flow.installer_tools);
        self.profile_flow
            .start(ctx, "Checking prerequisites…", move |_| {
                Ok(JobResult::Checked(check_tools(&launcher, &installer)))
            });
    }

    fn show_setup_options(&mut self, ctx: &egui::Context) {
        let response = egui::Modal::new(egui::Id::new("verified-setup-options")).show(ctx, |ui| {
            ui.set_max_width(440.0);
            ui.heading("Setup options");
            egui::ScrollArea::vertical()
                .max_height(380.0)
                .show(ui, |ui| {
                    if self.uses_guided_msi() {
                        ui.label("Enter MSI options in the setup window inside Windows Sandbox.");
                    } else {
                        let heading = ui.label("Setup arguments, one per line");
                        if ui
                            .add(
                                egui::TextEdit::multiline(&mut self.profile_flow.setup_args)
                                    .desired_rows(3)
                                    .desired_width(f32::INFINITY),
                            )
                            .labelled_by(heading.id)
                            .changed()
                        {
                            self.profile_flow.reset_trust();
                        }
                    }
                    ui.label("Companion files stay beside setup inside the box.");
                    if ui.button("Choose companion files…").clicked() {
                        if let Some(paths) = rfd::FileDialog::new()
                            .set_title("Choose files used by setup")
                            .pick_files()
                        {
                            self.profile_flow.companions = paths;
                            self.profile_flow.reset_trust();
                        }
                    }
                    if !self.profile_flow.companions.is_empty() {
                        let paths = self
                            .profile_flow
                            .companions
                            .iter()
                            .map(|path| path.display().to_string())
                            .collect::<Vec<_>>()
                            .join("\n");
                        dense::view_text(
                            ui,
                            "View companion files",
                            "Companion files",
                            &paths,
                            true,
                        );
                        if ui.button("Clear companion files").clicked() {
                            self.profile_flow.companions.clear();
                            self.profile_flow.reset_trust();
                        }
                    }
                    if self.uses_guided_msi() {
                        return;
                    }
                    path_field(
                        ui,
                        "Capture folder",
                        &mut self.profile_flow.capture_parent,
                        FieldKind::Folder,
                    );
                    let launcher = path_field(
                        ui,
                        "Launcher Generator folder",
                        &mut self.profile_flow.launcher_tools,
                        FieldKind::Folder,
                    );
                    let installer = path_field(
                        ui,
                        "Installer maker folder",
                        &mut self.profile_flow.installer_tools,
                        FieldKind::Folder,
                    );
                    if launcher || installer {
                        self.profile_flow.prerequisites = None;
                    }
                });
            ui.button("Done").clicked()
        });
        if response.inner || response.should_close() {
            self.profile_flow.setup_options_open = false;
        }
    }

    pub(super) fn start_verified_capture(&mut self, ctx: &egui::Context) {
        if !self.capture_ready() {
            return;
        }
        #[cfg(all(windows, target_arch = "x86_64"))]
        {
            let cancellation = match CancellationToken::new() {
                Ok(token) => token,
                Err(error) => {
                    self.profile_flow.error = Some(error);
                    return;
                }
            };
            let input = CaptureInputs {
                setup: entered(&self.installer_path),
                installed: (!self.profile_flow.installed_path.trim().is_empty())
                    .then(|| entered(&self.profile_flow.installed_path)),
                parent: (!self.profile_flow.capture_parent.trim().is_empty())
                    .then(|| entered(&self.profile_flow.capture_parent)),
                args: self
                    .profile_flow
                    .setup_args
                    .lines()
                    .filter(|line| !line.is_empty())
                    .map(std::ffi::OsString::from)
                    .collect(),
                companions: self.profile_flow.companions.clone(),
            };
            let worker_token = cancellation.clone();
            if self.profile_flow.start_with_events(
                ctx,
                "Preparing the dedicated box…",
                move |tx, ctx| capture_job(input, tx, ctx, &worker_token),
            ) {
                self.profile_flow.cancellation = Some(cancellation);
                self.profile_flow.capture_setup = self.installer_path.trim().to_owned();
                self.profile_flow.cleaned_capture = None;
                self.mark_dirty();
            }
        }
        #[cfg(not(all(windows, target_arch = "x86_64")))]
        let _ = ctx;
    }

    pub(super) fn show_msi_safety(&mut self, ui: &mut egui::Ui) {
        match self.profile_flow.msi_ready.as_ref() {
            Some(Ok(())) => {
                ui.label("Windows Sandbox · Available to start");
            }
            Some(Err(error)) => {
                ui.label("Windows Sandbox · Needs attention");
                dense::view_text(
                    ui,
                    "View prerequisite details",
                    "Windows Sandbox",
                    error,
                    true,
                );
            }
            None => {
                ui.label("Check whether this computer can open Windows Sandbox.");
            }
        }
        if ui
            .add_enabled(
                !self.profile_flow.busy(),
                egui::Button::new("Check Windows Sandbox"),
            )
            .clicked()
        {
            self.profile_flow
                .start(ui.ctx(), "Checking Windows Sandbox…", move |_| {
                    #[cfg(windows)]
                    let result = crate::guided_msi_available();
                    #[cfg(not(windows))]
                    let result = Err("Guided MSI capture requires Windows Sandbox.".into());
                    Ok(JobResult::MsiChecked(result))
                });
        }
        self.profile_flow.show_feedback(ui);
    }

    pub(super) fn show_msi_capture(&mut self, ui: &mut egui::Ui) {
        let busy = self.profile_flow.busy();
        if self.layout.is_some() {
            ui.label("Wait for the clean note before opening the MSI in Windows Sandbox.");
            if ui.button("Check on the test box").clicked() {
                self.check_test_box();
            }
            return;
        }
        ui.add_enabled_ui(!busy, |ui| {
            if ui.button("Setup options…").clicked() {
                self.profile_flow.setup_options_open = true;
            }
            ui.checkbox(
                &mut self.profile_flow.trusted,
                "I trust this MSI and the selected companion files.",
            );
            let ready = self.profile_flow.trusted
                && self
                    .profile_flow
                    .msi_ready
                    .as_ref()
                    .is_some_and(Result::is_ok);
            if ui
                .add_enabled(ready, dense::primary("Start Windows Sandbox"))
                .clicked()
            {
                self.start_msi_capture(ui.ctx());
            }
        });
        self.profile_flow.show_feedback(ui);
        if self.profile_flow.setup_options_open && !busy {
            self.show_setup_options(ui.ctx());
        }
    }

    fn start_msi_capture(&mut self, ctx: &egui::Context) {
        #[cfg(windows)]
        {
            let setup = entered(&self.installer_path);
            let companions = self.profile_flow.companions.clone();
            let memory = self.sandbox_memory_mb;
            self.profile_flow
                .start(ctx, "Preparing Windows Sandbox…", move |_| {
                    crate::guided_msi_available()?;
                    let agent = std::env::current_exe()
                        .map_err(|error| error.to_string())?
                        .with_file_name("portableforge-agent.exe");
                    let stamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_err(|error| error.to_string())?
                        .as_nanos();
                    let id = format!("Guided-MSI-{}-{stamp}", std::process::id());
                    let layout = crate::prepare_guided_msi_capture(
                        &super::sessions_base(),
                        &id,
                        &setup,
                        &companions,
                        &agent,
                        memory,
                    )?;
                    crate::launch_guided_msi_capture(&layout)?;
                    Ok(JobResult::MsiStarted(layout))
                });
        }
        #[cfg(not(windows))]
        let _ = ctx;
    }

    pub(super) fn restore_cleaned_capture(&mut self, ctx: &egui::Context, capture: CleanedCapture) {
        self.enter_verified_capture();
        self.profile_flow.capture_setup = self.installer_path.trim().to_owned();
        self.profile_flow.cleaned_capture = Some(capture.clone());
        self.profile_flow
            .start(ctx, "Checking the saved capture…", move |_| {
                #[cfg(all(windows, target_arch = "x86_64"))]
                {
                    load_cleaned_capture(&capture.folder, Some(&capture.receipt_sha256))
                }
                #[cfg(not(all(windows, target_arch = "x86_64")))]
                {
                    let _ = capture;
                    Err(
                        "Opening a verified capture needs 64-bit Windows on an x64 computer."
                            .into(),
                    )
                }
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
            && (self.profile_flow.update_base.is_none()
                || self.profile_flow.update_details.is_some())
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
                self.settings.saved_profiles.iter().position(|profile| {
                    profile.folder == saved.folder
                        && profile.manifest_sha256 == saved.manifest_sha256
                })
            })
            .unwrap_or(0);
        self.profile_flow.profile_page = self.profile_flow.selected_profile;
        self.profile_flow.replay_setup = if from_capture {
            self.profile_flow.capture_setup.clone()
        } else {
            self.installer_path.clone()
        };
        self.profile_flow.replay_destination.clear();
        self.profile_flow.build = None;
        self.profile_flow.library_tab = LibraryTab::Profiles;
        self.profile_flow.profile_search.clear();
        self.profile_flow.result_page = 0;
        self.profile_flow.error = None;
    }

    pub(super) fn show_profile_replay(&mut self, ctx: &egui::Context) {
        self.show_header(ctx, "Saved profiles", None);
        let busy = self.profile_flow.busy();
        let request = self.replay_request();
        egui::TopBottomPanel::bottom("profile-replay-back").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.add_enabled(!busy, egui::Button::new("Back")).clicked() {
                    self.profile_flow.replay_open = false;
                }
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| match self.profile_flow.library_tab {
                        LibraryTab::Profiles => {
                            if ui
                                .add_enabled(
                                    !busy && request.is_some(),
                                    dense::primary("Build installer"),
                                )
                                .clicked()
                            {
                                if let Some(request) = request.clone() {
                                    self.start_rebuild(ctx, request, None);
                                }
                            }
                            if ui
                                .add_enabled(
                                    !busy && request.is_some(),
                                    egui::Button::new("Add to queue"),
                                )
                                .clicked()
                            {
                                if let Some(request) = request.clone() {
                                    self.enqueue(request);
                                }
                            }
                        }
                        LibraryTab::Queue => {
                            let waiting = self
                                .profile_flow
                                .queue
                                .iter()
                                .any(|job| matches!(job.state, QueueState::Waiting));
                            if self.profile_flow.queue_running {
                                if ui.button("Pause after this build").clicked() {
                                    self.profile_flow.queue_running = false;
                                }
                            } else if ui
                                .add_enabled(!busy && waiting, dense::primary("Build queued apps"))
                                .clicked()
                            {
                                self.profile_flow.queue_running = true;
                                self.advance_queue(ctx);
                            }
                        }
                        LibraryTab::Results => {
                            if ui
                                .add_enabled(!busy, egui::Button::new("Build another"))
                                .clicked()
                            {
                                self.profile_flow.library_tab = LibraryTab::Profiles;
                            }
                        }
                    },
                );
            });
        });
        egui::CentralPanel::default()
            .frame(dense::content_frame(ctx))
            .show(ctx, |ui| {
                texture::paint(ui);
                dense::heading(ui, "Build and verify", "");
                ui.add_enabled_ui(!busy, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for (tab, name) in [
                            (LibraryTab::Profiles, "Saved profiles"),
                            (LibraryTab::Queue, "Build queue"),
                            (LibraryTab::Results, "Build results"),
                        ] {
                            ui.selectable_value(&mut self.profile_flow.library_tab, tab, name);
                        }
                    });
                });
                match self.profile_flow.library_tab {
                    LibraryTab::Profiles => self.show_library_profiles(ui, busy),
                    LibraryTab::Queue => self.show_queue(ui),
                    LibraryTab::Results => self.show_build_results(ui),
                }
                self.profile_flow.show_feedback(ui);
            });
        if self.profile_flow.profile_actions_open && !busy {
            self.show_profile_actions(ctx);
        }
        self.show_build_dialogs(ctx);
    }

    fn begin_profile_update(&mut self, ctx: &egui::Context, approval: SavedProfileApproval) {
        self.profile_flow.update_base = Some(approval);
        self.profile_flow.update_details = None;
        self.profile_flow.profile_actions_open = false;
        self.profile_flow.replay_open = false;
        self.profile_flow.captured = false;
        self.profile_flow.saved = None;
        self.profile_flow.approval_error = None;
        self.profile_flow.cleaned_capture = None;
        self.profile_flow.reset_trust();
        self.profile_flow.installed_path.clear();
        self.profile_flow.capture_setup.clear();
        self.profile_flow.setup_args.clear();
        self.profile_flow.companions.clear();
        self.profile_flow.profile_destination.clear();
        self.installer_path.clear();
        self.app_name.clear();
        self.publisher.clear();
        self.homepage.clear();
        self.description.clear();
        self.main_exe.clear();
        self.package_version = "1.0.0.0".into();
        self.review = Default::default();
        self.review_page = 0;
        self.review_needs_attention = false;
        self.sharing = None;
        self.layout = None;
        self.session_mode = None;
        self.resume_offer = false;
        self.enter_verified_capture();
        self.mark_dirty();
        self.check_prerequisites(ctx);
    }

    fn start_update_comparison(&mut self, ctx: &egui::Context) {
        let (Some(approval), Some(layout)) = (self.profile_flow.update_base.clone(), &self.layout)
        else {
            return;
        };
        let session = layout.root.clone();
        let review = self.review.clone();
        self.profile_flow.start(
            ctx,
            "Comparing the update with the saved profile…",
            move |_| compare_update_job(&approval, &session, &review),
        );
    }

    pub(super) fn show_update_review(&mut self, ui: &mut egui::Ui) {
        let Some(approval) = &self.profile_flow.update_base else {
            return;
        };
        ui.label(format!("Update for {}", approval.label));
        if let Some(details) = &self.profile_flow.update_details {
            dense::view_text(
                ui,
                "View update comparison",
                "Saved profile and new capture",
                details,
                true,
            );
        } else if !self.profile_flow.busy() && ui.button("Compare with saved profile").clicked() {
            self.start_update_comparison(ui.ctx());
        }
        self.profile_flow.show_feedback(ui);
    }

    fn show_build_dialogs(&mut self, ctx: &egui::Context) {
        if let Some((title, details)) = &self.profile_flow.build_details {
            let response = egui::Modal::new(egui::Id::new("build-details")).show(ctx, |ui| {
                ui.set_max_width(500.0);
                ui.heading(*title);
                egui::ScrollArea::vertical()
                    .max_height(380.0)
                    .show(ui, |ui| {
                        ui.add(egui::Label::new(details).wrap().selectable(true));
                    });
                ui.button("Done").clicked()
            });
            if response.inner || response.should_close() {
                self.profile_flow.build_details = None;
            }
        }
        if let Some(build) = self.profile_flow.runtime_confirm.clone() {
            let response = egui::Modal::new(egui::Id::new("confirm-installer-test")).show(ctx, |ui| {
                ui.set_max_width(440.0);
                ui.heading("Test the built installer");
                ui.label("Complete installation in the fresh Sandboxie-Plus box. Leave any option to open the app unchecked. Change a setting during the first launch and check it after reopening, then close the app.");
                ui.horizontal(|ui| (ui.button("Cancel").clicked(), ui.add(dense::primary("Start test")).clicked())).inner
            });
            if response.inner.0 || response.should_close() {
                self.profile_flow.runtime_confirm = None;
            } else if response.inner.1 {
                self.profile_flow.runtime_confirm = None;
                self.start_runtime_test(ctx, build);
            }
        }
    }

    fn start_runtime_test(&mut self, ctx: &egui::Context, build: SavedBuild) {
        #[cfg(all(windows, target_arch = "x86_64"))]
        {
            match CancellationToken::new() {
                Ok(cancellation) => {
                    let worker_token = cancellation.clone();
                    if self.profile_flow.start(
                        ctx,
                        "Complete setup, then close each app launch when ready.",
                        move |_| runtime_job(&build, &worker_token),
                    ) {
                        self.profile_flow.cancellation = Some(cancellation);
                    }
                }
                Err(error) => self.profile_flow.error = Some(error),
            }
        }
        #[cfg(not(all(windows, target_arch = "x86_64")))]
        {
            let _ = (ctx, build);
            self.profile_flow.error = Some(
                "The dedicated installer test needs 64-bit Windows on an x64 computer.".into(),
            );
        }
    }

    fn show_library_profiles(&mut self, ui: &mut egui::Ui, busy: bool) {
        if self.settings.saved_profiles.is_empty() {
            ui.label("Capture, review and save a profile before rebuilding.");
            return;
        }
        ui.add_enabled_ui(!busy, |ui| {
            ui.horizontal(|ui| {
                let label = ui.label("Search saved profiles");
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.profile_flow.profile_search)
                            .desired_width(ui.available_width() - 10.0),
                    )
                    .labelled_by(label.id)
                    .changed()
                {
                    self.profile_flow.profile_page = 0;
                }
            });
            let query = self.profile_flow.profile_search.trim().to_lowercase();
            let matches = self
                .settings
                .saved_profiles
                .iter()
                .enumerate()
                .filter_map(|(index, profile)| profile_matches(profile, &query).then_some(index))
                .collect::<Vec<_>>();
            if matches.is_empty() {
                self.profile_flow.selected_profile = usize::MAX;
                ui.label("No saved profiles match your search.");
                return;
            }
            for index in dense::page(
                ui,
                &mut self.profile_flow.profile_page,
                matches.len(),
                1,
                "Profile",
            ) {
                if self.profile_flow.selected_profile != matches[index] {
                    self.profile_flow.build = None;
                    self.profile_flow.result_page = 0;
                }
                self.profile_flow.selected_profile = matches[index];
                let approval = &self.settings.saved_profiles[matches[index]];
                ui.horizontal_wrapped(|ui| {
                    let label = ui.label("Saved profile");
                    let version = &approval.details.version;
                    ui.label(if version.is_empty() {
                        approval.label.clone()
                    } else {
                        format!("{} · {}", approval.label, version)
                    })
                    .labelled_by(label.id);
                    if ui.button("Profile actions…").clicked() {
                        self.profile_flow.profile_actions_open = true;
                    }
                });
                if let Some(build) = approval.details.builds.first() {
                    ui.label(format!("Last build · {}", build.completed_utc));
                }
            }
            path_field(
                ui,
                "Matching setup file",
                &mut self.profile_flow.replay_setup,
                FieldKind::File,
            );
            if path_field(
                ui,
                "PortableApps.com Installer folder (3.9.19)",
                &mut self.profile_flow.installer_tools,
                FieldKind::Folder,
            ) {
                self.profile_flow.prerequisites = None;
            }
            path_field(
                ui,
                "New build folder",
                &mut self.profile_flow.replay_destination,
                FieldKind::NewFolder("PortableForge Installer"),
            );
        });
    }

    fn show_profile_actions(&mut self, ctx: &egui::Context) {
        let Some(approval) = self
            .settings
            .saved_profiles
            .get(self.profile_flow.selected_profile)
            .cloned()
        else {
            self.profile_flow.profile_actions_open = false;
            return;
        };
        let response = egui::Modal::new(egui::Id::new("saved-profile-actions")).show(ctx, |ui| {
            ui.set_max_width(440.0);
            ui.heading(&approval.label);
            egui::ScrollArea::vertical()
                .max_height(360.0)
                .show(ui, |ui| {
                    let path = approval.folder.display().to_string();
                    dense::view_text(ui, "View profile path", "Profile folder", &path, true);
                    if !approval.details.setup_sha256.is_empty() {
                        dense::view_text(
                            ui,
                            "View setup fingerprint",
                            "Setup fingerprint",
                            &approval.details.setup_sha256,
                            true,
                        );
                    }
                    if ui.button("Verify profile details").clicked() {
                        let selected = approval.clone();
                        self.profile_flow
                            .start(ctx, "Checking the saved profile…", move |_| {
                                inspect_job(&selected)
                            });
                        self.profile_flow.profile_actions_open = false;
                    }
                    if ui.button("Capture an update").clicked() {
                        self.begin_profile_update(ctx, approval.clone());
                    }
                    path_field(
                        ui,
                        "New profile location",
                        &mut self.profile_flow.relocate_destination,
                        FieldKind::NewFolder("PortableForge Profile"),
                    );
                    if ui
                        .add_enabled(
                            !self.profile_flow.relocate_destination.trim().is_empty(),
                            egui::Button::new("Relocate verified profile"),
                        )
                        .clicked()
                    {
                        let selected = approval.clone();
                        let destination = entered(&self.profile_flow.relocate_destination);
                        self.profile_flow.start(
                            ctx,
                            "Verifying and relocating your profile…",
                            move |_| relocate_job(selected, &destination),
                        );
                        self.profile_flow.profile_actions_open = false;
                    }
                });
            ui.button("Done").clicked()
        });
        if response.inner || response.should_close() {
            self.profile_flow.profile_actions_open = false;
        }
    }

    fn replay_request(&self) -> Option<ReplayRequest> {
        let setup = entered(&self.profile_flow.replay_setup);
        let tools = entered(&self.profile_flow.installer_tools);
        let destination = entered(&self.profile_flow.replay_destination);
        if [&setup, &tools, &destination]
            .iter()
            .any(|path| path.as_os_str().is_empty())
        {
            return None;
        }
        Some(ReplayRequest {
            setup,
            approval: self
                .settings
                .saved_profiles
                .get(self.profile_flow.selected_profile)?
                .clone(),
            tools,
            destination,
        })
    }

    fn enqueue(&mut self, request: ReplayRequest) {
        if self
            .profile_flow
            .queue
            .iter()
            .any(|item| item.request.destination == request.destination)
        {
            self.profile_flow.error =
                Some("Choose a new build folder for each queue entry.".into());
            return;
        }
        self.profile_flow.queue.push(QueuedBuild {
            request,
            state: QueueState::Waiting,
        });
        self.profile_flow.queue_page = self.profile_flow.queue.len() - 1;
        self.profile_flow.library_tab = LibraryTab::Queue;
        self.profile_flow.replay_destination.clear();
        self.profile_flow.error = None;
    }

    fn start_rebuild(
        &mut self,
        ctx: &egui::Context,
        request: ReplayRequest,
        queue_index: Option<usize>,
    ) {
        if self
            .profile_flow
            .start(ctx, "Building your installer…", move |_| {
                rebuild_job(request)
            })
        {
            self.profile_flow.active_queue = queue_index;
            if let Some(index) = queue_index {
                self.profile_flow.queue[index].state = QueueState::Building;
            }
        } else {
            self.profile_flow.queue_running = false;
        }
    }

    fn advance_queue(&mut self, ctx: &egui::Context) {
        if !self.profile_flow.queue_running || self.profile_flow.busy() {
            return;
        }
        if let Some(index) = self
            .profile_flow
            .queue
            .iter()
            .position(|item| matches!(item.state, QueueState::Waiting))
        {
            self.start_rebuild(
                ctx,
                self.profile_flow.queue[index].request.clone(),
                Some(index),
            );
        } else {
            self.profile_flow.queue_running = false;
        }
    }

    fn show_queue(&mut self, ui: &mut egui::Ui) {
        if self.profile_flow.queue.is_empty() {
            ui.label("Choose a saved profile and add it to the queue.");
            return;
        }
        for index in dense::page(
            ui,
            &mut self.profile_flow.queue_page,
            self.profile_flow.queue.len(),
            1,
            "Queued app",
        ) {
            let job = &self.profile_flow.queue[index];
            ui.heading(&job.request.approval.label);
            let destination = job.request.destination.display().to_string();
            dense::view_text(ui, "View build folder", "Build folder", &destination, true);
            match &job.state {
                QueueState::Waiting => {
                    ui.label("Waiting");
                }
                QueueState::Building => {
                    ui.label("Building");
                }
                #[cfg(all(windows, target_arch = "x86_64"))]
                QueueState::Built(build) => {
                    ui.label("Installer ready");
                    if ui.button("View this build").clicked() {
                        self.profile_flow.selected_profile = self
                            .settings
                            .saved_profiles
                            .iter()
                            .position(|profile| {
                                profile.folder == job.request.approval.folder
                                    && profile.manifest_sha256
                                        == job.request.approval.manifest_sha256
                            })
                            .unwrap_or(usize::MAX);
                        self.profile_flow.build = Some(build.clone());
                        self.profile_flow.library_tab = LibraryTab::Results;
                    }
                }
                QueueState::Failed(error) => {
                    ui.label("Build failed");
                    dense::view_text(ui, "View build log", "Build problem", error, true);
                    let work = job.request.destination.join("work");
                    if work.is_dir() && ui.button("Open build working folder").clicked() {
                        if let Err(error) = open_folder(&work) {
                            self.profile_flow.error = Some(error);
                        }
                    }
                    if ui
                        .add_enabled(
                            !self.profile_flow.busy(),
                            egui::Button::new("Retry in a new folder…"),
                        )
                        .clicked()
                    {
                        if let Some(parent) = rfd::FileDialog::new()
                            .set_title("Choose a parent for the retry")
                            .pick_folder()
                        {
                            match fresh_retry_path(&parent) {
                                Ok(destination) => {
                                    let mut retry = job.request.clone();
                                    retry.destination = destination;
                                    self.enqueue(retry);
                                }
                                Err(error) => self.profile_flow.error = Some(error),
                            }
                        }
                    }
                }
            }
        }
    }

    fn show_build_results(&mut self, ui: &mut egui::Ui) {
        let busy = self.profile_flow.busy();
        let history = self
            .settings
            .saved_profiles
            .get(self.profile_flow.selected_profile)
            .map(|profile| profile.details.builds.as_slice())
            .unwrap_or_default();
        let mut history_index = None;
        if !history.is_empty() {
            for index in dense::page(
                ui,
                &mut self.profile_flow.result_page,
                history.len(),
                1,
                "Saved build",
            ) {
                history_index = Some(index);
            }
        }
        let Some(build) = self
            .profile_flow
            .build
            .clone()
            .or_else(|| history_index.map(|index| history[index].clone()))
        else {
            ui.label("Build a saved profile to see its installer and verification details.");
            return;
        };
        if let Some(index) = history_index {
            if history.len() > 1 && ui.button("View selected saved build").clicked() {
                self.profile_flow.build = Some(history[index].clone());
            }
        }
        ui.heading("Your installer is ready");
        ui.label(&build.completed_utc);
        let path = build.artifact.display().to_string();
        if dense::path_needs_view(ui, &path, ui.available_width()) {
            dense::view_text(ui, "View path", "Installer path", &path, true);
        } else {
            ui.label(path);
        }
        let previous = history
            .iter()
            .find(|other| other.directory != build.directory)
            .cloned();
        ui.add_enabled_ui(!busy, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button("Open the installer folder").clicked() {
                    if let Err(error) = open_folder(&build.directory) {
                        self.profile_flow.error = Some(error);
                    }
                }
                if ui.button("View build details").clicked() {
                    let selected = build.clone();
                    self.profile_flow.start(
                        ui.ctx(),
                        "Verifying the build receipt…",
                        move |_| build_details_job(&selected),
                    );
                }
                if ui
                    .add_enabled(previous.is_some(), egui::Button::new("Compare builds"))
                    .clicked()
                {
                    if let Some(previous) = previous {
                        let selected = build.clone();
                        self.profile_flow.start(
                            ui.ctx(),
                            "Checking both build receipts…",
                            move |_| compare_builds_job(&selected, &previous),
                        );
                    }
                }
                if ui.button("Test installer").clicked() {
                    self.profile_flow.runtime_confirm = Some(build.clone());
                }
            });
        });
        #[cfg(all(windows, target_arch = "x86_64"))]
        if self.profile_flow.cancellation.is_some() && ui.button("Cancel test").clicked() {
            self.profile_flow.cancel_capture();
        }
    }
}

fn check_tools(launcher: &Path, installer: &Path) -> Prerequisites {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        use portableforge_core::wizard_package::{
            verify_installer_toolchain, verify_launcher_toolchain,
        };
        Prerequisites {
            capture: portableforge_core::sbie_verified::preflight(),
            launcher: (!launcher.as_os_str().is_empty())
                .then(|| verify_launcher_toolchain(launcher)),
            installer: (!installer.as_os_str().is_empty())
                .then(|| verify_installer_toolchain(installer)),
        }
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        let _ = (launcher, installer);
        Prerequisites {
            capture: Err("Verified quick capture needs 64-bit Windows on an x64 computer.".into()),
            launcher: None,
            installer: None,
        }
    }
}

#[cfg(all(windows, target_arch = "x86_64"))]
struct CaptureInputs {
    setup: PathBuf,
    installed: Option<PathBuf>,
    parent: Option<PathBuf>,
    args: Vec<std::ffi::OsString>,
    companions: Vec<PathBuf>,
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn unique_folder(parent: &Path, prefix: &str) -> Result<PathBuf, String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    Ok(parent.join(format!("{prefix}-{stamp}")))
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn capture_job(
    inputs: CaptureInputs,
    tx: &mpsc::Sender<JobEvent>,
    ctx: &egui::Context,
    cancellation: &CancellationToken,
) -> Result<JobResult, String> {
    use portableforge_core::sbie_verified::{
        capture_with_options, preflight, CaptureRequest, CaptureStage,
    };
    preflight()?;
    let parent = match inputs.parent {
        Some(parent) => parent,
        None => {
            let default_parent = sessions_base();
            std::fs::create_dir_all(&default_parent).map_err(|error| error.to_string())?;
            default_parent
        }
    };
    let output = unique_folder(&parent, "Verified")?;
    let root = capture_with_options(
        CaptureRequest {
            setup: &inputs.setup,
            installed: inputs.installed.as_deref(),
            output: &output,
            setup_args: &inputs.args,
            companions: &inputs.companions,
            timeout: std::time::Duration::from_secs(600),
        },
        |state| {
            let _ = tx.send(JobEvent::Stage(match state {
                CaptureStage::Preparing => "Preparing the dedicated box…",
                CaptureStage::Installing => "Complete setup in its window.",
                CaptureStage::SelectingApplication => "Choose the app installed by setup.",
                CaptureStage::Exercising => "Try the app, then close it.",
                CaptureStage::CleaningUp => "Checking capture and removing the dedicated box…",
            }));
            ctx.request_repaint();
        },
        |paths| {
            let (reply, choice) = mpsc::channel();
            tx.send(JobEvent::Choose(CandidateChoice {
                paths: paths.to_vec(),
                page: 0,
                reply,
            }))
            .map_err(|_| "The wizard closed before choosing an installed app.")?;
            ctx.request_repaint();
            if cancellation.is_cancelled() {
                return Err("App selection was cancelled.".into());
            }
            choice
                .recv()
                .map_err(|_| "App selection was cancelled.".into())
        },
        cancellation,
    )?;
    load_cleaned_capture(&root, None)
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn load_cleaned_capture(root: &Path, expected: Option<&str>) -> Result<JobResult, String> {
    let (installed, receipt_sha256) =
        portableforge_core::sbie_verified::verify_capture_application(root, expected)?;
    let installed = installed.ok_or(
        "This older capture receipt has no selected app. Start a new capture to review the app.",
    )?;
    let layout = crate::session_layout_for(root);
    let outcome = portableforge_core::orchestrate::read_capture(&layout)?;
    let receipt = CleanedCapture {
        folder: root.to_path_buf(),
        receipt_sha256,
    };
    Ok(JobResult::Captured(Box::new(CaptureComplete {
        layout,
        outcome,
        installed,
        receipt,
        reopened: expected.is_some(),
    })))
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
            details: settings::ProfileDetails {
                version: meta.display_version.clone(),
                setup_sha256: result.setup_sha256,
                builds: Vec::new(),
            },
        }))
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        let _ = (session, meta, review, tools, destination);
        Err("Saving this capture needs 64-bit Windows on an x64 computer.".into())
    }
}

fn rebuild_job(request: ReplayRequest) -> Result<JobResult, String> {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        use portableforge_core::{compile, wizard_package};
        let ReplayRequest {
            setup,
            approval,
            tools,
            destination,
        } = request;
        let result = wizard_package::build_reviewed_profile(
            &setup,
            &approval.folder,
            &approval.manifest_sha256,
            &tools,
            &destination,
        )?;
        let directory = result
            .artifact
            .parent()
            .ok_or("The installer folder is missing.")?;
        let verified = compile::verify_build_result(directory)?;
        if verified.sha256 != result.sha256
            || verified.app_id.as_deref() != Some(result.app_id.as_str())
            || verified
                .profile
                .as_ref()
                .map(|proof| proof.manifest_sha256.as_str())
                != Some(approval.manifest_sha256.as_str())
        {
            return Err("The installer receipt changed before it could be recorded.".into());
        }
        let build = SavedBuild {
            directory: directory.to_path_buf(),
            artifact: verified.artifact,
            manifest_sha256: verified.manifest_sha256,
            artifact_sha256: result.sha256,
            app_id: result.app_id,
            completed_utc: portableforge_core::timeutil::now_iso(),
        };
        Ok(JobResult::Rebuilt(Box::new((approval, build))))
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        let _ = (&request.setup, &request.tools);
        Err("Making this installer needs 64-bit Windows on an x64 computer.".into())
    }
}

fn profile_matches(approval: &SavedProfileApproval, query: &str) -> bool {
    [
        approval.label.as_str(),
        &approval.details.version,
        &approval.details.setup_sha256,
    ]
    .iter()
    .any(|text| text.to_lowercase().contains(query))
}

fn fresh_retry_path(parent: &Path) -> Result<PathBuf, String> {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        unique_folder(parent, "PortableForge-Retry")
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        let _ = parent;
        Err("Building needs 64-bit Windows on an x64 computer.".into())
    }
}

fn inspect_job(approval: &SavedProfileApproval) -> Result<JobResult, String> {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        let result =
            portableforge_core::profile::inspect(&approval.folder, &approval.manifest_sha256)?;
        let details = serde_json::to_string_pretty(&result).map_err(|error| error.to_string())?;
        Ok(JobResult::BuildDetails("Verified profile details", details))
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        let _ = approval;
        Err("Verification needs 64-bit Windows on an x64 computer.".into())
    }
}

fn relocate_job(approval: SavedProfileApproval, destination: &Path) -> Result<JobResult, String> {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        let result = portableforge_core::profile::relocate(
            &approval.folder,
            &approval.manifest_sha256,
            destination,
        )?;
        let folder = result
            .manifest
            .parent()
            .ok_or("The new profile folder is missing.")?
            .to_path_buf();
        Ok(JobResult::Relocated(approval, folder))
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        let _ = (approval, destination);
        Err("Moving a profile needs 64-bit Windows on an x64 computer.".into())
    }
}

fn compare_update_job(
    approval: &SavedProfileApproval,
    session: &Path,
    review: &portableforge_core::review::Review,
) -> Result<JobResult, String> {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        let comparison = portableforge_core::profile::compare_capture(
            &approval.folder,
            &approval.manifest_sha256,
            session,
            review,
        )?;
        let mut report = format!(
            "Setup file · {}\nSetup arguments · {}\nCompanion files · {}\nApp files changed · {}\n\nSaved choices\n{}\n\nNew findings\n{}",
            change(comparison.previous_setup_sha256 == comparison.current_setup_sha256),
            change(!comparison.setup_arguments_changed),
            change(!comparison.setup_companions_changed),
            comparison.payload_changes.len(),
            comparison.previous_review.summary(),
            comparison.current_review.summary(),
        );
        let review_changes =
            review_changes(&comparison.previous_review, &comparison.current_review);
        report.push_str(&format!(
            "\n\nReview findings changed · {}",
            review_changes.len()
        ));
        for line in review_changes {
            report.push('\n');
            report.push_str(&line);
        }
        report.push_str("\n\nApp file changes");
        for item in comparison.payload_changes {
            let status = match (item.previous_sha256, item.current_sha256) {
                (None, Some(_)) => "Added",
                (Some(_), None) => "Removed",
                _ => "Changed",
            };
            report.push_str(&format!("\n{status} · {}", item.relative_path));
        }
        Ok(JobResult::UpdateCompared(report))
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        let _ = (approval, session, review);
        Err("Comparing an update needs 64-bit Windows on an x64 computer.".into())
    }
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn review_changes(
    previous: &portableforge_core::review::Review,
    current: &portableforge_core::review::Review,
) -> Vec<String> {
    use portableforge_core::classify::Finding;
    use std::collections::BTreeMap;
    let mut rows = BTreeMap::<_, (Vec<_>, Vec<_>)>::new();
    for (before, review) in [(true, previous), (false, current)] {
        for item in &review.items {
            let key = match &item.classification.finding {
                Finding::DirectoriesMove { host_pattern, .. } => {
                    ("Settings folder", host_pattern.clone())
                }
                Finding::FilesMove {
                    host_pattern,
                    data_target,
                } => ("Settings file", format!("{host_pattern}\\{data_target}")),
                Finding::RegistryKey { key } => ("App setting", key.clone()),
                Finding::AssociationCandidate { key } => ("File association", key.clone()),
                Finding::UninstallMetadata { key } => ("Uninstall entry", key.clone()),
                Finding::AppFile { path } => ("App file", path.clone()),
                Finding::FileWriteRewrite { path, .. } => ("File path update", path.clone()),
                Finding::RedFlag { kind, detail } => ("Warning", format!("{kind} · {detail}")),
            };
            let (old, new) = rows.entry(key).or_default();
            if before {
                old.push(&item.classification);
            } else {
                new.push(&item.classification);
            }
        }
    }
    rows.into_iter()
        .filter_map(|((label, location), (old, new))| {
            let status = if old.is_empty() {
                "Added"
            } else if new.is_empty() {
                "Removed"
            } else if old != new {
                "Changed"
            } else {
                return None;
            };
            Some(format!("{status} · {label} · {location}"))
        })
        .collect()
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn change(same: bool) -> &'static str {
    if same {
        "Same"
    } else {
        "Changed"
    }
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn check_build_identity(
    approval: &SavedBuild,
    verified: &portableforge_core::compile::VerifiedBuild,
) -> Result<(), String> {
    if verified.manifest_sha256 != approval.manifest_sha256
        || verified.sha256 != approval.artifact_sha256
        || verified.app_id.as_deref() != Some(approval.app_id.as_str())
        || verified.artifact != approval.artifact
    {
        return Err("This installer or its build receipt changed since the build.".into());
    }
    Ok(())
}

fn build_details_job(build: &SavedBuild) -> Result<JobResult, String> {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        let result = portableforge_core::compile::verify_build_result(&build.directory)?;
        check_build_identity(build, &result)?;
        let details = serde_json::to_string_pretty(&result).map_err(|error| error.to_string())?;
        Ok(JobResult::BuildDetails("Verified build details", details))
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        let _ = build;
        Err("Reading build details needs 64-bit Windows on an x64 computer.".into())
    }
}

fn compare_builds_job(left: &SavedBuild, right: &SavedBuild) -> Result<JobResult, String> {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        let comparison =
            portableforge_core::compile::compare_build_results(&left.directory, &right.directory)?;
        check_build_identity(left, &comparison.left)?;
        check_build_identity(right, &comparison.right)?;
        let details = [
            ("Setup", comparison.same_setup),
            ("Package", comparison.same_package),
            ("App files", comparison.same_payload),
            ("Settings rules", comparison.same_configuration),
            ("Package icons", comparison.same_icon_set),
            ("Installer icon", Some(comparison.same_primary_icon)),
            ("Installer maker files", comparison.same_installer_support),
            (
                "Staged file times before normalization",
                comparison.same_staged_file_times,
            ),
            (
                "Compiler programs",
                Some(comparison.same_pinned_executables),
            ),
            (
                "Compiler exit codes",
                Some(comparison.same_compiler_exit_codes),
            ),
            ("Installer file", Some(comparison.same_artifact)),
        ]
        .iter()
        .map(|(label, same)| {
            format!(
                "{label} · {}",
                same.map_or("Unavailable in older receipts", change)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
        let details = format!(
            "{details}\n\nFirst installer SHA-256\n{}\n\nSecond installer SHA-256\n{}",
            comparison.left.sha256, comparison.right.sha256
        );
        Ok(JobResult::BuildDetails("Build comparison", details))
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        let _ = (left, right);
        Err("Comparing builds needs 64-bit Windows on an x64 computer.".into())
    }
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn runtime_job(build: &SavedBuild, cancellation: &CancellationToken) -> Result<JobResult, String> {
    use portableforge_core::sbie_verified::{
        preflight, test_built_paf, RuntimeDataStatus, RuntimeRequest,
    };
    preflight()?;
    let verified = portableforge_core::compile::verify_build_result(&build.directory)?;
    check_build_identity(build, &verified)?;
    let parent = sessions_base();
    std::fs::create_dir_all(&parent).map_err(|error| error.to_string())?;
    let output = unique_folder(&parent, "Installer-Test")?;
    let report = test_built_paf(
        RuntimeRequest {
            installer: &verified.artifact,
            installer_sha256: &build.artifact_sha256,
            app_id: &build.app_id,
            output: &output,
            timeout: std::time::Duration::from_secs(600),
        },
        cancellation,
    )?;
    let state = match report.data_status {
        RuntimeDataStatus::NoDataFilesChanged => {
            "No portable data files changed during the first launch."
        }
        RuntimeDataStatus::ObservedChangesRetained => {
            "The observed portable data changes were still present after reopening."
        }
        RuntimeDataStatus::ObservedChangesDifferAfterReopen => {
            "Some observed portable data changes differed after reopening."
        }
    };
    let details = format!(
        "Installer finished with exit code {}.\nApp launches finished with exit codes {} and {}.\nDedicated box removed · {}\n\n{state}\nNew or changed data files · {}\nDeleted data files · {}\nUnchanged on reopen · {}\n\nTest folder\n{}",
        report.install_exit,
        report.launch_exits[0],
        report.launch_exits[1],
        if report.cleanup_verified { "Yes" } else { "Unconfirmed" },
        report.data_files_created_or_changed,
        report.data_files_deleted,
        report.data_files_unchanged_on_reopen,
        output.display(),
    );
    Ok(JobResult::BuildDetails("Installer test result", details))
}
