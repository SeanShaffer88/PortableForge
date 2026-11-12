//! The guided wizard (egui, dark only).
//!
//! The GUI renders state and collects intent only — every word it shows
//! and every decision it offers comes from `portableforge_core::guide`
//! and the other core modules. No business rules live here.

use eframe::egui;
use portableforge_core::guide::folder_flow as folder_copy;
use portableforge_core::guide::{self, SharingAnswer, SHARING_ANSWERS, STEPS};
use portableforge_core::orchestrate::{
    evaluate_preflight, CapturePoint, PreflightReport, SessionLayout,
};
use portableforge_core::review::Review;
use portableforge_core::session::Phase;
use portableforge_core::settings::{self, WizardDraft, WizardSettings};

mod artwork_ui;
mod dense;
mod folder_flow;
mod profiles;
mod shell;
mod theme;

pub struct WizardApp {
    profile_flow: profiles::ProfileFlow,
    folder_flow: folder_flow::Flow,
    artwork: artwork_ui::Editor,
    theme_state: theme::State,
    shell: shell::State,
    step: usize,
    show_details: bool,
    detail_page: usize,
    review_page: usize,
    review_needs_attention: bool,
    msi_review_ready: bool,
    safety_page: usize,
    recent_page: usize,
    applied_layout_zoom: f32,
    installer_path: String,
    app_name: String,
    description: String,
    publisher: String,
    homepage: String,
    main_exe: String,
    package_version: String,
    online_url: String,
    online_sha: String,
    sharing: Option<SharingAnswer>,
    preflight: Option<PreflightReport>,
    /// The collected machine facts behind the safety report, kept
    /// so the "use it anyway" choice can re-judge them on the spot.
    preflight_facts: Option<portableforge_core::orchestrate::PreflightFacts>,
    /// Set by Finish on the last step: the window closes (the run
    /// is complete and its draft already cleared).
    finished: bool,
    /// The current session's way of testing, once one is started.
    session_mode: Option<portableforge_core::wsb::SandboxMode>,
    /// The person's word that this computer can run the safe test
    /// box, whatever the checks concluded (persisted in the draft).
    force_sandbox: bool,
    review: Review,
    layout: Option<SessionLayout>,
    build_summary: Option<String>,
    built_dir: Option<std::path::PathBuf>,
    icon_path: String,
    /// Test-box choices for this run (safe defaults: 4 GB, no
    /// internet, no clipboard sharing).
    sandbox_memory_mb: u32,
    sandbox_networking: bool,
    sandbox_clipboard: bool,
    /// The leak judge's verdict, once the test step has read the
    /// two notes from the leak-test packet.
    leak_result: Option<String>,
    log: Vec<String>,
    // The wizard's own memory (recent files, text size, the
    // unfinished-run draft) and the bookkeeping for saving it.
    settings: WizardSettings,
    settings_path: std::path::PathBuf,
    settings_dirty: bool,
    last_save: std::time::Instant,
    resume_offer: bool,
    hints_applied: bool,
}

impl Default for WizardApp {
    fn default() -> Self {
        let settings_path = test_state_root()
            .map(|root| root.join("wizard-settings.json"))
            .unwrap_or_else(settings::settings_path);
        let settings = settings::load(&settings_path);
        let resume_offer = settings
            .draft
            .as_ref()
            .map(|d| d.has_progress())
            .unwrap_or(false);
        Self {
            profile_flow: profiles::ProfileFlow::default(),
            folder_flow: folder_flow::Flow::default(),
            artwork: artwork_ui::Editor::default(),
            theme_state: theme::State::default(),
            shell: shell::State::default(),
            step: 0,
            show_details: false,
            detail_page: 0,
            review_page: 0,
            review_needs_attention: false,
            msi_review_ready: false,
            safety_page: 0,
            recent_page: 0,
            applied_layout_zoom: 0.0,
            installer_path: String::new(),
            app_name: String::new(),
            description: String::new(),
            publisher: String::new(),
            homepage: String::new(),
            main_exe: String::new(),
            package_version: "1.0.0.0".to_string(),
            online_url: String::new(),
            online_sha: String::new(),
            sharing: None,
            preflight: None,
            preflight_facts: None,
            force_sandbox: false,
            finished: false,
            session_mode: None,
            review: Review::default(),
            layout: None,
            build_summary: None,
            built_dir: None,
            icon_path: String::new(),
            sandbox_memory_mb: 4096,
            sandbox_networking: false,
            sandbox_clipboard: false,
            leak_result: None,
            log: vec!["Welcome to PortableForge.".to_string()],
            settings,
            settings_path,
            settings_dirty: false,
            last_save: std::time::Instant::now(),
            resume_offer,
            hints_applied: false,
        }
    }
}

/// Where the file chooser should open: the folder of the path the
/// person already typed, when there is one — never somewhere random.
fn picker_start_dir(current: &str) -> Option<std::path::PathBuf> {
    let p = std::path::Path::new(current.trim());
    if p.as_os_str().is_empty() {
        return None;
    }
    if p.is_dir() {
        return Some(p.to_path_buf());
    }
    p.parent()
        .filter(|parent| parent.is_dir())
        .map(|parent| parent.to_path_buf())
}

/// A file size in plain words: `812 KB`, `14.2 MB`, `1.3 GB`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SetupKind {
    Exe,
    Msi,
}

impl SetupKind {
    fn from_path(path: &std::path::Path) -> Option<Self> {
        let extension = path.extension()?;
        if extension.eq_ignore_ascii_case("exe") {
            Some(Self::Exe)
        } else if extension.eq_ignore_ascii_case("msi") {
            Some(Self::Msi)
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SetupInput {
    Empty,
    Missing,
    Folder,
    Unsupported,
    Ready { kind: SetupKind, size: u64 },
}

impl SetupInput {
    fn problem(self) -> Option<&'static str> {
        match self {
            Self::Empty => Some(guide::SETUP_REQUIRED),
            Self::Missing => Some(guide::SETUP_MISSING),
            Self::Folder => Some(guide::SETUP_FOLDER),
            Self::Unsupported => Some(guide::SETUP_UNSUPPORTED),
            Self::Ready { .. } => None,
        }
    }

    fn allows_verified_picker(self) -> bool {
        matches!(
            self,
            Self::Empty
                | Self::Ready {
                    kind: SetupKind::Exe,
                    ..
                }
        )
    }
}

fn setup_input(input: &str) -> SetupInput {
    let input = input.trim();
    if input.is_empty() {
        return SetupInput::Empty;
    }
    let path = std::path::Path::new(input);
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => SetupInput::Folder,
        Ok(metadata) if metadata.is_file() => {
            SetupKind::from_path(path).map_or(SetupInput::Unsupported, |kind| SetupInput::Ready {
                kind,
                size: metadata.len(),
            })
        }
        Ok(_) => SetupInput::Unsupported,
        Err(_) => SetupInput::Missing,
    }
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1_000;
    const MB: u64 = 1_000_000;
    const GB: u64 = 1_000_000_000;
    if bytes < KB {
        format!("{bytes} bytes")
    } else if bytes < MB {
        format!("{} KB", bytes / KB)
    } else if bytes < GB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    }
}

/// Open a folder in the computer's file manager. Best effort — the
/// wizard always prints the path too, so a failure is never a trap.
fn open_folder(path: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let program = "explorer";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    let program = "xdg-open";
    std::process::Command::new(program)
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("We could not open {}: {e}", path.display()))
}

/// Where session folders live: a `PortableForge Sessions` folder in
/// the person's home folder (current directory as a last resort).
fn sessions_base() -> std::path::PathBuf {
    if let Some(root) = test_state_root() {
        return root.join("sessions");
    }
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home).join("PortableForge Sessions")
}

fn test_state_root() -> Option<std::path::PathBuf> {
    #[cfg(debug_assertions)]
    {
        std::env::var_os("PORTABLEFORGE_TEST_STATE_ROOT")
            .map(std::path::PathBuf::from)
            .filter(|root| root.is_absolute())
    }
    #[cfg(not(debug_assertions))]
    {
        None
    }
}

fn card(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .inner_margin(egui::Margin::same(8))
}

