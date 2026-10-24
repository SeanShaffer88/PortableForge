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
    review: Review,
    layout: Option<SessionLayout>,
    build_summary: Option<String>,
    log: Vec<String>,
}

impl Default for WizardApp {
    fn default() -> Self {
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
            review: Review::default(),
            layout: None,
            build_summary: None,
            log: vec!["Welcome to PortableForge.".to_string()],
        }
    }
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
    v.panel_fill = color(guide::theme::PANEL);
    v.window_fill = color(guide::theme::PANEL);
    v.extreme_bg_color = color(guide::theme::BG);
    v.faint_bg_color = color(guide::theme::PANEL);
    v.override_text_color = Some(color(guide::theme::TEXT));
    v.hyperlink_color = color(guide::theme::ACCENT_RED);
    v.selection.bg_fill = color(guide::theme::PRIMARY_RED);
    v.widgets.active.bg_fill = color(guide::theme::PRIMARY_RED);
    v.widgets.hovered.bg_fill = color(guide::theme::HAIRLINE);
    v.widgets.inactive.bg_fill = color(guide::theme::HAIRLINE);
    v
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
            let id: String = stem
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                        c
                    } else {
                        '-'
                    }
                })
                .collect();
            let mode = self
                .preflight
                .as_ref()
                .map(|r| r.best_mode)
                .unwrap_or(portableforge_core::wsb::SandboxMode::WindowsSandbox);
            let session = portableforge_core::Session::new(
                if id.is_empty() { "run".to_string() } else { id },
                mode,
            );
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
                false,
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
        let state = portableforge_core::orchestrate::read_agent_state(&layout);
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
        let outcome = portableforge_core::build::build_package(
            &out_root,
            &meta,
            &self.review.accepted(),
            harvest.as_deref(),
            None,
        );
        match outcome {
            Ok(outcome) => {
                let mut text = format!("{}\nMade in: {}", outcome.summary(), out_root.display());
                for issue in &outcome.issues {
                    text.push_str(&format!("\n{}: {}", issue.field, issue.message));
                }
                self.log.push(outcome.summary());
                self.build_summary = Some(text);
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
}

impl eframe::App for WizardApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.set_visuals(dark_visuals());
        let step = &STEPS[self.step.min(STEPS.len() - 1)];
        egui::TopBottomPanel::top("progress").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("Step {} of {}", self.step + 1, STEPS.len()))
                        .color(color(guide::theme::TEXT_MUTED)),
                );
                ui.label(
                    egui::RichText::new("PortableForge")
                        .color(color(guide::theme::ACCENT_RED))
                        .strong(),
                );
            });
            let frac = (self.step as f32 + 1.0) / STEPS.len() as f32;
            ui.add(
                egui::ProgressBar::new(frac)
                    .fill(color(guide::theme::PRIMARY_RED))
                    .show_percentage(),
            );
            ui.add_space(6.0);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(10.0);
            ui.heading(egui::RichText::new(step.title).size(26.0));
            ui.add_space(6.0);
            ui.label(egui::RichText::new(step.what_happens).size(16.0));
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(step.what_you_do)
                    .size(16.0)
                    .color(color(guide::theme::TEXT_MUTED)),
            );
            ui.add_space(12.0);

            // Compute the safety report the first time its step shows.
            if step.id == "safety" && self.preflight.is_none() {
                self.preflight = Some(evaluate_preflight(&crate::collect_preflight_facts()));
            }

            // Per-step intent collection (the core decides what it means).
            match step.id {
                "safety" => {
                    if let Some(report) = &self.preflight {
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
                        }
                        ui.add_space(6.0);
                        let (name, honesty) = report.best_mode.describe();
                        ui.label(format!("Way we will use: {name}"));
                        ui.label(
                            egui::RichText::new(honesty).color(color(guide::theme::TEXT_MUTED)),
                        );
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
                    }
                }
                "pick" => {
                    ui.label("Where is the program's setup file?");
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut self.installer_path)
                            .on_hover_text(
                                "Type or paste the full path of the setup file you downloaded, or use Browse.",
                            );
                        if ui
                            .button("Browse…")
                            .on_hover_text("Open a file chooser to find the setup file.")
                            .clicked()
                        {
                            if let Some(path) = rfd::FileDialog::new()
                                .set_title("Pick the program's setup file")
                                .add_filter("Setup files", &["exe", "msi", "zip"])
                                .add_filter("All files", &["*"])
                                .pick_file()
                            {
                                self.installer_path = path.to_string_lossy().into_owned();
                            }
                        }
                    });
                }
                "clean" => {
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
                }
                "name" => {
                    ui.label("Name for your portable copy");
                    ui.text_edit_singleline(&mut self.app_name);
                    ui.label("Who makes the program");
                    ui.text_edit_singleline(&mut self.publisher);
                    ui.label("The program's main file name (ends in .exe)");
                    ui.text_edit_singleline(&mut self.main_exe)
                        .on_hover_text("The file that starts the program, as it appears inside its folder — for example widget.exe. It was in the list of things the program installed.");
                    ui.label("Version, as four numbers (like 1.0.0.0)");
                    ui.text_edit_singleline(&mut self.package_version);
                    ui.label("A short description");
                    ui.text_edit_multiline(&mut self.description);
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
                        ui.text_edit_singleline(&mut self.online_url);
                        ui.label("That file's fingerprint — 64 letters and numbers given by the maker, if they publish one");
                        ui.text_edit_singleline(&mut self.online_sha)
                            .on_hover_text("This is how the finished copy checks the download is genuine. It is called a SHA-256 fingerprint.");
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

            ui.add_space(16.0);
            ui.horizontal(|ui| {
                if self.step > 0
                    && ui
                        .button("Back")
                        .on_hover_text("Go back one step. Nothing is lost.")
                        .clicked()
                {
                    self.step -= 1;
                }
                let next_label = if self.step + 1 >= STEPS.len() {
                    "Finish"
                } else {
                    "Continue"
                };
                if ui
                    .button(egui::RichText::new(next_label).strong())
                    .on_hover_text("Go to the next step.")
                    .clicked()
                {
                    // Each step checks its own promise before letting the
                    // person continue — a plain refusal, never a trap.
                    let blocked = match STEPS[self.step].id {
                        "pick" => {
                            if self.installer_path.trim().is_empty() {
                                self.log.push(
                                    "Pick the program's setup file first — type its full path above."
                                        .to_string(),
                                );
                                true
                            } else if !std::path::Path::new(self.installer_path.trim()).exists()
                            {
                                self.log.push(format!(
                                    "We could not find a file at {} — check the path and try again.",
                                    self.installer_path.trim()
                                ));
                                true
                            } else {
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
                            if !self.review.items.is_empty()
                                && !self.review.all_acknowledged()
                            {
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
                    if !blocked && self.step + 1 < STEPS.len() {
                        self.step += 1;
                        self.log
                            .push(format!("Moved to: {}", STEPS[self.step].title));
                    }
                }
                let _ = self.phase();
            });
        });

        egui::TopBottomPanel::bottom("log").show(ctx, |ui| {
            if let Some(last) = self.log.last() {
                ui.label(egui::RichText::new(last.clone()).color(color(guide::theme::TEXT_FAINT)));
            }
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
