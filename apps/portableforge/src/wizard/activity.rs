use std::collections::VecDeque;

use eframe::egui;

use super::{dense, shell, theme, WizardApp};

const PANEL_WIDTH: f32 = 248.0;
const WIDE_MIN: f32 = 1040.0;
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
    open: bool,
    details: bool,
    focus_pending: bool,
    return_focus: bool,
    opener: Option<egui::Id>,
    heading: Option<egui::Id>,
    bounds: Option<egui::Rect>,
    had_focus: bool,
    history: VecDeque<Notice>,
    page: usize,
    detail_page: usize,
}

impl Activity {
    #[cfg(test)]
    pub(super) fn open_for_layout_test(&mut self) {
        self.open = true;
    }

    pub(super) fn wide(ctx: &egui::Context) -> bool {
        ctx.content_rect().width() >= WIDE_MIN
    }

    pub(super) fn compact(&self, ctx: &egui::Context) -> bool {
        self.open && !Self::wide(ctx)
    }

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

    pub(super) fn toggle_button(&mut self, ui: &mut egui::Ui) {
        let label = if self.open {
            "Hide Activity"
        } else {
            "Activity"
        };
        let button = dense::explained(
            ui.button(label),
            "Open or close the record of progress and problems.",
        );
        self.opener = Some(button.id);
        theme::focus_outline(ui, &button);
        if button.clicked() {
            if self.open {
                self.close(ui.ctx());
            } else {
                self.open = true;
                self.focus_pending = true;
                ui.ctx().request_repaint();
            }
        }
    }

    fn close(&mut self, ctx: &egui::Context) {
        self.open = false;
        self.return_focus = true;
        ctx.request_repaint();
    }

    pub(super) fn finish_frame(&mut self, ctx: &egui::Context) {
        if std::mem::take(&mut self.return_focus) {
            if let Some(opener) = self.opener {
                ctx.memory_mut(|memory| memory.request_focus(opener));
            }
        }
        self.had_focus = self.open
            && ctx.memory(|memory| memory.focused()).is_some_and(|id| {
                Some(id) == self.heading
                    || self.bounds.is_some_and(|bounds| {
                        ctx.read_response(id)
                            .is_some_and(|response| bounds.contains(response.rect.center()))
                    })
            });
    }

    pub(super) fn handle_escape(&mut self, ctx: &egui::Context) -> bool {
        if !self.open || ctx.memory(|memory| memory.top_modal_layer().is_some()) {
            return false;
        }
        let can_close = self.had_focus || self.compact(ctx);
        if can_close
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.close(ctx);
            return true;
        }
        false
    }

    pub(super) fn handle_compact_focus(&self, ctx: &egui::Context) {
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::F6)) {
            ctx.memory_mut(|memory| {
                let next = if memory.focused() == self.heading {
                    self.opener
                } else {
                    self.heading
                };
                if let Some(next) = next {
                    memory.request_focus(next);
                }
            });
        }
    }

    fn show(&mut self, ui: &mut egui::Ui, technical: &[&str]) {
        let response = ui.add(
            egui::Label::new(egui::RichText::new("Activity").heading().strong())
                .sense(egui::Sense::focusable_noninteractive()),
        );
        self.bounds = Some(ui.max_rect());
        let previous_heading = self.heading.replace(response.id);
        let heading_moved = previous_heading
            .is_some_and(|id| id != response.id && ui.ctx().memory(|memory| memory.has_focus(id)));
        shell::register_region(ui, shell::Region::Activity, response.id);
        if std::mem::take(&mut self.focus_pending) || heading_moved {
            response.request_focus();
        }
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
            let rows = if self.compact(ui.ctx()) { 5 } else { 3 };
            for index in dense::page(ui, &mut self.page, self.history.len(), rows, "Activity") {
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
        if !self.activity.open || !Activity::wide(ctx) {
            return;
        }
        let visuals = &ctx.style().visuals;
        egui::SidePanel::right("wizard-activity")
            .resizable(false)
            .exact_width(PANEL_WIDTH)
            .frame(
                egui::Frame::new()
                    .fill(visuals.panel_fill)
                    .stroke(visuals.window_stroke)
                    .inner_margin(egui::Margin::same(12)),
            )
            .show(ctx, |ui| self.show_activity_contents(ui));
    }

    pub(super) fn show_activity_compact(&mut self, ctx: &egui::Context) {
        shell::clear_activity_focus(ctx);
        self.show_header(ctx, "", None);
        dense::show_help_bar(ctx, "Choose Hide Activity to return to your work.");
        egui::CentralPanel::default()
            .frame(dense::content_frame(ctx))
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

    fn draw(
        wizard: &mut WizardApp,
        ctx: &egui::Context,
        size: [f32; 2],
        key: Option<egui::Key>,
    ) -> egui::FullOutput {
        let bounds = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::from(size));
        let mut frame = eframe::Frame::_new_kittest();
        ctx.run(
            egui::RawInput {
                screen_rect: Some(bounds),
                events: key
                    .into_iter()
                    .map(|key| egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    })
                    .collect(),
                ..Default::default()
            },
            |ctx| eframe::App::update(wizard, ctx, &mut frame),
        )
    }

    fn text_is_visible(output: &egui::FullOutput, label: &str) -> bool {
        output.shapes.iter().any(|clipped| {
            matches!(&clipped.shape, egui::Shape::Text(text) if text.galley.text() == label)
                && clipped
                    .clip_rect
                    .expand(0.9)
                    .contains_rect(clipped.shape.visual_bounding_rect())
        })
    }

    #[test]
    fn activity_stays_inside_the_window_at_minimum_and_wide_sizes() {
        for (size, compact) in [
            ([640.0, 480.0], true),
            ([WIDE_MIN, 480.0], false),
            ([1160.0, 800.0], false),
        ] {
            let ctx = egui::Context::default();
            theme::install_fonts(&ctx);
            let mut wizard = WizardApp::default();
            wizard.settings.text_scale = 1.5;
            wizard.settings.welcomed = true;
            assert!(!wizard.activity.open);
            wizard.activity.open = true;
            wizard.activity.focus_pending = true;
            draw(&mut wizard, &ctx, size, None);
            draw(&mut wizard, &ctx, size, None);
            let output = draw(&mut wizard, &ctx, size, None);
            assert_eq!(ctx.zoom_factor(), 1.5);
            assert_eq!(wizard.activity.compact(&ctx), compact);
            for label in [
                "Technical details",
                "Hide Activity",
                "Choose an app to begin.",
            ] {
                assert!(text_is_visible(&output, label), "{label} at {size:?}");
            }
            assert_eq!(
                text_is_visible(&output, portableforge_core::guide::folder_flow::PICK_TITLE),
                !compact
            );
            assert_eq!(
                ctx.memory(|memory| memory.focused()),
                wizard.activity.heading
            );
            let output = draw(&mut wizard, &ctx, size, Some(egui::Key::Escape));
            assert!(!wizard.activity.open, "Escape at {size:?}");
            assert!(
                !text_is_visible(&output, "Hide Activity"),
                "Close label at {size:?}"
            );
            assert_eq!(
                ctx.memory(|memory| memory.focused()),
                wizard.activity.opener
            );
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