impl WizardApp {
    fn setup_changed(&mut self) {
        self.profile_flow.detach_capture();
        self.artwork = artwork_ui::Editor::default();
        self.layout = None;
        self.session_mode = None;
        self.review = Review::default();
        self.review_page = 0;
        self.review_needs_attention = false;
        self.msi_review_ready = false;
        self.sharing = None;
        self.hints_applied = false;
        self.main_exe.clear();
        self.app_name.clear();
        self.publisher.clear();
        self.description.clear();
        self.homepage.clear();
        self.package_version = "1.0.0.0".into();
        self.mark_dirty();
    }

    fn uses_guided_msi(&self) -> bool {
        !self.profile_flow.selected_capture
            && SetupKind::from_path(std::path::Path::new(self.installer_path.trim()))
                == Some(SetupKind::Msi)
    }

    /// Make the session folder, copy the setup file and the helper
    /// into it, ask the helper for the clean note, and start the test
    /// box (on Windows). Safe to press again — an existing session is
    /// kept and only the box start is retried.
    fn start_test_box(&mut self) {
        if self.use_verified_capture() || self.uses_guided_msi() {
            return;
        }
        if self.layout.is_none() {
            if let Some(problem) = setup_input(&self.installer_path).problem() {
                self.log.push(problem.into());
                return;
            }
            let stem = std::path::Path::new(self.installer_path.trim())
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "run".to_string());
            let stem_clean: String = stem
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                        c
                    } else {
                        '-'
                    }
                })
                .collect();
            // Collision-proof session ids (item 77): the setup file's
            // name plus the UTC second this run started — starting
            // the same conversion twice never lands in one folder.
            let id = format!(
                "{}-{}",
                if stem_clean.is_empty() {
                    "run".to_string()
                } else {
                    stem_clean
                },
                portableforge_core::timeutil::now_compact()
            );
            let mode = self
                .preflight
                .as_ref()
                .map(|r| r.best_mode)
                .unwrap_or(portableforge_core::wsb::SandboxMode::WindowsSandbox);
            let session = portableforge_core::Session::new(id, mode);
            self.session_mode = Some(mode);
            let agent = std::env::current_exe().ok().and_then(|exe| {
                let p = exe.with_file_name("portableforge-agent.exe");
                if p.is_file() {
                    Some(p)
                } else {
                    None
                }
            });
            match portableforge_core::orchestrate::prepare_session(
                &sessions_base(),
                &session,
                agent.as_deref(),
                self.sandbox_networking,
                self.sandbox_memory_mb,
                self.sandbox_clipboard,
            ) {
                Ok(layout) => {
                    // The setup file goes into the read-only input folder.
                    let src = std::path::Path::new(self.installer_path.trim());
                    if let Some(name) = src.file_name() {
                        let _ = std::fs::copy(src, layout.input.join(name));
                    }
                    let _ = portableforge_core::orchestrate::request_snapshot(
                        &layout,
                        CapturePoint::Before,
                    );
                    self.log.push(format!(
                        "Your work folder is ready: {}",
                        layout.root.display()
                    ));
                    self.hints_applied = false;
                    self.layout = Some(layout);
                }
                Err(e) => {
                    self.log.push(e);
                    return;
                }
            }
        }
        if let Some(layout) = &self.layout {
            match crate::launch_sandbox(layout) {
                Ok(()) => self.log.push("The test box is starting.".to_string()),
                Err(e) => self.log.push(e),
            }
        }
    }

    /// The whole run as one plain text file: what was picked, what
    /// the review decided, what was built, what the leak test said,
    /// and the window's own log lines in order.
    fn run_log_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "PortableForge run log — version {}\n",
            env!("CARGO_PKG_VERSION")
        ));
        out.push_str(&format!(
            "Written (UTC): {}\n",
            portableforge_core::timeutil::now_iso()
        ));
        out.push_str(&format!("Setup file: {}\n", self.installer_path.trim()));
        if let Some(layout) = &self.layout {
            out.push_str(&format!("Work folder: {}\n", layout.root.display()));
        }
        out.push_str(&format!("\nReview: {}\n", self.review.summary()));
        if let Some(text) = &self.build_summary {
            out.push_str(&format!("\nBuild: {text}\n"));
        }
        if let Some(text) = &self.leak_result {
            out.push_str(&format!("\nLeak test:\n{text}\n"));
        }
        out.push_str("\nWhat happened, in order:\n");
        for line in &self.log {
            out.push_str(&format!("- {line}\n"));
        }
        out
    }

    fn export_run_log(&mut self) {
        let text = self.run_log_text();
        let picked = rfd::FileDialog::new()
            .set_title("Save the run log")
            .set_file_name("portableforge-run-log.txt")
            .add_filter("Text file", &["txt"])
            .save_file();
        if let Some(path) = picked {
            match std::fs::write(&path, text) {
                Ok(()) => self
                    .log
                    .push(format!("Run log saved to {}.", path.display())),
                Err(e) => self
                    .log
                    .push(format!("The run log could not be saved: {e}")),
            }
        }
    }

    /// Judge the leak test: read the packet's two notes from this
    /// run's exchange folder and say, in plain words, whether the
    /// finished copy stayed inside its own folder.
    fn check_leak_result(&mut self) {
        let Some(layout) = &self.layout else {
            self.log
                .push("Start the test box first — there is no run yet.".to_string());
            return;
        };
        let read = |name: &str| -> Option<portableforge_core::snapshot::Snapshot> {
            let text = std::fs::read_to_string(layout.exchange.join(name)).ok()?;
            serde_json::from_str(&text).ok()
        };
        let (Some(before), Some(after)) = (read("leak-before.json"), read("leak-after.json"))
        else {
            self.leak_result = Some(
                "No leak-test notes yet. The leak-test packet (portableforge leak-packet) makes two notes in the test box — leak-before.json before the finished copy runs, leak-after.json after it closes — and they belong in this run's exchange folder."
                    .to_string(),
            );
            return;
        };
        let d = portableforge_core::diff::diff_snapshots(&before, &after);
        let app_id = self
            .built_dir
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Package".to_string());
        let box_root = format!(r"C:\Users\WDAGUtilityAccount\Desktop\{app_id}");
        let report = portableforge_core::leaktest::analyse_leaks(&box_root, &before, &after, &d);
        let mut text = report.verdict();
        for (label, leaks) in report.grouped() {
            text.push_str(&format!("\n{label} ({})", leaks.len()));
            for leak in leaks {
                text.push_str(&format!("\n- {}", leak.plain));
                text.push_str(&format!("\n  Fix: {}", leak.fix_line()));
            }
        }
        self.leak_result = Some(text);
    }

    /// Look at the helper's state; when the capture has finished, read
    /// its notes into the review the "What it left behind" step shows.
    fn check_test_box(&mut self) {
        let Some(layout) = self.layout.clone() else {
            self.log.push(
                "Start the test box first — the button is on the clean test computer step."
                    .to_string(),
            );
            return;
        };
        // On the try/close steps, make sure the later notes are asked
        // for in order: after-install once its note is missing and the
        // clean note exists, after-runtime likewise.
        let exchange = &layout.exchange;
        let guided_msi = self.uses_guided_msi();
        if (!guided_msi || self.step == 5)
            && exchange.join("after-install.json").is_file()
            && !exchange.join("after-runtime.json").is_file()
        {
            let _ = portableforge_core::orchestrate::request_snapshot(
                &layout,
                CapturePoint::AfterRuntime,
            );
        } else if (!guided_msi || self.step == 3)
            && exchange.join("before.json").is_file()
            && !exchange.join("after-install.json").is_file()
        {
            let _ = portableforge_core::orchestrate::request_snapshot(
                &layout,
                CapturePoint::AfterInstall,
            );
        }
        // A request the helper has not answered for a long while
        // usually means the test box was closed mid-run — say so
        // plainly instead of letting the person wait forever.
        if let Some((point, age)) = portableforge_core::orchestrate::pending_request(&layout) {
            if age > portableforge_core::orchestrate::CAPTURE_WAIT_WARN_SECS {
                self.log.push(format!(
                    "Still waiting for the {} note after {} minutes — the test box may have been closed. You can start it again from the clean test computer step.",
                    point.note_name(),
                    age / 60
                ));
            }
        }
        let state = portableforge_core::orchestrate::read_agent_state(&layout);
        if let Some(s) = &state {
            if let Some(w) =
                portableforge_core::orchestrate::agent_version_warning(s, env!("CARGO_PKG_VERSION"))
            {
                self.log.push(w);
            }
        }
        match portableforge_core::orchestrate::next_host_action(state.as_ref()) {
            portableforge_core::orchestrate::HostAction::Wait => {
                self.log.push(
                    "The test box is still working. Leave it open and check again after closing the app."
                        .to_string(),
                );
            }
            portableforge_core::orchestrate::HostAction::Failed(line) => {
                self.log.push(line);
            }
            portableforge_core::orchestrate::HostAction::Harvest => {
                self.finish_capture(&layout);
            }
        }
    }

    pub(super) fn show_bundled_test_box_help(&mut self, ui: &mut egui::Ui) {
        let folder = std::env::current_exe().ok().and_then(|exe| {
            let folder = exe.parent()?.join("bundled").join("Sandboxie-Plus");
            folder.join("SandMan.exe").is_file().then_some(folder)
        });
        ui.horizontal_wrapped(|ui| {
            let open = ui
                .add_enabled(folder.is_some(), egui::Button::new("Open bundled test box folder"))
                .on_disabled_hover_text("The portable test box files are missing from this copy.");
            if open.clicked() {
                if let Some(folder) = folder {
                    if let Err(error) = open_folder(&folder) {
                        self.log.push(error);
                    }
                }
            }
            dense::view_text(
                ui,
                "Portable test box instructions",
                "Start the bundled test box",
                "Copy the whole bundled Sandboxie-Plus folder to a new protected local folder under Program Files.\n\nOpen SandMan.exe from that copy and approve its administrator prompts to start the Sandboxie-Plus service and driver.\n\nReturn to Capture with Sandboxie-Plus in PortableForge and choose Check prerequisites.\n\nWhen finished with the portable test box, close SandMan and choose Yes when it asks to clean up the services it created.",
                true,
            );
        });
    }

    fn finish_capture(&mut self, layout: &portableforge_core::orchestrate::SessionLayout) {
        match portableforge_core::orchestrate::post_capture(layout) {
            Ok(outcome) => {
                self.apply_capture(outcome);
                if self.uses_guided_msi() {
                    self.msi_review_ready = true;
                }
            }
            Err(error) => self.log.push(error),
        }
    }

    fn apply_capture(&mut self, outcome: portableforge_core::orchestrate::CaptureOutcome) {
        self.log
            .push(format!("Notes read in. {}", outcome.review.summary()));
        self.review = outcome.review;
        // Pre-fill the naming step from the program's
        // own tags — once per capture, and only into
        // fields the person has not filled themselves.
        if !self.hints_applied {
            let hints = &outcome.hints;
            let mut filled: Vec<&str> = Vec::new();
            if self.app_name.trim().is_empty() {
                if let Some(name) = &hints.display_name {
                    self.app_name = name.clone();
                    filled.push("the name");
                }
            }
            if self.publisher.trim().is_empty() {
                if let Some(publisher) = &hints.publisher {
                    self.publisher = publisher.clone();
                    filled.push("who makes it");
                }
            }
            if self.package_version == "1.0.0.0" {
                if let Some(version) = hints
                    .display_version
                    .as_deref()
                    .and_then(portableforge_core::paf::pad_package_version)
                {
                    self.package_version = version;
                    filled.push("the version");
                }
            }
            if !filled.is_empty() {
                self.log.push(format!(
                                    "Filled in {} from the program's own tags — you can check and change them on the naming step.",
                                    filled.join(", ")
                                ));
                self.mark_dirty();
            }
            self.hints_applied = true;
        }
        // Start Menu shortcuts are the honest guess
        // for the main file: the shortcut's name is
        // usually the program's name.
        if !outcome.shortcut_candidates.is_empty() {
            self.log.push(format!(
                                "Shortcut found for: {}. The main file is often the same name with .exe on the end.",
                                outcome.shortcut_candidates.join(", ")
                            ));
            if self.main_exe.trim().is_empty() {
                self.main_exe = format!("{}.exe", outcome.shortcut_candidates[0]);
                self.mark_dirty();
            }
        }
    }

    fn package_metadata(&self) -> Result<portableforge_core::paf::PafMetadata, String> {
        if let Some(error) = self.verified_metadata_error() {
            return Err(error.into());
        }
        let Some(sharing) = self.sharing else {
            return Err("Answer the sharing question first (step 9).".into());
        };
        if self.main_exe.trim().is_empty() {
            return Err(
                "Tell us the program's main file name on the naming step (step 8) first.".into(),
            );
        }
        let online = if sharing == SharingAnswer::NotSure {
            let filename = self
                .online_url
                .trim()
                .rsplit('/')
                .next()
                .unwrap_or("")
                .to_string();
            Some((
                self.online_url.trim().to_string(),
                filename,
                self.online_sha.trim().to_string(),
            ))
        } else {
            None
        };
        let distribution = guide::distribution_for(sharing, online)?;
        let compact: String = self
            .app_name
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();
        if compact.is_empty() {
            return Err("Give your portable copy a name first (step 8).".into());
        }
        let base = compact
            .strip_suffix("Portable")
            .unwrap_or(&compact)
            .to_string();
        let app_id = if compact.ends_with("Portable") {
            compact.clone()
        } else {
            format!("{compact}Portable")
        };
        Ok(portableforge_core::paf::PafMetadata {
            name: self.app_name.trim().to_string(),
            app_id,
            base_app_name: base,
            publisher: self.publisher.trim().to_string(),
            homepage: self.homepage.trim().to_string(),
            category: "Utilities".to_string(),
            description: self.description.trim().to_string(),
            language: "English".to_string(),
            shareable: !matches!(
                distribution,
                portableforge_core::paf::Distribution::Online { .. }
            ),
            open_source: false,
            freeware: sharing == SharingAnswer::FreeToShare,
            commercial_use: false,
            package_version: self.package_version.trim().to_string(),
            display_version: self.package_version.trim().to_string(),
            requires_admin: self.review.has_red_flags(),
            main_exe: self.main_exe.trim().to_string(),
            main_exe_64bit: None,
            distribution,
        })
    }

    fn build_now(&mut self) {
        let meta = match self.package_metadata() {
            Ok(meta) => meta,
            Err(error) => {
                self.log.push(error);
                return;
            }
        };
        let out_root = match &self.layout {
            Some(layout) => layout.output.join(&meta.app_id),
            None => sessions_base().join(&meta.app_id),
        };
        let harvest = self
            .layout
            .as_ref()
            .map(|l| l.exchange.join("harvest"))
            .filter(|p| p.is_dir());
        let icon = if self.icon_path.trim().is_empty() {
            None
        } else {
            Some(std::path::PathBuf::from(self.icon_path.trim()))
        };
        let outcome = portableforge_core::build::build_package(
            &out_root,
            &meta,
            &self.review.accepted(),
            harvest.as_deref(),
            None,
            icon.as_deref(),
            false,
        );
        match outcome {
            Ok(outcome) => {
                let mut text = format!("{}\nMade in: {}", outcome.summary(), out_root.display());
                for issue in &outcome.issues {
                    text.push_str(&format!("\n{}: {}", issue.field, issue.message));
                }
                self.log.push(outcome.summary());
                self.build_summary = Some(text);
                self.built_dir = Some(out_root);
            }
            Err(e) => self.log.push(e),
        }
    }

    fn phase(&self) -> Phase {
        match self.step {
            0 => Phase::Pick,
            1 => Phase::SafetyCheck,
            2 => Phase::PreScan,
            3 => Phase::Install,
            4 => Phase::TryOut,
            5 => Phase::Close,
            6 => Phase::ReviewChanges,
            7 => Phase::Name,
            8 => Phase::Sharing,
            _ => Phase::Build,
        }
    }

    /// The run so far, as a draft for the settings file.
    fn current_draft(&self) -> WizardDraft {
        WizardDraft {
            step: if self.use_verified_capture() {
                "pick"
            } else if self.uses_guided_msi() && self.step > 1 {
                "safety"
            } else {
                STEPS[self.step].id
            }
            .to_string(),
            installer_path: self.installer_path.clone(),
            app_name: self.app_name.clone(),
            description: self.description.clone(),
            publisher: self.publisher.clone(),
            homepage: self.homepage.clone(),
            main_exe: self.main_exe.clone(),
            package_version: self.package_version.clone(),
            online_url: self.online_url.clone(),
            online_sha: self.online_sha.clone(),
            force_sandbox: self.force_sandbox,
            cleaned_capture: self.profile_flow.cleaned_capture.clone(),
            profile_update_base: self.profile_flow.update_base.clone(),
        }
    }

    /// Note that the settings (draft included) changed and should be
    /// written out soon — the write itself is throttled in `update`.
    fn mark_dirty(&mut self) {
        self.settings.draft = Some(self.current_draft());
        self.settings_dirty = true;
    }

    /// Write the settings out if they changed and a moment has passed
    /// since the last write (called every frame; the throttle keeps
    /// typing from hammering the disk).
    fn persist_if_due(&mut self) {
        if self.settings_dirty && self.last_save.elapsed() >= std::time::Duration::from_secs(1) {
            self.persist_now();
        }
    }

    fn persist_now(&mut self) {
        // Saves the settings exactly as they stand — callers that
        // changed the run snapshot the draft first (see mark_dirty);
        // callers clearing the draft (Finish, Start fresh) rely on
        // this not putting it back.
        self.last_save = std::time::Instant::now();
        if settings::save(&self.settings_path, &self.settings).is_ok() {
            self.settings_dirty = false;
        }
    }

    /// Take up the unfinished run the settings remembered.
    fn resume_draft(&mut self, ctx: &egui::Context) {
        let Some(draft) = self.settings.draft.clone() else {
            return;
        };
        self.installer_path = draft.installer_path;
        self.app_name = draft.app_name;
        self.description = draft.description;
        self.publisher = draft.publisher;
        self.homepage = draft.homepage;
        self.main_exe = draft.main_exe;
        if !draft.package_version.trim().is_empty() {
            self.package_version = draft.package_version;
        }
        self.online_url = draft.online_url;
        self.online_sha = draft.online_sha;
        self.force_sandbox = draft.force_sandbox;
        if let Some(pos) = STEPS.iter().position(|s| s.id == draft.step) {
            self.step = pos;
        }
        self.resume_offer = false;
        self.profile_flow.update_base = draft.profile_update_base;
        if let Some(capture) = draft.cleaned_capture {
            self.restore_cleaned_capture(ctx, capture);
        } else if self.profile_flow.update_base.is_some() {
            self.enter_verified_capture();
            self.check_prerequisites(ctx);
        }
        self.mark_dirty();
        self.log
            .push("Picked up your unfinished run where you left it.".to_string());
    }

    /// Throw the remembered draft away and start the run fresh.
    fn discard_draft(&mut self) {
        self.settings.draft = None;
        self.resume_offer = false;
        self.settings_dirty = true;
        self.persist_now();
        self.log
            .push("Started fresh — the unfinished run was cleared.".to_string());
    }

    /// One step back. Nothing is lost.
    fn go_back(&mut self) {
        if !self.can_go_back() {
            return;
        }
        self.step = match (self.use_verified_capture(), self.step) {
            (true, 2) if self.profile_flow.selected_capture => {
                self.profile_flow.selected_capture = false;
                0
            }
            (true, 6) => 2,
            (_, step) => step - 1,
        };
        self.mark_dirty();
    }

    fn can_go_back(&self) -> bool {
        self.step > 0
            && !self.profile_flow.busy()
            && !self.profile_flow.profile_saved()
            && !(self.use_verified_capture() && self.step == 2 && self.profile_flow.captured)
            && !(self.uses_guided_msi() && self.step == 2 && self.layout.is_some())
    }

    fn can_advance(&self) -> bool {
        if self.profile_flow.busy() {
            return false;
        }
        if self.step == 0 {
            return matches!(setup_input(&self.installer_path), SetupInput::Ready { .. });
        }
        if self.uses_guided_msi() {
            let note = |name| {
                self.layout
                    .as_ref()
                    .is_some_and(|layout| layout.exchange.join(name).is_file())
            };
            match self.step {
                1 => {
                    return self
                        .profile_flow
                        .msi_ready
                        .as_ref()
                        .is_some_and(Result::is_ok)
                }
                2 => return note("before.json"),
                3 => return note("after-install.json"),
                5 => return self.msi_review_ready,
                _ => {}
            }
        }
        match (self.use_verified_capture(), self.step) {
            (true, 2) => self.profile_flow.captured,
            (true, 6) => self.review.all_acknowledged(),
            (true, 9) => self.profile_flow.profile_saved(),
            _ => true,
        }
    }

    /// One step forward, when the current step's promise is kept —
    /// the same check the Continue button makes (a plain refusal in
    /// the log, never a trap). Returns true when the step moved or
    /// the run finished.
    fn advance(&mut self) -> bool {
        if !self.can_advance() {
            return false;
        }
        if self.use_verified_capture() && self.step == 2 {
            self.step = 6;
            self.mark_dirty();
            return true;
        }
        if self.use_verified_capture() && self.step == 9 {
            self.step = 11;
            return true;
        }
        let blocked = match STEPS[self.step].id {
            "pick" => {
                self.settings.remember_installer(self.installer_path.trim());
                if self.uses_guided_msi() {
                    self.force_sandbox = false;
                }
                false
            }
            "name" => {
                if self.app_name.trim().is_empty() {
                    self.log.push(
                        "Give your portable copy a name first — anything short and clear."
                            .to_string(),
                    );
                    true
                } else if self.main_exe.trim().is_empty() {
                    self.log.push(
                        "Add the program's main file name — it ends in .exe and starts the program."
                            .to_string(),
                    );
                    true
                } else if let Some(error) = self.verified_metadata_error() {
                    self.log.push(error.into());
                    true
                } else {
                    false
                }
            }
            "changes" => {
                if !self.review.items.is_empty() && !self.review.all_acknowledged() {
                    self.log.push(
                        "There is a warning above that needs a tick in its box before we go on."
                            .to_string(),
                    );
                    true
                } else {
                    false
                }
            }
            "sharing" => {
                if self.use_verified_capture()
                    && matches!(self.sharing, None | Some(SharingAnswer::NotSure))
                {
                    self.log
                        .push("Confirm redistribution rights before saving a profile.".into());
                    true
                } else if self.sharing.is_none() {
                    self.log.push(
                        "Pick one answer above — 'I am not sure' is a good, safe answer."
                            .to_string(),
                    );
                    true
                } else {
                    if self.sharing == Some(SharingAnswer::NotSure) {
                        self.log.push(
                            "Safest choice picked. When we build, we will use the maker's own download link and check it is genuine."
                                .to_string(),
                        );
                    }
                    false
                }
            }
            _ => false,
        };
        if blocked {
            return false;
        }
        if self.step + 1 < STEPS.len() {
            self.step += 1;
            self.log
                .push(format!("Moved to: {}", STEPS[self.step].title));
            self.mark_dirty();
            true
        } else {
            // Finish on the last step: the run is complete, so the
            // unfinished-run draft goes away — and the window
            // closes, so Finish visibly finishes. (Before this, the
            // click only cleared the draft behind the scenes and
            // looked like it did nothing — field report,
            // 2026-10-25.)
            self.settings.draft = None;
            self.settings_dirty = true;
            self.persist_now();
            self.log
                .push("All done — this run is finished.".to_string());
            self.finished = true;
            true
        }
    }
}

