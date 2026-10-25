//! The guided wizard (egui, dark only).
//!
//! The GUI renders state and collects intent only — every word it shows
//! and every decision it offers comes from `portableforge_core::guide`
//! and the other core modules. No business rules live here.

use eframe::egui;
use portableforge_core::guide::{self, SharingAnswer, SHARING_ANSWERS, STEPS};
use portableforge_core::orchestrate::{
    evaluate_preflight, CapturePoint, PreflightReport, SessionLayout,
};
use portableforge_core::review::Review;
use portableforge_core::session::Phase;
use portableforge_core::settings::{self, WizardDraft, WizardSettings};

pub struct WizardApp {
    step: usize,
    show_details: bool,
    installer_path: String,
    app_name: String,
    description: String,
    publisher: String,
    main_exe: String,
    package_version: String,
    online_url: String,
    online_sha: String,
    sharing: Option<SharingAnswer>,
    preflight: Option<PreflightReport>,
    /// The collected machine facts behind the safety report, kept
    /// so the "use it anyway" choice can re-judge them on the spot.
    preflight_facts: Option<portableforge_core::orchestrate::PreflightFacts>,
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
        let settings_path = settings::settings_path();
        let settings = settings::load(&settings_path);
        let resume_offer = settings
            .draft
            .as_ref()
            .map(|d| d.has_progress())
            .unwrap_or(false);
        Self {
            step: 0,
            show_details: false,
            installer_path: String::new(),
            app_name: String::new(),
            description: String::new(),
            publisher: String::new(),
            main_exe: String::new(),
            package_version: "1.0.0.0".to_string(),
            online_url: String::new(),
            online_sha: String::new(),
            sharing: None,
            preflight: None,
            preflight_facts: None,
            force_sandbox: false,
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
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home).join("PortableForge Sessions")
}

fn color(hex: &str) -> egui::Color32 {
    let (r, g, b) = guide::theme::rgb(hex);
    egui::Color32::from_rgb(r, g, b)
}

pub fn dark_visuals() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    v.panel_fill = color(guide::theme::BG);
    v.window_fill = color(guide::theme::BG);
    v.extreme_bg_color = color(guide::theme::PANEL);
    v.faint_bg_color = color(guide::theme::PANEL);
    v.override_text_color = Some(color(guide::theme::TEXT));
    v.hyperlink_color = color(guide::theme::ACCENT_RED);
    v.selection.bg_fill = color(guide::theme::PRIMARY_RED);
    v.selection.stroke = egui::Stroke::new(1.0_f32, color(guide::theme::ACCENT_RED));
    v.widgets.active.bg_fill = color(guide::theme::PRIMARY_RED);
    v.widgets.active.bg_stroke = egui::Stroke::new(1.0_f32, color(guide::theme::ACCENT_RED));
    v.widgets.hovered.bg_fill = color(guide::theme::HAIRLINE);
    v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0_f32, color(guide::theme::TEXT_FAINT));
    v.widgets.inactive.bg_fill = color(guide::theme::PANEL);
    v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0_f32, color(guide::theme::HAIRLINE));
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, color(guide::theme::HAIRLINE));
    let radius = egui::CornerRadius::same(6);
    for w in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
        &mut v.widgets.noninteractive,
    ] {
        w.corner_radius = radius;
    }
    v
}

