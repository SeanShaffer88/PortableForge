use std::collections::VecDeque;

use eframe::egui;

use super::{dense, shell, theme, WizardApp};

const HISTORY_LIMIT: usize = 40;
const DETAIL_LIMIT: usize = 6000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Notice {
    Progress(&'static str),
    Problem(&'static str),
}

impl Notice {
    fn text(self) -> &'static str {
        match self {
            Self::Progress(text) | Self::Problem(text) => text,
        }
    }
}

#[derive(Default)]
pub(super) struct Activity {
    details: bool,
    history: VecDeque<Notice>,
    page: usize,
    detail_page: usize,
}

impl Activity {
    fn record(&mut self, notice: Notice) {
        if self.history.back() == Some(&notice) {
            return;
        }
        if self.history.len() == HISTORY_LIMIT {
            self.history.pop_front();
        }
        self.history.push_back(notice);
        self.page = 0;
    }

    fn show(&mut self, ui: &mut egui::Ui, technical: &[&str]) {
        let response = ui.add(
            egui::Label::new(egui::RichText::new("Activity").heading().strong())
                .sense(egui::Sense::focusable_noninteractive()),
        );
        shell::register_region(ui, shell::Region::Activity, response.id);
        theme::focus_outline(ui, &response);
        let toggle = dense::explained(
            ui.button(if self.details {
                "Show progress"
            } else {
                "Technical details"
            }),
            "Show or hide the diagnostic output for this run.",
        );
        if toggle.clicked() {
            self.details = !self.details;
            self.detail_page = 0;
        }
        ui.separator();
        if self.details {
            let report = bounded_details(technical);
            let copy = ui.button("Copy details");
            if copy.clicked() {
                ui.ctx().copy_text(report.clone());
            }
            dense::diagnostic(ui, &report, &mut self.detail_page);
        } else {
            for index in dense::page(ui, &mut self.page, self.history.len(), 3, "Event") {
                let notice = self.history[self.history.len() - index - 1];
                let color = match notice {
                    Notice::Progress(_) => theme::rgb(theme::palette(ui.ctx()).foreground),
                    Notice::Problem(_) => theme::rgb(theme::palette(ui.ctx()).danger),
                };
                ui.add(egui::Label::new(egui::RichText::new(notice.text()).color(color)).wrap());
                ui.separator();
            }
        }
    }
}

fn bounded_details(lines: &[&str]) -> String {
    let mut text = String::new();
    for line in lines.iter().rev().filter(|line| !line.is_empty()) {
        if !text.is_empty() {
            text.push('\n');
        }
        text.extend(
            line.chars()
                .take(DETAIL_LIMIT.saturating_sub(text.chars().count())),
        );
        if text.chars().count() >= DETAIL_LIMIT {
            break;
        }
    }
    if text.is_empty() {
        text.push_str("No technical output has been recorded yet.");
    }
    text
}

impl WizardApp {
    pub(super) fn record_activity(&mut self) {
        let notice = if self.folder_flow.open {
            self.folder_flow.activity_notice()
        } else if self.profile_flow.replay_open {
            self.profile_flow.activity_notice()
        } else if let Some(notice) = self.profile_flow.activity_signal() {
            notice
        } else if self.step == 0 && self.folder_flow.activity_problem().is_some() {
            self.folder_flow.activity_notice()
        } else {
            match self.step {
                0 if self.installer_path.trim().is_empty() => {
                    Notice::Progress("Choose an app to begin.")
                }
                0 => Notice::Progress("Setup selected. Review your source."),
                1 => Notice::Progress("Check how the setup can be run safely."),
                2 => Notice::Progress("Prepare the test box."),
                3..=5 => Notice::Progress("Follow the installation and capture steps."),
                6 => Notice::Progress("Review the captured changes."),
                7 => Notice::Progress("Review the name and app details."),
                8 => Notice::Progress("Confirm which files you can include."),
                _ if self.built_dir.is_some() => Notice::Progress("The portable package is ready."),
                _ if self.profile_flow.profile_saved() => {
                    Notice::Progress("The reviewed profile is saved.")
                }
                _ => Notice::Progress("Review build choices and prepare the package."),
            }
        };
        self.activity.record(notice);
    }

    pub(super) fn show_activity_side(&mut self, ctx: &egui::Context) {
        shell::clear_activity_focus(ctx);
        if !dense::room_for_both_rails(ctx) {
            return;
        }
        egui::SidePanel::right("wizard-activity")
            .resizable(false)
            .exact_width(dense::RAIL_WIDTH)
            .frame(dense::rail_frame(ctx))
            .show(ctx, |ui| self.show_activity_contents(ui));
    }

    fn show_activity_contents(&mut self, ui: &mut egui::Ui) {
        let mut technical: Vec<_> = self.log.iter().rev().take(24).map(String::as_str).collect();
        technical.reverse();
        if let Some(detail) = self.folder_flow.activity_problem() {
            technical.push(detail);
        }
        if let Some(detail) = self.profile_flow.activity_problem() {
            technical.push(detail);
        }
        self.activity.show(ui, &technical);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_and_roadmap_stay_visible_while_the_workspace_scales() {
        use super::super::window_scale::CANVAS;

        for scale in [0.75, 1.0, 1.25] {
            let ctx = egui::Context::default();
            theme::install_fonts(&ctx);
            let mut wizard = WizardApp {
                proportional_window: true,
                ..Default::default()
            };
            wizard.settings.welcomed = true;
            let mut frame = eframe::Frame::_new_kittest();
            let mut draw = |events| {
                let logical = egui::Vec2::from(CANVAS) * (scale / ctx.zoom_factor());
                ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, logical)),
                        events,
                        ..Default::default()
                    },
                    |ctx| eframe::App::update(&mut wizard, ctx, &mut frame),
                )
            };
            for _ in 0..3 {
                draw(vec![]);
            }
            let key = |key, pressed| egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            };
            draw(vec![key(egui::Key::F6, true)]);
            let activity = shell::region_focus(&ctx, shell::Region::Activity);
            assert!(activity.is_some());
            assert_eq!(ctx.memory(|memory| memory.focused()), activity);
            let output = draw(vec![
                key(egui::Key::F6, false),
                key(egui::Key::Escape, true),
            ]);
            assert!((ctx.zoom_factor() - scale).abs() < 0.001);
            assert_eq!(egui::TextStyle::Body.resolve(&ctx.style()).size, 18.0);
            let clip_for = |label: &str| {
                let shape = output.shapes.iter().find(|clipped| {
                    matches!(&clipped.shape, egui::Shape::Text(text) if text.galley.text() == label)
                }).unwrap_or_else(|| panic!("Missing {label} at {scale}"));
                assert!(
                    shape
                        .clip_rect
                        .expand(1.0)
                        .contains_rect(shape.shape.visual_bounding_rect()),
                    "{label} at {scale}"
                );
                shape.clip_rect
            };
            let left = clip_for("Roadmap");
            let right = clip_for("Activity");
            assert!((left.width() - right.width()).abs() < 1.0);
            assert!((left.min.x - (CANVAS[0] - right.max.x)).abs() < 1.0);
            clip_for("Technical details");
            clip_for(portableforge_core::guide::folder_flow::PICK_TITLE);
        }
    }

    #[test]
    fn activity_has_bounded_separate_diagnostics_and_deduplicates_milestones() {
        let mut activity = Activity::default();
        for _ in 0..100 {
            activity.record(Notice::Progress("Inspect the source."));
            activity.record(Notice::Progress("Inspect the source."));
            activity.record(Notice::Problem("Check the suggested fix."));
        }
        assert_eq!(activity.history.len(), HISTORY_LIMIT);
        assert!(activity
            .history
            .iter()
            .all(|event| !event.text().contains("C:\\")));
        let raw = format!("C:\\private\\{}", "λ".repeat(DETAIL_LIMIT));
        assert_eq!(bounded_details(&[&raw]).chars().count(), DETAIL_LIMIT);
        assert_eq!(bounded_details(&["old", "new"]), "new\nold");
    }
}