impl eframe::App for WizardApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.apply_layout_zoom(ctx);
        self.theme_state.apply(ctx, self.settings.appearance);
        if self.finished {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.poll_profile_work(ctx);
        if let Some(path) = self.folder_flow.poll(ctx) {
            self.installer_path = path.to_string_lossy().into_owned();
            self.settings.remember_installer(&self.installer_path);
            self.setup_changed();
            self.log
                .push(format!("Setup file chosen: {}", self.installer_path));
        }
        if (self.profile_flow.busy() || self.folder_flow.busy())
            && ctx.input(|input| input.viewport().close_requested())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        self.persist_if_due();
        if !self.artwork.is_open() {
            self.handle_shell_keys(ctx);
        }
        self.update_heading_focus(ctx);
        if self.profile_flow.replay_open {
            self.show_profile_replay(ctx);
            self.show_shell_popup(ctx);
            return;
        }

        let pick_step = STEPS[self.step.min(STEPS.len() - 1)].id == "pick";
        if (pick_step || self.folder_flow.open)
            && !self.folder_flow.busy()
            && !ctx.memory(|memory| memory.top_modal_layer().is_some())
        {
            let dropped = ctx.input_mut(|i| {
                std::mem::take(&mut i.raw.dropped_files)
                    .into_iter()
                    .find_map(|file| file.path)
            });
            if let Some(path) = dropped {
                self.inspect_app_folder(ctx, path, true);
                self.update_heading_focus(ctx);
            }
        }
        if self.folder_flow.open {
            self.show_folder_flow(ctx);
            self.show_shell_popup(ctx);
            return;
        }

        let step = &STEPS[self.step];
        let verified = self.use_verified_capture();
        let guided_msi = self.uses_guided_msi();
        let (title, what_happens, what_you_do) = match (guided_msi, step.id) {
            (true, "safety") => (
                "Check Windows Sandbox",
                "MSI setup runs inside Windows Sandbox.",
                "The guided steps prepare a package folder.",
            ),
            (true, "clean") => (
                "Prepare the MSI test box",
                "Choose any companion files before starting Windows Sandbox.",
                "Wait for the clean note before opening setup.",
            ),
            (true, "install") => (
                "Install the app in Windows Sandbox",
                r"Open the selected MSI from C:\PortableForge\Input inside the box.",
                "Tell PortableForge when setup finishes.",
            ),
            (true, "try") => (
                "Try the installed app",
                "Open the app inside Windows Sandbox and try the settings you want to keep.",
                "Continue when you are ready to close it.",
            ),
            (true, "close") => (
                "Close the installed app",
                "Keep Windows Sandbox open while PortableForge takes its last note.",
                "Tell PortableForge when the app has closed.",
            ),
            _ => match (verified, step.id) {
                (_, "pick") => (
                    folder_copy::PICK_TITLE,
                    folder_copy::PICK_HELP,
                    step.what_you_do,
                ),
                (true, "clean") => (
                    "Check your setup",
                    "Check the dedicated box and packaging tools before starting capture.",
                    "Review opens after cleanup is confirmed.",
                ),
                (true, "changes") => (
                    "Review captured changes",
                    "Review the location and suggested fix for each change.",
                    "Select the settings the portable copy should keep.",
                ),
                (true, "name") => (
                    "Name your portable copy",
                    "Review the app details that appear in the installer.",
                    "Enter a publisher, homepage and short description.",
                ),
                (true, "sharing") => (
                    "Confirm redistribution rights",
                    "Confirm that you have permission to include the app files.",
                    "A saved profile contains a copy of the app.",
                ),
                (true, "build") => (
                    "Save a reusable profile",
                    "Save the app files and fixes you approved.",
                    "Use the saved profile to build again from the same setup file.",
                ),
                (true, "done") => (
                    "Your profile is ready",
                    "The reviewed profile is saved.",
                    "Build installer becomes available when the approval is saved.",
                ),
                _ => (step.title, step.what_happens, step.what_you_do),
            },
        };
        self.show_header(ctx, step.title, Some((self.step + 1, STEPS.len())));
        self.show_navigation(ctx, what_you_do);
        egui::CentralPanel::default()
            .frame(dense::content_frame(ctx))
            .show(ctx, |ui| {
            if self.show_details {
                self.show_step_details(ui, step);
                return;
            }
            dense::heading(ui, title, what_happens);
            // An unfinished run from last time is offered, never forced.
            if self.resume_offer && step.id == "pick" {
                ui.horizontal(|ui| {
                    ui.colored_label(ui.visuals().error_fg_color, "Unfinished run");
                    if ui
                        .button("Pick up where I left off")
                        .on_hover_text("Fill the steps back in with what you had entered.")
                        .clicked()
                    {
                        self.resume_draft(ctx);
                    }
                    if ui
                        .button("Start again")
                        .on_hover_text("Clear the unfinished run and begin fresh.")
                        .clicked()
                    {
                        self.discard_draft();
                    }
                });
            }

            // Compute the safety report the first time its step shows.
            if step.id == "safety" && !guided_msi && self.preflight.is_none() {
                let mut facts = crate::collect_preflight_facts();
                facts.sandbox_override = self.force_sandbox;
                self.preflight = Some(evaluate_preflight(&facts));
                self.preflight_facts = Some(facts);
            }

            // Per-step intent collection (the core decides what it means).
            match step.id {
                "safety" if guided_msi => self.show_msi_safety(ui),
                "safety" => {
                    let mut override_change: Option<bool> = None;
                    if let Some(report) = &self.preflight {
                        card(ui).show(ui, |ui| {
                        for index in dense::page(ui, &mut self.safety_page, report.checks.len(), 1, "Check") {
                            let check = &report.checks[index];
                            let col = if check.ok {
                                ui.visuals().text_color()
                            } else {
                                ui.visuals().error_fg_color
                            };
                            // The tick / cross is DRAWN, not a
                            // character: the bundled font has no
                            // ✓/✗ glyphs and showed hollow squares
                            // on a real machine (field run,
                            // 2026-10-25) — pass and fail looked
                            // identical.
                            ui.horizontal(|ui| {
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(16.0, 16.0),
                                    egui::Sense::hover(),
                                );
                                let painter = ui.painter_at(rect);
                                let c = rect.center();
                                let stroke = egui::Stroke::new(2.0_f32, col);
                                if check.ok {
                                    painter.line_segment(
                                        [
                                            egui::pos2(c.x - 5.0, c.y),
                                            egui::pos2(c.x - 1.5, c.y + 4.0),
                                        ],
                                        stroke,
                                    );
                                    painter.line_segment(
                                        [
                                            egui::pos2(c.x - 1.5, c.y + 4.0),
                                            egui::pos2(c.x + 5.5, c.y - 4.5),
                                        ],
                                        stroke,
                                    );
                                } else {
                                    painter.line_segment(
                                        [
                                            egui::pos2(c.x - 4.0, c.y - 4.0),
                                            egui::pos2(c.x + 4.0, c.y + 4.0),
                                        ],
                                        stroke,
                                    );
                                    painter.line_segment(
                                        [
                                            egui::pos2(c.x + 4.0, c.y - 4.0),
                                            egui::pos2(c.x - 4.0, c.y + 4.0),
                                        ],
                                        stroke,
                                    );
                                }
                                ui.label(egui::RichText::new(&check.label).color(col))
                                    .on_hover_text("A tick means this is fine. A cross means it needs fixing — the fix is written under it.");
                            });
                            if !check.fix.is_empty() {
                                ui.label(
                                    egui::RichText::new(format!("     {}", check.fix))
                                        .color(ui.visuals().weak_text_color()),
                                );
                            }
                            if let Some(copy) = &check.copy_text {
                                if ui
                                    .small_button("Copy the fix")
                                    .on_hover_text(format!(
                                        "Copy this, so you can paste it instead of retyping: {copy}"
                                    ))
                                    .clicked()
                                {
                                    ui.ctx().copy_text(copy.clone());
                                }
                            }
                        }
                        ui.add_space(6.0);
                        let (name, honesty) = report.best_mode.describe();
                        ui.label(format!("Way we will use: {name}"));
                        ui.label(
                            egui::RichText::new(honesty).color(ui.visuals().weak_text_color()),
                        );
                        if report.best_mode != portableforge_core::wsb::SandboxMode::WindowsSandbox {
                            ui.add_space(6.0);
                            let mut want = self.force_sandbox;
                            if ui
                                .checkbox(&mut want, "Use the safe test box anyway — I know this computer can run it")
                                .on_hover_text("Tick this only if you are sure this computer can run the safe test box — for example, you have used it yourself. PortableForge will try the box; if Windows cannot open it, we will say so plainly and nothing will be touched.")
                                .changed()
                            {
                                override_change = Some(want);
                            }
                        }
                        });
                    }
                    if let Some(want) = override_change {
                        self.force_sandbox = want;
                        if let Some(facts) = &mut self.preflight_facts {
                            facts.sandbox_override = want;
                            let facts = facts.clone();
                            self.preflight = Some(evaluate_preflight(&facts));
                        }
                        self.mark_dirty();
                        self.persist_now();
                    }
                    let sbie_missing = self
                        .preflight_facts
                        .as_ref()
                        .map(|f| !f.sandboxie_present)
                        .unwrap_or(false);
                    if sbie_missing {
                        self.show_bundled_test_box_help(ui);
                        if ui
                            .button("Check again")
                            .on_hover_text("Refresh the standard wizard's system checks.")
                            .clicked()
                        {
                            let mut facts = crate::collect_preflight_facts();
                            facts.sandbox_override = self.force_sandbox;
                            self.preflight = Some(evaluate_preflight(&facts));
                            self.preflight_facts = Some(facts);
                            self.log
                                .push("Checked again with fresh eyes.".to_string());
                        }
                    }
                }
                "changes" => {
                    if verified {
                        self.show_update_review(ui);
                    }
                    if self.review.items.is_empty() {
                        ui.label(
                            egui::RichText::new(
                                "Nothing here yet — this list fills in after the program has been tried in the test box.",
                            )
                            .color(ui.visuals().weak_text_color()),
                        );
                    } else {
                        if verified {
                            let count = self.review.items.iter().filter(|item| {
                                matches!(item.decision, portableforge_core::review::Decision::Acknowledge { seen: false })
                            }).count();
                            ui.horizontal_wrapped(|ui| {
                                if ui.selectable_label(!self.review_needs_attention, "All findings").clicked() {
                                    self.review_needs_attention = false;
                                    self.review_page = 0;
                                }
                                if ui.selectable_label(self.review_needs_attention, format!("Needs attention ({count})")).clicked() {
                                    self.review_needs_attention = true;
                                    self.review_page = 0;
                                }
                                ui.label(dense::review_counts(&self.review));
                            });
                        } else {
                            ui.label(dense::review_counts(&self.review));
                        }
                        let mut items = std::mem::take(&mut self.review.items);
                        let visible = items.iter().enumerate().filter_map(|(index, item)| {
                            (!verified || !self.review_needs_attention || matches!(
                                item.decision, portableforge_core::review::Decision::Acknowledge { seen: false }
                            )).then_some(index)
                        }).collect::<Vec<_>>();
                        let range = dense::page(ui, &mut self.review_page, visible.len(), 1, "Finding");
                        if visible.is_empty() {
                            ui.label("All warnings have been reviewed.");
                        }
                        for index in range {
                            let item_index = visible[index];
                            let item = &mut items[item_index];
                            ui.push_id(("review-finding", item_index), |ui| {
                            card(ui).show(ui, |ui| {
                            let (title, body) = guide::plain_card(&item.classification.finding);
                            let excluded_fix = verified
                                && matches!(item.decision, portableforge_core::review::Decision::Fix { .. })
                                && !self.profile_flow.review_fix_supported(&item.classification.finding);
                            ui.label(egui::RichText::new(title).strong());
                            if !excluded_fix {
                                ui.label(
                                    egui::RichText::new(body)
                                        .color(ui.visuals().weak_text_color()),
                                );
                            }
                            if verified {
                                use portableforge_core::classify::Finding;

                                let location = match &item.classification.finding {
                                    Finding::DirectoriesMove { host_pattern, .. } => std::borrow::Cow::Borrowed(host_pattern.as_str()),
                                    Finding::FilesMove { host_pattern, data_target } => std::borrow::Cow::Owned(format!("{host_pattern}\\{data_target}")),
                                    Finding::RegistryKey { key }
                                    | Finding::AssociationCandidate { key }
                                    | Finding::UninstallMetadata { key } => std::borrow::Cow::Borrowed(key.as_str()),
                                    Finding::AppFile { path }
                                    | Finding::FileWriteRewrite { path, .. } => std::borrow::Cow::Borrowed(path.as_str()),
                                    Finding::RedFlag { detail, .. } => std::borrow::Cow::Borrowed(detail.as_str()),
                                };
                                if dense::path_needs_view(ui, &location, ui.available_width()) {
                                    dense::view_text(ui, "View location", "Finding location", &location, true);
                                } else {
                                    ui.label(location.as_ref());
                                }
                            }
                            match &mut item.decision {
                                portableforge_core::review::Decision::Fix { on } => {
                                    if excluded_fix {
                                        ui.label(self.profile_flow.exclusion_reason(&item.classification.finding));
                                    } else {
                                        ui.checkbox(on, "Apply this fix");
                                    }
                                }
                                portableforge_core::review::Decision::Acknowledge { seen } => {
                                    if verified {
                                        ui.label(self.profile_flow.exclusion_reason(&item.classification.finding));
                                    }
                                    ui.checkbox(seen, "Acknowledge warning");
                                }
                                portableforge_core::review::Decision::Info => {}
                            }
                            });
                            });
                        }
                        self.review.items = items;
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if ui
                                .button("Save these choices to a file")
                                .on_hover_text("Keep the choices you made here in a file, so they can be used again or sent with a problem report.")
                                .clicked()
                            {
                                if let Some(path) = rfd::FileDialog::new()
                                    .set_title("Save these choices")
                                    .set_file_name("review.json")
                                    .add_filter("Choices file", &["json"])
                                    .save_file()
                                {
                                    match std::fs::write(&path, self.review.to_json()) {
                                        Ok(()) => self.log.push(format!(
                                            "Choices saved to {}",
                                            path.display()
                                        )),
                                        Err(e) => self
                                            .log
                                            .push(format!("Saving the choices failed: {e}")),
                                    }
                                }
                            }
                            if !verified && ui
                                .button("Load choices from a file")
                                .on_hover_text("Bring back choices saved earlier and apply them to this list.")
                                .clicked()
                            {
                                if let Some(path) = rfd::FileDialog::new()
                                    .set_title("Load choices")
                                    .add_filter("Choices file", &["json"])
                                    .pick_file()
                                {
                                    match std::fs::read_to_string(&path)
                                        .map_err(|e| e.to_string())
                                        .and_then(|text| Review::from_json(&text))
                                    {
                                        Ok(review) => {
                                            self.review = review;
                                            self.log.push(
                                                "Choices loaded — the list above now uses them."
                                                    .to_string(),
                                            );
                                        }
                                        Err(e) => self.log.push(format!(
                                            "That file could not be read as a choices file: {e}"
                                        )),
                                    }
                                }
                            }
                        });
                    }
                }
                "pick" => {
                    if !self.settings.welcomed {
                        ui.horizontal(|ui| {
                            if ui.button("Got it").on_hover_text("Dismiss the welcome.").clicked() {
                                self.settings.welcomed = true;
                                self.settings_dirty = true;
                                self.persist_now();
                            }
                            dense::view_text(ui, "About this wizard", "Welcome to PortableForge",
                                &guide::WELCOME.join("\n"), true);
                        });
                    }
                    if ui.button(folder_copy::CHOOSE_FOLDER).clicked() {
                        self.choose_app_folder(ctx);
                    }
                    let setup_label = ui.label("Setup file (.exe or .msi)");
                    ui.horizontal(|ui| {
                        let width = dense::width_with_button(ui, "Browse…");
                        let (response, view) = dense::path_edit(ui, &mut self.installer_path, width);
                        if response
                            .labelled_by(setup_label.id)
                            .on_hover_text(
                                "Type or paste the path, use Browse, or drag the setup file here.",
                            )
                            .changed()
                        {
                            self.setup_changed();
                        }
                        if ui
                            .button("Browse…")
                            .on_hover_text("Open a file chooser to find the setup file.")
                            .clicked()
                        {
                            let mut dialog = rfd::FileDialog::new()
                                .set_title("Pick the program's setup file")
                                .add_filter("Setup files", &["exe", "msi"]);
                            if let Some(dir) = picker_start_dir(&self.installer_path) {
                                dialog = dialog.set_directory(dir);
                            }
                            if let Some(path) = dialog.pick_file() {
                                self.installer_path = path.to_string_lossy().into_owned();
                                self.settings
                                    .remember_installer(&self.installer_path.clone());
                                self.setup_changed();
                            }
                        }
                        dense::view_text(ui, "View path", "Setup path", &self.installer_path, view);
                    });
                    let typed = self.installer_path.trim().to_string();
                    let input = setup_input(&typed);
                    if let SetupInput::Ready { size, .. } = input {
                        let name = std::path::Path::new(&typed).file_name().unwrap_or_default().to_string_lossy();
                        let description = format!("Ready · {name} · {}", format_size(size));
                        ui.add(egui::Label::new(&description).truncate()).on_hover_text(description);
                    } else if let Some(problem) = input.problem() {
                        let color = if input == SetupInput::Empty {
                            ui.visuals().weak_text_color()
                        } else {
                            ui.visuals().error_fg_color
                        };
                        ui.colored_label(color, problem);
                    }
                    ui.horizontal_wrapped(|ui| {
                        if !guided_msi
                            && ui.add_enabled(input.allows_verified_picker(), egui::Button::new("Capture with Sandboxie-Plus")).clicked()
                            && setup_input(&self.installer_path).allows_verified_picker()
                        {
                            self.enter_verified_capture();
                            self.check_prerequisites(ctx);
                        }
                        if ui.button("Open saved profiles")
                            .on_hover_text("Rebuild with a saved profile and its original setup file.")
                            .clicked()
                        {
                            self.open_profile_replay(false);
                        }
                    });
                    // Files this wizard has packaged lately, one tap away.
                    let recents = self.settings.recent_installers.clone();
                    let others: Vec<&String> =
                        recents.iter().filter(|p| p.as_str() != typed).collect();
                    if !others.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(egui::RichText::new("Recent setup files").weak());
                            for index in dense::page(ui, &mut self.recent_page, others.len(), 1, "File") {
                                let path = others[index];
                                let name = std::path::Path::new(path)
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| path.clone());
                                if ui.button(name).on_hover_text(path.clone()).clicked() {
                                    self.installer_path = path.clone();
                                    self.setup_changed();
                                }
                            }
                        });
                    }
                }
                "clean" if guided_msi => self.show_msi_capture(ui),
                "clean" => {
                    if verified {
                        self.show_verified_capture(ui);
                    } else {
                        ui.label("How much memory should the test box get?");
                        ui.horizontal(|ui| {
                            for (mb, label) in
                                [(2048u32, "2 GB"), (4096, "4 GB"), (8192, "8 GB")]
                            {
                                if ui
                                    .radio(self.sandbox_memory_mb == mb, label)
                                    .clicked()
                                {
                                    self.sandbox_memory_mb = mb;
                                }
                            }
                        });
                        ui.checkbox(
                            &mut self.sandbox_networking,
                            "Let the test box use the internet (off is safer)",
                        );
                        ui.collapsing("More choices", |ui| {
                            ui.checkbox(
                                &mut self.sandbox_clipboard,
                                "Share the clipboard with the test box (off is safer)",
                            );
                        });
                        ui.add_space(4.0);
                        if ui.button("Start the test box").clicked() {
                            self.start_test_box();
                        }
                        ui.add_space(4.0);
                        if ui.button("Check on the test box").clicked() {
                            self.check_test_box();
                        }
                    }
                }
                "install" | "close" if guided_msi => {
                    let label = if self.step == 3 { "MSI setup finished" } else { "App is closed" };
                    if ui.button(label).clicked() {
                        self.check_test_box();
                    }
                }
                "try" if guided_msi => {}
                "install" | "try" | "close" => {
                    if ui
                        .button("Check on the test box")
                        .on_hover_text("Ask the test box for its next note and look at what it has done so far. When all three notes are in, the list on the next steps fills itself in.")
                        .clicked()
                    {
                        self.check_test_box();
                    }
                }
                "build" if verified => self.show_profile_save(ui),
                "done" if verified => self.show_saved_profile(ui),
                "build" => {
                    if ui
                        .button("Build my portable copy now")
                        .on_hover_text("Put everything together into the finished package folder and check it.")
                        .clicked()
                    {
                        self.build_now();
                    }
                    if let Some(text) = &self.build_summary {
                        dense::view_text(ui, "Show build report", "Build report", text, true);
                    }
                }
                "test" | "done" => {
                    if let Some(text) = &self.build_summary {
                        dense::view_text(ui, "Show build report", "Build report", text, true);
                    }
                    if step.id == "test" {
                        ui.add_space(4.0);
                        if ui
                            .button("Check the leak-test result")
                            .on_hover_text("Read the two notes the leak-test packet made (leak-before and leak-after, in this run's exchange folder) and judge whether the finished copy stayed inside its own folder.")
                            .clicked()
                        {
                            self.check_leak_result();
                        }
                        if let Some(text) = &self.leak_result {
                            dense::view_text(ui, "Show test result", "Test result", text, true);
                        }
                    }
                    if step.id == "done" {
                        ui.add_space(6.0);
                        if let Some(dir) = self.built_dir.clone() {
                            if ui
                                .button("Open the folder with your portable copy")
                                .on_hover_text(dir.to_string_lossy().into_owned())
                                .clicked()
                            {
                                if let Err(e) = open_folder(&dir) {
                                    self.log.push(e);
                                }
                            }
                        }
                        if let Some(layout) = self.layout.clone() {
                            if ui
                                .button("Open the work folder")
                                .on_hover_text(layout.root.to_string_lossy().into_owned())
                                .clicked()
                            {
                                if let Err(e) = open_folder(&layout.root) {
                                    self.log.push(e);
                                }
                            }
                        }
                        ui.add_space(6.0);
                        if ui
                            .button("Save the run log")
                            .on_hover_text("Save one plain text file with everything this run did and decided — handy to keep, or to send to whoever helps you.")
                            .clicked()
                        {
                            self.export_run_log();
                        }
                    }
                }
                "name" => {
                    let mut changed = false;
                    ui.columns(2, |columns| {
                        changed |= dense::text_field(&mut columns[0], "Portable app name", &mut self.app_name);
                        changed |= dense::text_field(&mut columns[1], "Publisher", &mut self.publisher);
                    });
                    ui.columns(2, |columns| {
                        changed |= dense::text_field(&mut columns[0], "Main app file (.exe)", &mut self.main_exe);
                        changed |= dense::text_field(&mut columns[1], "Package version (e.g. 1.0.0.0)", &mut self.package_version);
                    });
                    if verified {
                        changed |= dense::text_field(ui, "App homepage (HTTP or HTTPS)", &mut self.homepage);
                    }
                    let description = ui.label("App description");
                    changed |= ui.add(egui::TextEdit::multiline(&mut self.description)
                        .desired_width(f32::INFINITY).desired_rows(2))
                        .labelled_by(description.id).changed();
                    if changed {
                        self.mark_dirty();
                    }
                    if let Some(error) = self.verified_metadata_error() {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                    } else if !portableforge_core::paf::valid_package_version(self.package_version.trim()) {
                        ui.colored_label(ui.visuals().error_fg_color, "Use four numbers with dots between them, like 1.0.0.0.");
                    }
                    if !verified && self.show_icon_field(ui) {
                        self.mark_dirty();
                    }
                }
                "sharing" => {
                    for option in SHARING_ANSWERS {
                        if verified && option.answer == SharingAnswer::NotSure {
                            continue;
                        }
                        let selected = self.sharing == Some(option.answer);
                        if ui
                            .radio(selected, option.label)
                            .on_hover_text(option.explanation)
                            .clicked()
                        {
                            self.sharing = Some(option.answer);
                        }
                        ui.label(
                            egui::RichText::new(option.explanation)
                                .color(ui.visuals().weak_text_color()),
                        );
                        ui.add_space(4.0);
                    }
                    if !verified && self.sharing == Some(SharingAnswer::NotSure) {
                        ui.add_space(4.0);
                        ui.label("Where the maker offers the original program (a web address starting https://)");
                        if ui.text_edit_singleline(&mut self.online_url).changed() {
                            self.mark_dirty();
                        }
                        ui.label("That file's fingerprint — 64 letters and numbers given by the maker, if they publish one");
                        if ui.text_edit_singleline(&mut self.online_sha)
                            .on_hover_text("The finished copy checks this fingerprint against the file it downloads.")
                            .changed()
                        {
                            self.mark_dirty();
                        }
                    }
                }
                _ => {}
            }

            let _ = self.phase();
        });
        self.show_shell_popup(ctx);
        self.artwork.show(ctx);
    }
}