/// The whole window style: family colours, rounded widgets,
/// Fluent-rhythm spacing (4/8/16/24), readable text sizes, and a
/// short animation time so hovers, scrolls and state changes glide
/// instead of snapping. Presentation only — no wording lives here.
pub fn polished_style() -> egui::Style {
    let mut style = egui::Style {
        visuals: dark_visuals(),
        ..Default::default()
    };
    style.animation_time = 0.15;
    style.spacing.item_spacing = egui::vec2(10.0, 9.0);
    style.spacing.button_padding = egui::vec2(14.0, 7.0);
    style.spacing.indent = 20.0;
    style.spacing.interact_size = egui::vec2(42.0, 28.0);
    style.spacing.slider_width = 180.0;
    style.text_styles = [
        (
            egui::TextStyle::Heading,
            egui::FontId::new(26.0, egui::FontFamily::Proportional),
        ),
        (
            egui::TextStyle::Body,
            egui::FontId::new(15.5, egui::FontFamily::Proportional),
        ),
        (
            egui::TextStyle::Button,
            egui::FontId::new(15.0, egui::FontFamily::Proportional),
        ),
        (
            egui::TextStyle::Small,
            egui::FontId::new(12.5, egui::FontFamily::Proportional),
        ),
        (
            egui::TextStyle::Monospace,
            egui::FontId::new(13.5, egui::FontFamily::Monospace),
        ),
    ]
    .into();
    style
}

/// A content card: panel fill, hairline edge, soft corners,
/// comfortable padding. Step controls live on cards so the window
/// reads as grouped panels instead of text floating on black.
pub fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(color(guide::theme::PANEL))
        .stroke(egui::Stroke::new(1.0_f32, color(guide::theme::HAIRLINE)))
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::same(16))
}

