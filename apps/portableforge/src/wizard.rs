//! The guided wizard (egui, dark only).
//!
//! The GUI renders state and collects intent only — every word it shows
//! and every decision it offers comes from `portableforge_core::guide`
//! and the other core modules. No business rules live here.

use eframe::egui;
use portableforge_core::guide::{self, SharingAnswer, SHARING_ANSWERS, STEPS};
use portableforge_core::orchestrate::{evaluate_preflight, PreflightReport};
use portableforge_core::review::Review;
use portableforge_core::session::Phase;

pub struct WizardApp {
    step: usize,
    show_details: bool,
    installer_path: String,
    app_name: String,
    description: String,
    sharing: Option<SharingAnswer>,
    preflight: Option<PreflightReport>,
    review: Review,
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
            sharing: None,
            preflight: None,
            review: Review::default(),
            log: vec!["Welcome to PortableForge.".to_string()],
        }
    }
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
                    ui.text_edit_singleline(&mut self.installer_path)
                        .on_hover_text(
                            "Type or paste the full path of the setup file you downloaded.",
                        );
                }
                "name" => {
                    ui.label("Name for your portable copy");
                    ui.text_edit_singleline(&mut self.app_name);
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