pub fn run_gui() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([680.0, 480.0])
            .with_min_inner_size(dense::MIN_CONTENT)
            .with_title("PortableForge"),
        ..Default::default()
    };
    eframe::run_native(
        "PortableForge",
        options,
        Box::new(|cc| {
            theme::install_fonts(&cc.egui_ctx);
            Ok(Box::new(WizardApp::default()))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_validation_controls_both_routes() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("portableforge-pick-{}-{stamp}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let exe = root.join("Example Setup.EXE");
        let msi = root.join("Example Setup.MSI");
        let zip = root.join("Example Setup.zip");
        for path in [&exe, &msi, &zip] {
            std::fs::write(path, b"inert test bytes").unwrap();
        }
        let missing = root.join("Missing Setup.exe");
        let mut wizard = WizardApp {
            settings: WizardSettings::default(),
            ..Default::default()
        };
        for (path, expected, continue_ready, verified_picker) in [
            (" ".into(), SetupInput::Empty, false, true),
            (root.to_string_lossy(), SetupInput::Folder, false, false),
            (missing.to_string_lossy(), SetupInput::Missing, false, false),
            (zip.to_string_lossy(), SetupInput::Unsupported, false, false),
            (
                exe.to_string_lossy(),
                SetupInput::Ready {
                    kind: SetupKind::Exe,
                    size: 16,
                },
                true,
                true,
            ),
            (
                msi.to_string_lossy(),
                SetupInput::Ready {
                    kind: SetupKind::Msi,
                    size: 16,
                },
                true,
                false,
            ),
        ] {
            wizard.installer_path = path.into_owned();
            let observed = setup_input(&wizard.installer_path);
            assert_eq!(observed, expected);
            assert_eq!(observed.allows_verified_picker(), verified_picker);
            assert_eq!(observed.problem().is_none(), continue_ready);
            assert_eq!(
                wizard.current_wizard_action(),
                Some((shell::WizardAction::Continue, continue_ready))
            );
            if !continue_ready {
                assert!(!wizard.advance());
                assert_eq!(wizard.step, 0);
            }
        }
        assert!(wizard.advance());
        assert_eq!(wizard.step, 1);
        assert_eq!(
            wizard.settings.recent_installers,
            vec![msi.to_string_lossy()]
        );
        for path in [exe, msi, zip] {
            std::fs::remove_file(path).unwrap();
        }
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn changing_setup_detaches_previous_capture_and_approval() {
        let mut wizard = WizardApp::default();
        wizard.profile_flow.captured = true;
        wizard.profile_flow.cleaned_capture = Some(settings::CleanedCapture {
            folder: "old-capture".into(),
            receipt_sha256: "a".repeat(64),
        });
        wizard.main_exe = "old.exe".into();
        wizard.sharing = Some(SharingAnswer::IMadeIt);
        wizard.installer_path = "replacement.exe".into();
        wizard.setup_changed();
        assert!(!wizard.profile_flow.captured);
        assert!(wizard.profile_flow.cleaned_capture.is_none());
        assert!(wizard.main_exe.is_empty());
        assert!(wizard.sharing.is_none());
        assert_eq!(wizard.installer_path, "replacement.exe");
    }

    #[test]
    fn guided_msi_requires_its_own_preflight_even_with_override() {
        let mut wizard = WizardApp {
            installer_path: "selected.MSI".into(),
            step: 1,
            force_sandbox: true,
            ..Default::default()
        };
        assert!(wizard.uses_guided_msi());
        assert!(!wizard.use_verified_capture());
        assert!(!wizard.can_advance());
        wizard.profile_flow.msi_ready = Some(Err("Windows Sandbox unavailable".into()));
        assert!(!wizard.can_advance());
        wizard.profile_flow.msi_ready = Some(Ok(()));
        assert!(wizard.can_advance());
        wizard.step = 2;
        assert!(!wizard.can_advance());
    }

    #[test]
    fn sizes_read_plainly() {
        assert_eq!(format_size(0), "0 bytes");
        assert_eq!(format_size(812), "812 bytes");
        assert_eq!(format_size(812_000), "812 KB");
        assert_eq!(format_size(14_200_000), "14.2 MB");
        assert_eq!(format_size(1_300_000_000), "1.3 GB");
    }

    /// Item 83: the jargon guard covers the wizard's own strings.
    /// Extract every string literal that looks like a sentence (it
    /// contains a space) from this file — outside comments and
    /// outside this test — and check it against the same banned list
    /// as the guide's primary wording. A tiny allowlist names the
    /// literals that are data, not wording (file paths, format
    /// internals).
    #[test]
    fn wizard_strings_stay_plain() {
        let src = include_str!("wizard.rs");
        // Cut this test itself out (it names the banned list logic).
        let src = src.split("fn wizard_strings_stay_plain").next().unwrap();
        // Strip // line comments.
        let mut cleaned = String::new();
        for line in src
            .lines()
            .chain(include_str!("wizard/profiles.rs").lines())
            .chain(include_str!("wizard/dense.rs").lines())
            .chain(include_str!("wizard/artwork_ui.rs").lines())
        {
            if !line.trim_start().starts_with("//") {
                cleaned.push_str(line);
                cleaned.push('\n');
            }
        }
        // Extract string literals (plain and raw).
        let bytes = cleaned.as_bytes();
        let mut literals: Vec<String> = Vec::new();
        let mut i = 0usize;
        while i < bytes.len() {
            if bytes[i] == b'r' && i + 1 < bytes.len() && bytes[i + 1] == b'"' {
                // Raw string r"..."
                if let Some(end) = cleaned[i + 2..].find('"') {
                    literals.push(cleaned[i + 2..i + 2 + end].to_string());
                    i += 2 + end + 1;
                    continue;
                }
            }
            if bytes[i] == b'"' {
                let mut j = i + 1;
                let mut lit = String::new();
                while j < bytes.len() {
                    match bytes[j] {
                        b'\\' if j + 1 < bytes.len() => {
                            lit.push(bytes[j + 1] as char);
                            j += 2;
                        }
                        b'"' => break,
                        c => {
                            lit.push(c as char);
                            j += 1;
                        }
                    }
                }
                literals.push(lit);
                i = j + 1;
                continue;
            }
            i += 1;
        }
        // Literals that are data rather than wording: file names and
        // paths, format pieces, and clipboard payloads the user
        // asked us to copy verbatim.
        let allow = [
            "wizard-settings.json",
            "portableforge-agent.exe",
            "state.json",
            "session.json",
            "leak-before.json",
            "leak-after.json",
            "before.json",
            "after-install.json",
            "after-runtime.json",
            "build-report.json",
            "SystemPropertiesProtection",
        ];
        let mut failures = Vec::new();
        for lit in &literals {
            if !lit.contains(' ') {
                continue;
            }
            if allow.iter().any(|a| lit.contains(a)) {
                continue;
            }
            let lower = lit
                .to_ascii_lowercase()
                .replace("sandboxie-plus", "")
                .replace("windows sandbox", "")
                .replace("launcher generator", "")
                .replace("sha-256", "");
            for banned in portableforge_core::guide::BANNED_IN_PRIMARY {
                if lower.contains(banned) {
                    failures.push(format!("'{banned}' in: {lit}"));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "wizard wording must stay plain:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn picker_opens_where_the_person_already_is() {
        assert_eq!(picker_start_dir(""), None);
        assert_eq!(picker_start_dir("   "), None);
        // A folder path opens in itself.
        let dir = std::env::temp_dir();
        assert_eq!(picker_start_dir(&dir.to_string_lossy()), Some(dir.clone()));
        // A file path opens in its folder.
        let file = dir.join("setup.exe");
        assert_eq!(picker_start_dir(&file.to_string_lossy()), Some(dir));
        // A path whose folder does not exist leaves the choice alone.
        assert_eq!(picker_start_dir(r"Z:\no\such\place\setup.exe"), None);
    }
}