impl WizardApp {
    /// Make the session folder, copy the setup file and the helper
    /// into it, ask the helper for the clean note, and start the test
    /// box (on Windows). Safe to press again — an existing session is
    /// kept and only the box start is retried.
    fn start_test_box(&mut self) {
        if self.layout.is_none() {
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
        if exchange.join("after-install.json").is_file()
            && !exchange.join("after-runtime.json").is_file()
        {
            let _ = portableforge_core::orchestrate::request_snapshot(
                &layout,
                CapturePoint::AfterRuntime,
            );
        } else if exchange.join("before.json").is_file()
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
                    "The test box is still working. Finish inside it, close it, then check again."
                        .to_string(),
                );
            }
            portableforge_core::orchestrate::HostAction::Failed(line) => {
                self.log.push(line);
            }
            portableforge_core::orchestrate::HostAction::Harvest => {
                match portableforge_core::orchestrate::post_capture(&layout) {
                    Ok(outcome) => {
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
                    Err(e) => self.log.push(e),
                }
            }
        }
    }

    /// Assemble the package details from the name/sharing steps and
    /// run the build pipeline into the session's output folder.
    fn build_now(&mut self) {
        let Some(sharing) = self.sharing else {
            self.log
                .push("Answer the sharing question first (step 9).".to_string());
            return;
        };
        if self.main_exe.trim().is_empty() {
            self.log.push(
                "Tell us the program's main file name on the naming step (step 8) first."
                    .to_string(),
            );
            return;
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
        let distribution = match guide::distribution_for(sharing, online) {
            Ok(d) => d,
            Err(e) => {
                self.log.push(e);
                return;
            }
        };
        let compact: String = self
            .app_name
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();
        if compact.is_empty() {
            self.log
                .push("Give your portable copy a name first (step 8).".to_string());
            return;
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
        let meta = portableforge_core::paf::PafMetadata {
            name: self.app_name.trim().to_string(),
            app_id: app_id.clone(),
            base_app_name: base,
            publisher: self.publisher.trim().to_string(),
            homepage: String::new(),
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
        };
        let out_root = match &self.layout {
            Some(layout) => layout.output.join(&app_id),
            None => sessions_base().join(&app_id),
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
            step: STEPS[self.step.min(STEPS.len() - 1)].id.to_string(),
            installer_path: self.installer_path.clone(),
            app_name: self.app_name.clone(),
            description: self.description.clone(),
            publisher: self.publisher.clone(),
            main_exe: self.main_exe.clone(),
            package_version: self.package_version.clone(),
            online_url: self.online_url.clone(),
            online_sha: self.online_sha.clone(),
            force_sandbox: self.force_sandbox,
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
        if settings::save(&self.settings_path, &self.settings).is_ok() {
            self.settings_dirty = false;
            self.last_save = std::time::Instant::now();
        }
    }

    /// Take up the unfinished run the settings remembered.
    fn resume_draft(&mut self) {
        let Some(draft) = self.settings.draft.clone() else {
            return;
        };
        self.installer_path = draft.installer_path;
        self.app_name = draft.app_name;
        self.description = draft.description;
        self.publisher = draft.publisher;
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
        if self.step > 0 {
            self.step -= 1;
            self.mark_dirty();
        }
    }

    /// One step forward, when the current step's promise is kept —
    /// the same check the Continue button makes (a plain refusal in
    /// the log, never a trap). Returns true when the step moved or
    /// the run finished.
    fn advance(&mut self) -> bool {
        let blocked = match STEPS[self.step].id {
            "pick" => {
                if self.installer_path.trim().is_empty() {
                    self.log.push(
                        "Pick the program's setup file first — type its full path above."
                            .to_string(),
                    );
                    true
                } else if !std::path::Path::new(self.installer_path.trim()).exists() {
                    self.log.push(format!(
                        "We could not find a file at {} — check the path and try again.",
                        self.installer_path.trim()
                    ));
                    true
                } else {
                    self.settings.remember_installer(self.installer_path.trim());
                    false
                }
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
                if self.sharing.is_none() {
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
            // unfinished-run draft goes away.
            self.settings.draft = None;
            self.settings_dirty = true;
            self.persist_now();
            self.log
                .push("All done — this run is finished.".to_string());
            true
        }
    }
}

impl eframe::App for WizardApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.set_style(polished_style());
        ctx.set_zoom_factor(self.settings.text_scale());
        self.persist_if_due();

        // Keyboard: Enter continues and Escape goes back — but never
        // while the person is typing in a box, where those keys are
        // theirs.
        let typing = ctx.memory(|m| m.focused().is_some());
        if !typing {
            if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
                self.advance();
            } else if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.go_back();
            }
        }

        // Drag-and-drop: dropping the setup file anywhere on the pick
        // step is the same as choosing it with Browse.
        if STEPS[self.step.min(STEPS.len() - 1)].id == "pick" {
            let dropped = ctx.input(|i| i.raw.dropped_files.iter().find_map(|f| f.path.clone()));
            if let Some(path) = dropped {
                self.installer_path = path.to_string_lossy().into_owned();
                self.settings
                    .remember_installer(&self.installer_path.clone());
                self.mark_dirty();
                self.log.push(format!(
                    "Setup file chosen: {}",
                    self.installer_path.clone()
                ));
            }
        }

        let step = &STEPS[self.step.min(STEPS.len() - 1)];
        egui::TopBottomPanel::top("progress").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "Step {} of {} · {}%",
                        self.step + 1,
                        STEPS.len(),
                        (((self.step as f32 + 1.0) / STEPS.len() as f32) * 100.0).round() as u32
                    ))
                    .color(color(guide::theme::TEXT_MUTED)),
                );
                ui.label(
                    egui::RichText::new("PortableForge")
                        .color(color(guide::theme::ACCENT_RED))
                        .strong(),
                );
                ui.label(
                    egui::RichText::new(format!("version {}", env!("CARGO_PKG_VERSION")))
                        .color(color(guide::theme::TEXT_FAINT)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let scale = self.settings.text_scale();
                    for (label, value) in [
                        ("Extra large", 1.5f32),
                        ("Large", 1.25),
                        ("Standard", 1.0),
                    ] {
                        if ui
                            .selectable_label(scale == value, label)
                            .on_hover_text("Make all the text in this window bigger or smaller. Your choice is remembered.")
                            .clicked()
                        {
                            self.settings.text_scale = value;
                            self.settings_dirty = true;
                        }
                    }
                    ui.label(
                        egui::RichText::new("Text size:")
                            .color(color(guide::theme::TEXT_MUTED)),
                    );
                });
            });
            // A slim progress track whose fill glides to the new
            // step instead of jumping — small, but it makes the
            // whole window feel calmer.
            let frac = (self.step as f32 + 1.0) / STEPS.len() as f32;
            let shown = ctx.animate_value_with_time(egui::Id::new("pf-progress"), frac, 0.35);
            ui.add_space(8.0);
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 6.0), egui::Sense::hover());
            let painter = ui.painter_at(rect);
            painter.rect_filled(
                rect,
                egui::CornerRadius::same(3),
                color(guide::theme::PANEL),
            );
            let fill_w = rect.width() * shown.clamp(0.0, 1.0);
            if fill_w > 0.5 {
                let mut fill = rect;
                fill.max.x = rect.min.x + fill_w;
                painter.rect_filled(
                    fill,
                    egui::CornerRadius::same(3),
                    color(guide::theme::ACCENT_RED),
                );
            }
            ui.add_space(8.0);
            ui.separator();
        });

        egui::CentralPanel::default()
            .frame(
                egui::Frame::central_panel(&ctx.style()).inner_margin(egui::Margin {
                    left: 26,
                    right: 26,
                    top: 18,
                    bottom: 14,
                }),
            )
            .show(ctx, |ui| {
            ui.heading(egui::RichText::new(step.title).size(30.0));
            ui.add_space(6.0);
            let (rule, _) =
                ui.allocate_exact_size(egui::vec2(46.0, 3.0), egui::Sense::hover());
            ui.painter_at(rule).rect_filled(
                rule,
                egui::CornerRadius::same(2),
                color(guide::theme::ACCENT_RED),
            );
            ui.add_space(10.0);
            ui.label(egui::RichText::new(step.what_happens).size(16.0));
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(step.what_you_do)
                    .size(16.0)
                    .color(color(guide::theme::TEXT_MUTED)),
            );
            ui.add_space(12.0);

            // An unfinished run from last time is offered, never forced.
            if self.resume_offer {
                ui.label(
                    egui::RichText::new(
                        "You have an unfinished run from last time. You can pick it up where you left it, or start again.",
                    )
                    .color(color(guide::theme::ACCENT_RED)),
                );
                ui.horizontal(|ui| {
                    if ui
                        .button("Pick up where I left off")
                        .on_hover_text("Fill the steps back in with what you had entered.")
                        .clicked()
                    {
                        self.resume_draft();
                    }
                    if ui
                        .button("Start again")
                        .on_hover_text("Clear the unfinished run and begin fresh.")
                        .clicked()
                    {
                        self.discard_draft();
                    }
                });
                ui.add_space(10.0);
            }

            // Compute the safety report the first time its step shows.
            if step.id == "safety" && self.preflight.is_none() {
                let mut facts = crate::collect_preflight_facts();
                facts.sandbox_override = self.force_sandbox;
                self.preflight = Some(evaluate_preflight(&facts));
                self.preflight_facts = Some(facts);
            }

            // Per-step intent collection (the core decides what it means).
            match step.id {
                "safety" => {
                    let mut override_change: Option<bool> = None;
                    if let Some(report) = &self.preflight {
                        card().show(ui, |ui| {
                        for check in &report.checks {
                            let mark = if check.ok { "✓" } else { "✗" };
                            let col = if check.ok {
                                color(guide::theme::TEXT)
                            } else {
                                color(guide::theme::ACCENT_RED)
                            };
                            ui.label(egui::RichText::new(format!("{mark}  {}", check.label)).color(col))
                                .on_hover_text("A tick means this is fine. A cross means it needs fixing — the fix is written under it.");
                            if !check.fix.is_empty() {
                                ui.label(
                                    egui::RichText::new(format!("     {}", check.fix))
                                        .color(color(guide::theme::TEXT_MUTED)),
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
                            egui::RichText::new(honesty).color(color(guide::theme::TEXT_MUTED)),
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
                }
                "changes" => {
                    if self.review.items.is_empty() {
                        ui.label(
                            egui::RichText::new(
                                "Nothing here yet — this list fills in after the program has been tried in the test box.",
                            )
                            .color(color(guide::theme::TEXT_MUTED)),
                        );
                    } else {
                        ui.label(self.review.summary());
                        let mut items = std::mem::take(&mut self.review.items);
                        for item in items.iter_mut() {
                            let (title, body) = guide::plain_card(&item.classification.finding);
                            ui.label(egui::RichText::new(title).strong());
                            ui.label(
                                egui::RichText::new(body)
                                    .color(color(guide::theme::TEXT_MUTED)),
                            );
                            match &mut item.decision {
                                portableforge_core::review::Decision::Fix { on } => {
                                    ui.checkbox(on, "Use this fix");
                                }
                                portableforge_core::review::Decision::Acknowledge { seen } => {
                                    ui.checkbox(seen, "I understand — continue anyway");
                                }
                                portableforge_core::review::Decision::Info => {}
                            }
                            ui.add_space(4.0);
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
                            if ui
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
                        for line in portableforge_core::guide::WELCOME {
                            ui.label(*line);
                        }
                        if ui
                            .button("Got it")
                            .on_hover_text("Hide this welcome. It only shows the first time.")
                            .clicked()
                        {
                            self.settings.welcomed = true;
                            self.settings_dirty = true;
                            self.persist_now();
                        }
                        ui.add_space(8.0);
                    }
                    ui.label("Where is the program's setup file?");
                    ui.horizontal(|ui| {
                        if ui
                            .text_edit_singleline(&mut self.installer_path)
                            .on_hover_text(
                                "Type or paste the full path of the setup file you downloaded, or use Browse.",
                            )
                            .changed()
                        {
                            self.mark_dirty();
                        }
                        if ui
                            .button("Browse…")
                            .on_hover_text("Open a file chooser to find the setup file.")
                            .clicked()
                        {
                            let mut dialog = rfd::FileDialog::new()
                                .set_title("Pick the program's setup file")
                                .add_filter("Setup files", &["exe", "msi", "zip"])
                                .add_filter("All files", &["*"]);
                            if let Some(dir) = picker_start_dir(&self.installer_path) {
                                dialog = dialog.set_directory(dir);
                            }
                            if let Some(path) = dialog.pick_file() {
                                self.installer_path = path.to_string_lossy().into_owned();
                                self.settings
                                    .remember_installer(&self.installer_path.clone());
                                self.mark_dirty();
                            }
                        }
                    });
                    ui.label(
                        egui::RichText::new("You can also drag the setup file onto this window.")
                            .color(color(guide::theme::TEXT_MUTED)),
                    );
                    // Say plainly what the typed path points at.
                    let typed = self.installer_path.trim().to_string();
                    if !typed.is_empty() {
                        match std::fs::metadata(&typed) {
                            Ok(md) if md.is_file() => {
                                let name = std::path::Path::new(&typed)
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_default();
                                ui.label(format!(
                                    "That file is {} — {}.",
                                    format_size(md.len()),
                                    name
                                ));
                            }
                            Ok(_) => {
                                ui.label(
                                    egui::RichText::new(
                                        "That path is a folder — pick the setup file inside it.",
                                    )
                                    .color(color(guide::theme::ACCENT_RED)),
                                );
                            }
                            Err(_) => {
                                ui.label(
                                    egui::RichText::new(
                                        "We can't find a file there yet — check the path, or use Browse.",
                                    )
                                    .color(color(guide::theme::TEXT_MUTED)),
                                );
                            }
                        }
                    }
                    // Files this wizard has packaged lately, one tap away.
                    let recents = self.settings.recent_installers.clone();
                    let others: Vec<&String> =
                        recents.iter().filter(|p| p.as_str() != typed).collect();
                    if !others.is_empty() {
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new("Setup files you used lately:")
                                .color(color(guide::theme::TEXT_MUTED)),
                        );
                        for path in others {
                            let name = std::path::Path::new(path)
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| path.clone());
                            if ui
                                .button(name)
                                .on_hover_text(path.clone())
                                .clicked()
                            {
                                self.installer_path = path.clone();
                                self.mark_dirty();
                            }
                        }
                    }
                }
                "clean" => {
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
                    if ui
                        .button("Start the test box")
                        .on_hover_text("Make the work folder and open the safe test box. The setup file and our helper go inside; nothing touches your real computer.")
                        .clicked()
                    {
                        self.start_test_box();
                    }
                    ui.add_space(4.0);
                    if ui
                        .button("Check on the test box")
                        .on_hover_text("Look at what the helper in the test box has done so far.")
                        .clicked()
                    {
                        self.check_test_box();
                    }
                }
                "install" | "try" | "close" => {
                    if ui
                        .button("Check on the test box")
                        .on_hover_text("Ask the helper for its next note and look at what it has done so far. When all three notes are in, the list on the next steps fills itself in.")
                        .clicked()
                    {
                        self.check_test_box();
                    }
                }
                "build" => {
                    if ui
                        .button("Build my portable copy now")
                        .on_hover_text("Put everything together into the finished package folder and check it.")
                        .clicked()
                    {
                        self.build_now();
                    }
                    if let Some(text) = &self.build_summary {
                        ui.add_space(4.0);
                        ui.label(text.clone());
                    }
                }
                "test" | "done" => {
                    if let Some(text) = &self.build_summary {
                        ui.label(text.clone());
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
                            ui.add_space(4.0);
                            ui.label(text.clone());
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
                    ui.label("Name for your portable copy");
                    if ui.text_edit_singleline(&mut self.app_name).changed() {
                        self.mark_dirty();
                    }
                    if self.app_name.trim().is_empty() {
                        ui.label(
                            egui::RichText::new("A name is needed before you can continue.")
                                .color(color(guide::theme::TEXT_MUTED)),
                        );
                    }
                    ui.label("Who makes the program");
                    if ui.text_edit_singleline(&mut self.publisher).changed() {
                        self.mark_dirty();
                    }
                    ui.label("The program's main file name (ends in .exe)");
                    if ui.text_edit_singleline(&mut self.main_exe)
                        .on_hover_text("The file that starts the program, as it appears inside its folder — for example widget.exe. It was in the list of things the program installed.")
                        .changed()
                    {
                        self.mark_dirty();
                    }
                    let exe = self.main_exe.trim();
                    if exe.is_empty() {
                        ui.label(
                            egui::RichText::new("The main file name is needed before you can continue.")
                                .color(color(guide::theme::TEXT_MUTED)),
                        );
                    } else if exe.contains('\\') || exe.contains('/') {
                        ui.label(
                            egui::RichText::new("Just the file name — not a whole path.")
                                .color(color(guide::theme::ACCENT_RED)),
                        );
                    } else if !exe.to_ascii_lowercase().ends_with(".exe") {
                        ui.label(
                            egui::RichText::new("That doesn't end in .exe — the main file nearly always does.")
                                .color(color(guide::theme::ACCENT_RED)),
                        );
                    }
                    ui.label("Version, as four numbers (like 1.0.0.0)");
                    if ui.text_edit_singleline(&mut self.package_version).changed() {
                        self.mark_dirty();
                    }
                    if !portableforge_core::paf::valid_package_version(
                        self.package_version.trim(),
                    ) {
                        ui.label(
                            egui::RichText::new("Use four numbers with dots between them, like 1.0.0.0.")
                                .color(color(guide::theme::ACCENT_RED)),
                        );
                    }
                    ui.label("A short description");
                    if ui.text_edit_multiline(&mut self.description).changed() {
                        self.mark_dirty();
                    }
                    ui.label("Program picture (not required — a picture file ending in .ico)");
                    ui.horizontal(|ui| {
                        if ui.text_edit_singleline(&mut self.icon_path).changed() {
                            self.mark_dirty();
                        }
                        if ui.button("Browse…").clicked() {
                            let mut dialog = rfd::FileDialog::new()
                                .set_title("Pick the program's picture file")
                                .add_filter("Picture files", &["ico"]);
                            if let Some(dir) = picker_start_dir(&self.icon_path) {
                                dialog = dialog.set_directory(dir);
                            }
                            if let Some(path) = dialog.pick_file() {
                                self.icon_path = path.display().to_string();
                                self.mark_dirty();
                            }
                        }
                    });
                    if !self.icon_path.trim().is_empty()
                        && !std::path::Path::new(self.icon_path.trim()).is_file()
                    {
                        ui.label(
                            egui::RichText::new(
                                "That picture file can't be found — check the name, or leave it blank.",
                            )
                            .color(color(guide::theme::ACCENT_RED)),
                        );
                    }
                }
                "sharing" => {
                    for option in SHARING_ANSWERS {
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
                                .color(color(guide::theme::TEXT_MUTED)),
                        );
                        ui.add_space(4.0);
                    }
                    if self.sharing == Some(SharingAnswer::NotSure) {
                        ui.add_space(4.0);
                        ui.label("Where the maker offers the original program (a web address starting https://)");
                        if ui.text_edit_singleline(&mut self.online_url).changed() {
                            self.mark_dirty();
                        }
                        ui.label("That file's fingerprint — 64 letters and numbers given by the maker, if they publish one");
                        if ui.text_edit_singleline(&mut self.online_sha)
                            .on_hover_text("This is how the finished copy checks the download is genuine. It is called a SHA-256 fingerprint.")
                            .changed()
                        {
                            self.mark_dirty();
                        }
                    }
                }
                _ => {}
            }

            ui.add_space(8.0);
            ui.checkbox(&mut self.show_details, "Show details")
                .on_hover_text(
                    "Technical details, for people who want them. You never need these to finish.",
                );
            if self.show_details {
                ui.label(
                    egui::RichText::new(step.expert_detail)
                        .color(color(guide::theme::TEXT_FAINT))
                        .monospace(),
                );
            }

            let _ = self.phase();
            });

        // Pinned footer: navigation always in the same place,
        // Continue as the one filled accent button, and the latest
        // plain status line under it.
        egui::TopBottomPanel::bottom("log").show(ctx, |ui| {
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if self.step > 0
                    && ui
                        .add(egui::Button::new("Back").min_size(egui::vec2(110.0, 38.0)))
                        .on_hover_text("Go back one step. Nothing is lost.")
                        .clicked()
                {
                    self.go_back();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let next_label = if self.step + 1 >= STEPS.len() {
                        "Finish"
                    } else {
                        "Continue"
                    };
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new(next_label)
                                    .strong()
                                    .color(color(guide::theme::TEXT)),
                            )
                            .fill(color(guide::theme::PRIMARY_RED))
                            .min_size(egui::vec2(160.0, 38.0)),
                        )
                        .on_hover_text("Go to the next step.")
                        .clicked()
                    {
                        // Each step checks its own promise before letting
                        // the person continue — a plain refusal, never a
                        // trap. Enter on the keyboard takes this same path.
                        self.advance();
                    }
                });
            });
            if let Some(last) = self.log.last() {
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(last.clone())
                        .size(12.5)
                        .color(color(guide::theme::TEXT_FAINT)),
                );
            }
            ui.add_space(8.0);
        });
    }
}

pub fn run_gui() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([880.0, 640.0])
            .with_min_inner_size([640.0, 480.0])
            .with_title("PortableForge"),
        ..Default::default()
    };
    eframe::run_native(
        "PortableForge",
        options,
        Box::new(|_cc| Ok(Box::new(WizardApp::default()))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
        for line in src.lines() {
            let cut = line.find("//").map(|i| &line[..i]).unwrap_or(line);
            cleaned.push_str(cut);
            cleaned.push('\n');
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
            let lower = lit.to_ascii_lowercase();
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
