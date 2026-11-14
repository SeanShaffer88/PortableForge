//! Compact layouts shared by the wizard and the profile rebuild page.

use eframe::egui;

use super::{guide, picker_start_dir, shell, theme, WizardApp};

pub(super) const MIN_CONTENT: [f32; 2] = [640.0, 480.0];

pub(super) fn content_frame(ctx: &egui::Context) -> egui::Frame {
    let style = ctx.style();
    egui::Frame::central_panel(&style)
        .fill(style.visuals.window_fill)
        .inner_margin(style.spacing.window_margin)
}

pub(super) fn compact_body(ui: &mut egui::Ui) {
    if ui.available_height() < 500.0 {
        ui.spacing_mut().interact_size.y = theme::compact_row_height();
        ui.spacing_mut().item_spacing.y = 4.0;
    }
}

pub(super) fn heading(ui: &mut egui::Ui, title: &str, description: &str) {
    let heading = ui.add(
        egui::Label::new(egui::RichText::new(title).heading().strong())
            .sense(egui::Sense::focusable_noninteractive()),
    );
    shell::register_heading(ui, &heading);
    if !description.is_empty() {
        ui.label(egui::RichText::new(description).weak());
    }
}

pub(super) fn primary_action(ui: &mut egui::Ui, enabled: bool, label: &str) -> egui::Response {
    let palette = theme::palette(ui.ctx());
    let response = ui
        .scope(|ui| {
            let visuals = &mut ui.style_mut().visuals;
            for (widget, fill) in [
                (&mut visuals.widgets.inactive, palette.action),
                (&mut visuals.widgets.hovered, palette.action_hover),
                (&mut visuals.widgets.active, palette.action_pressed),
                (&mut visuals.widgets.open, palette.action_pressed),
            ] {
                widget.bg_fill = theme::rgb(fill);
                widget.weak_bg_fill = theme::rgb(fill);
                widget.bg_stroke = egui::Stroke::NONE;
            }
            ui.add_enabled(
                enabled,
                egui::Button::new(
                    egui::RichText::new(label)
                        .strong()
                        .color(theme::rgb(palette.action_foreground)),
                ),
            )
        })
        .inner;
    theme::focus_outline(ui, &response);
    response
}

pub(super) fn width_with_button(ui: &egui::Ui, label: &str) -> f32 {
    (ui.available_width() - button_width(ui, label) - 12.0).max(70.0)
}

pub(super) fn button_width(ui: &egui::Ui, label: &str) -> f32 {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let label_width = ui.fonts_mut(|fonts| {
        fonts
            .layout_no_wrap(label.into(), font, egui::Color32::WHITE)
            .size()
            .x
    });
    label_width + 2.0 * ui.spacing().button_padding.x + ui.spacing().item_spacing.x
}

#[derive(Clone, PartialEq)]
struct HelpTarget {
    id: egui::Id,
    rect: egui::Rect,
    message: String,
}

#[derive(Clone, Default)]
struct HelpTargets {
    hovered: Option<HelpTarget>,
    focused: Option<HelpTarget>,
}

#[derive(Clone, Default)]
struct HelpState {
    pass: u64,
    previous: HelpTargets,
    current: HelpTargets,
}

fn help_key(modal: bool) -> egui::Id {
    egui::Id::new(("wizard-context-help", modal))
}

pub(super) fn clear_help_targets(ctx: &egui::Context) {
    ctx.data_mut(|data| data.remove::<HelpState>(help_key(false)));
    ctx.request_repaint();
}

fn help_targets(ctx: &egui::Context, modal: bool, update: impl FnOnce(&mut HelpState)) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|data| {
        let state = data.get_temp_mut_or_default::<HelpState>(help_key(modal));
        if state.pass != pass {
            state.previous = if state.pass + 1 == pass {
                std::mem::take(&mut state.current)
            } else {
                HelpTargets::default()
            };
            state.pass = pass;
            state.current = HelpTargets::default();
        }
        update(state);
    });
}

pub(super) fn field_help(response: &egui::Response, help: &str) {
    response.ctx.accesskit_node_builder(response.id, |node| {
        node.set_description(help);
    });
    let hovered = response.contains_pointer();
    let focused = response.has_focus();
    if !hovered && !focused {
        return;
    }
    let modal = response.layer_id.order == egui::Order::Foreground;
    let mut repaint = false;
    help_targets(&response.ctx, modal, |state| {
        for (active, current, previous) in [
            (hovered, &mut state.current.hovered, &state.previous.hovered),
            (focused, &mut state.current.focused, &state.previous.focused),
        ] {
            if active {
                let target = HelpTarget {
                    id: response.id,
                    rect: response.interact_rect,
                    message: help.to_owned(),
                };
                repaint |= previous.as_ref() != Some(&target);
                *current = Some(target);
            }
        }
    });
    if repaint {
        response.ctx.request_repaint();
    }
}

pub(super) fn explained(response: egui::Response, help: impl AsRef<str>) -> egui::Response {
    field_help(&response, help.as_ref());
    response
}

pub(super) fn help_strip(ui: &mut egui::Ui, default: &str) {
    let modal = ui.layer_id().order == egui::Order::Foreground;
    let (focused, modal_open) =
        ui.memory(|memory| (memory.focused(), memory.top_modal_layer().is_some()));
    let pointer = ui.input(|input| input.pointer.hover_pos());
    let mut message = None;
    if modal || !modal_open {
        help_targets(ui.ctx(), modal, |state| {
            let active = &state.current;
            let previous = &state.previous;
            let previous_hover = previous
                .hovered
                .as_ref()
                .filter(|target| pointer.is_some_and(|position| target.rect.contains(position)));
            let previous_focus = previous
                .focused
                .as_ref()
                .filter(|target| focused == Some(target.id));
            message = active
                .hovered
                .as_ref()
                .or(previous_hover)
                .or(active.focused.as_ref())
                .or(previous_focus)
                .map(|target| target.message.clone());
        });
    }
    let text = message.as_deref().unwrap_or(default);
    let font = egui::TextStyle::Small.resolve(ui.style());
    let line = ui.fonts_mut(|fonts| fonts.row_height(&font));
    ui.set_min_height(line * 2.0);
    let color = ui.visuals().weak_text_color();
    let mut layout =
        egui::text::LayoutJob::simple(text.to_owned(), font, color, ui.available_width());
    layout.wrap.max_rows = 2;
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(layout));
    let label = ui.label(galley);
    ui.ctx().accesskit_node_builder(label.id, |node| {
        node.set_role(egui::accesskit::Role::Status)
    });
}

pub(super) fn show_help_bar(ctx: &egui::Context, default: &str) {
    let visuals = &ctx.style().visuals;
    egui::TopBottomPanel::bottom("wizard-context-help")
        .resizable(false)
        .frame(
            egui::Frame::new()
                .fill(visuals.window_fill)
                .stroke(visuals.widgets.noninteractive.bg_stroke)
                .inner_margin(egui::Margin::symmetric(12, 4)),
        )
        .show(ctx, |ui| help_strip(ui, default));
}

pub(super) fn explained_text_field(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut String,
    help: &str,
) -> bool {
    let heading = explained(ui.label(label), help);
    let input = ui
        .add_sized(
            [ui.available_width(), ui.spacing().interact_size.y],
            egui::TextEdit::singleline(value),
        )
        .labelled_by(heading.id);
    field_help(&input, help);
    input.changed()
}

pub(super) fn room_for_sidebar(ctx: &egui::Context) -> bool {
    ctx.content_rect().width() >= 768.0
}

pub(super) fn task_sidebar(ctx: &egui::Context, show: impl FnOnce(&mut egui::Ui)) {
    shell::clear_sidebar_focus(ctx);
    if !room_for_sidebar(ctx) {
        return;
    }
    let palette = theme::palette(ctx);
    let frame = egui::Frame::new()
        .fill(theme::rgb(palette.background))
        .inner_margin(egui::Margin::same(8));
    egui::SidePanel::left("wizard-roadmap")
        .exact_width(136.0)
        .resizable(false)
        .frame(frame)
        .show(ctx, |ui| {
            ui.label(egui::RichText::new("Roadmap").strong());
            ui.add_space(4.0);
            show(ui);
        });
}

pub(super) fn mark_roadmap_selection(response: &egui::Response, selected: bool) {
    if selected {
        response.ctx.accesskit_node_builder(response.id, |node| {
            node.set_selected(true);
        });
    }
}

pub(super) fn roadmap_item(
    ui: &mut egui::Ui,
    label: &str,
    selected: bool,
    enabled: bool,
    help: &str,
) -> egui::Response {
    let response = ui.add_enabled(
        enabled,
        egui::Button::selectable(selected, label)
            .min_size(egui::vec2(
                ui.available_width(),
                theme::compact_row_height(),
            ))
            .wrap(),
    );
    mark_roadmap_selection(&response, selected);
    if selected {
        let left = response.rect.left() + 1.0;
        ui.painter().vline(
            left,
            response.rect.y_range().shrink(4.0),
            ui.visuals().selection.stroke,
        );
    }
    theme::focus_outline(ui, &response);
    field_help(&response, help);
    response
}

pub(super) fn review_counts(review: &portableforge_core::review::Review) -> String {
    use portableforge_core::review::Decision;

    let (mut selected, mut excluded, mut pending) = (0, 0, 0);
    for item in &review.items {
        match item.decision {
            Decision::Fix { on: true } => selected += 1,
            Decision::Fix { on: false } => excluded += 1,
            Decision::Acknowledge { seen: false } => pending += 1,
            _ => {}
        }
    }
    let mut counts = format!("{selected} selected · {excluded} excluded");
    if pending > 0 {
        counts.push_str(&format!(" · {pending} to acknowledge"));
    }
    counts
}

pub(super) fn page(
    ui: &mut egui::Ui,
    page: &mut usize,
    total: usize,
    per_page: usize,
    item: &str,
) -> std::ops::Range<usize> {
    let pages = total.div_ceil(per_page);
    *page = (*page).min(pages.saturating_sub(1));
    if pages > 1 {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(*page > 0, egui::Button::new("‹ Previous"))
                .clicked()
            {
                *page -= 1;
            }
            let first = *page * per_page + 1;
            let last = (first + per_page - 1).min(total);
            ui.label(if first == last {
                format!("{item} {first} of {total}")
            } else {
                format!("{item} {first}–{last} of {total}")
            });
            if ui
                .add_enabled(*page + 1 < pages, egui::Button::new("Next ›"))
                .clicked()
            {
                *page += 1;
            }
        });
    }
    let first = *page * per_page;
    first..(first + per_page).min(total)
}

/// Break variable-length diagnostics into screen-sized line groups.
pub(super) fn diagnostic(ui: &mut egui::Ui, text: &str, selected: &mut usize) {
    diagnostic_in_height(ui, text, selected, ui.available_height());
}

fn diagnostic_in_height(ui: &mut egui::Ui, text: &str, selected: &mut usize, height: f32) {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let galley = ui.fonts_mut(|fonts| {
        fonts.layout(
            text.to_owned(),
            font,
            ui.visuals().text_color(),
            ui.available_width(),
        )
    });
    let line_height = galley.rows.first().map_or(20.0, |row| row.rect().height());
    let rows = ((height - 36.0) / line_height).floor().max(1.0) as usize;
    let range = page(ui, selected, galley.rows.len(), rows, "Line");
    let start = galley.rows[..range.start]
        .iter()
        .map(|row| row.char_count_including_newline())
        .sum();
    let count = galley.rows[range]
        .iter()
        .map(|row| row.char_count_including_newline())
        .sum();
    let text: String = text.chars().skip(start).take(count).collect();
    ui.label(text);
}

pub(super) fn path_needs_view(ui: &egui::Ui, path: &str, field_width: f32) -> bool {
    let font = egui::TextStyle::Body.resolve(ui.style());
    ui.fonts_mut(|fonts| {
        fonts
            .layout_no_wrap(path.into(), font, egui::Color32::WHITE)
            .size()
            .x
            > field_width - 18.0
    })
}

pub(super) fn path_edit(
    ui: &mut egui::Ui,
    value: &mut String,
    width: f32,
) -> (egui::Response, bool) {
    let mut layout = |ui: &egui::Ui, text: &dyn egui::TextBuffer, width: f32| {
        let font = egui::TextStyle::Monospace.resolve(ui.style());
        let mut job = egui::text::LayoutJob::simple(
            text.as_str().to_owned(),
            font,
            ui.visuals().text_color(),
            width,
        );
        job.wrap.break_anywhere = true;
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };
    let rows = layout(ui, value, (width - 8.0).max(1.0)).rows.len();
    let view = rows > 2;
    let field_width = if view {
        width - button_width(ui, "View path")
    } else {
        width
    };
    let editor = if rows == 2 {
        egui::TextEdit::multiline(value)
            .desired_rows(2)
            .return_key(None)
            .lock_focus(false)
            .layouter(&mut layout)
    } else {
        egui::TextEdit::singleline(value)
    };
    let response = ui.add_sized(
        [field_width, ui.spacing().interact_size.y],
        editor
            .font(egui::TextStyle::Monospace)
            .desired_width(field_width),
    );
    if response.changed() {
        value.retain(|c| !matches!(c, '\r' | '\n' | '\t'));
    }
    (response, view)
}

pub(super) fn view_text(ui: &mut egui::Ui, action: &str, title: &str, text: &str, offered: bool) {
    let id = ui.make_persistent_id(("text-details", title));
    let mut selected = ui.data(|data| data.get_temp::<usize>(id));
    if offered && ui.button(action).clicked() {
        selected = Some(0);
    }
    if let Some(mut page) = selected {
        let ctx = ui.ctx();
        let viewport = ctx.content_rect().size();
        let response = egui::Modal::new(id).show(ctx, |ui| {
            ui.set_width((viewport.x - 60.0).min(720.0));
            ui.heading(title);
            diagnostic_in_height(ui, text, &mut page, viewport.y - 135.0);
            ui.horizontal(|ui| {
                if ui.button("Copy full text").clicked() {
                    ctx.copy_text(text.to_owned());
                }
                ui.button("Close").clicked()
            })
            .inner
        });
        let close = response.inner || response.should_close();
        ui.data_mut(|data| {
            if close {
                data.remove::<usize>(id);
            } else {
                data.insert_temp(id, page);
            }
        });
    }
}

impl WizardApp {
    pub(super) fn apply_layout_zoom(&mut self, ctx: &egui::Context) {
        let scale = ctx.zoom_factor();
        if self.applied_layout_zoom != scale {
            self.applied_layout_zoom = scale;
            let minimum = egui::Vec2::from(MIN_CONTENT);
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(minimum));
            let current = ctx.content_rect().size();
            if current.x < minimum.x || current.y < minimum.y {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(current.max(minimum)));
            }
        }
        ctx.set_zoom_factor(self.settings.text_scale());
    }

    pub(super) fn show_header(
        &mut self,
        ctx: &egui::Context,
        section: &str,
        step: Option<(usize, usize)>,
    ) {
        shell::clear_sidebar_focus(ctx);
        let visuals = &ctx.style().visuals;
        let frame = egui::Frame::new()
            .fill(visuals.window_fill)
            .stroke(visuals.widgets.noninteractive.bg_stroke)
            .inner_margin(egui::Margin::symmetric(12, 4));
        egui::TopBottomPanel::top("wizard-header")
            .frame(frame)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("PortableForge").strong());
                    if step.is_none() && (!self.folder_flow.open || room_for_sidebar(ctx)) {
                        ui.label(egui::RichText::new(section).small().weak());
                    }
                    if step.is_none() && self.folder_flow.open && !room_for_sidebar(ctx) {
                        self.folder_flow.show_compact_roadmap(ui);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let appearance = explained(
                            ui.button("Appearance"),
                            "Adjust the theme or change the text size. Shortcut: Ctrl+comma.",
                        );
                        if appearance.clicked() {
                            self.open_shell_popup(shell::Popup::Appearance, Some(appearance.id));
                        }
                        let commands = explained(
                            ui.button("Commands · Ctrl+K"),
                            "Search for an action available in the current step.",
                        );
                        shell::register_region(ui, shell::Region::Header, commands.id);
                        if commands.clicked() {
                            self.open_shell_popup(shell::Popup::Commands, Some(commands.id));
                        }
                    });
                });
                if let Some((index, total)) = step.filter(|_| !room_for_sidebar(ctx)) {
                    self.show_phase_navigation(ui, index, total, false);
                }
            });
    }

    pub(super) fn show_navigation(&mut self, ctx: &egui::Context) {
        let visuals = &ctx.style().visuals;
        let frame = egui::Frame::new()
            .fill(visuals.window_fill)
            .stroke(visuals.widgets.noninteractive.bg_stroke)
            .inner_margin(egui::Margin::symmetric(12, 4));
        egui::TopBottomPanel::bottom("wizard-actions")
            .frame(frame)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let back = ui.add_enabled(
                        self.can_go_back() && !self.show_details,
                        egui::Button::new("Back"),
                    );
                    field_help(&back, "Go to the previous guided step. Changes you entered stay in this project.");
                    if back.clicked() {
                        self.go_back();
                        ctx.request_repaint();
                    }
                    let details = ui
                        .add_enabled(
                            !self.profile_flow.busy(),
                            egui::Button::new(if self.show_details {
                                "Back to step"
                            } else {
                                "Status and details"
                            }),
                        );
                    field_help(&details, "Read the current status or troubleshooting details for this step. Shortcut: F1.");
                    if details.clicked() {
                        self.toggle_step_details();
                        ctx.request_repaint();
                    }
                    let action = self.current_wizard_action();
                    if matches!(action, Some((shell::WizardAction::BuildInstaller, _)))
                        && ui.button("Finish").clicked()
                    {
                        self.advance();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some((action, ready)) = action {
                            let response = primary_action(ui, ready, action.label());
                            field_help(&response, if ready {
                                "Confirm this step and continue the guided workflow."
                            } else {
                                "Complete the required choices in this step before continuing."
                            });
                            shell::register_region(
                                ui,
                                shell::Region::Footer,
                                if ready { response.id } else { details.id },
                            );
                            if response.clicked() {
                                self.run_wizard_action(ctx, action);
                            }
                        } else {
                            shell::register_region(ui, shell::Region::Footer, details.id);
                        }
                    });
                });
            });
    }

    pub(super) fn context_help_default<'a>(&'a self, hint: &'a str) -> &'a str {
        let status = self
            .log
            .last()
            .filter(|line| {
                !self.show_details
                    && !self.profile_flow.busy()
                    && !line.starts_with("Moved to: ")
                    && line.as_str() != "Welcome to PortableForge."
            })
            .map(String::as_str)
            .filter(|line| !line.trim().is_empty())
            .unwrap_or(hint);
        if status.is_empty() {
            "Move the pointer over a control or press Tab to read what it does."
        } else {
            status
        }
    }

    pub(super) fn show_step_details(&mut self, ui: &mut egui::Ui, step: &guide::GuideStep) {
        let detail = if self.use_verified_capture() {
            match step.id {
                "clean" | "changes" => "Sandboxie-Plus capture verifies setup containment and box cleanup before review.",
                "name" | "sharing" => "App details and redistribution permission apply to this reviewed profile.",
                _ => "Rebuild verifies the setup SHA-256, the saved profile and its separate approval.",
            }
        } else {
            step.expert_detail
        };
        heading(ui, "Status and details", detail);
        let log = self.run_log_text();
        diagnostic(ui, &log, &mut self.detail_page);
        if ui.button("Save the run log").clicked() {
            self.export_run_log();
        }
    }

    pub(super) fn show_icon_field(&mut self, ui: &mut egui::Ui) -> bool {
        let label = ui.label("Program picture (optional .ico file)");
        let mut changed = false;
        ui.horizontal(|ui| {
            changed = ui
                .add(
                    egui::TextEdit::singleline(&mut self.icon_path)
                        .desired_width(width_with_button(ui, "Browse…")),
                )
                .labelled_by(label.id)
                .changed();
            if ui.button("Browse…").clicked() {
                let mut dialog = rfd::FileDialog::new()
                    .set_title("Pick the program's picture file")
                    .add_filter("Picture files", &["ico"]);
                if let Some(dir) = picker_start_dir(&self.icon_path) {
                    dialog = dialog.set_directory(dir);
                }
                if let Some(path) = dialog.pick_file() {
                    self.icon_path = path.display().to_string();
                    changed = true;
                }
            }
        });
        if !self.icon_path.trim().is_empty()
            && !std::path::Path::new(self.icon_path.trim()).is_file()
        {
            ui.colored_label(
                ui.visuals().error_fg_color,
                "The picture file could not be found.",
            );
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use portableforge_core::classify::{Classification, Finding, Severity};
    use portableforge_core::review::Review;
    use portableforge_core::settings::{SavedProfileApproval, TEXT_SCALES};

    #[test]
    fn full_path_modal_renders_without_reentering_egui_memory() {
        let ctx = egui::Context::default();
        let path = format!(r"C:\Setups\{}\app.exe", "folder\\".repeat(100));
        let draw = || {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::Vec2::from(MIN_CONTENT),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let id = ui.make_persistent_id(("text-details", "Setup path"));
                        ui.data_mut(|data| data.insert_temp(id, 0usize));
                        view_text(ui, "View path", "Setup path", &path, true);
                    });
                },
            )
        };
        draw();
        let output = draw();
        assert!(output.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "Copy full text")
        }));
    }

    fn assert_page_fits(scale: f32, mut wizard: WizardApp, expected: &[&str]) {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let mut frame = eframe::Frame::_new_kittest();
        assert_wizard_fits(&ctx, &mut frame, &mut wizard, scale, expected);
    }

    fn assert_wizard_fits(
        ctx: &egui::Context,
        frame: &mut eframe::Frame,
        wizard: &mut WizardApp,
        scale: f32,
        expected: &[&str],
    ) {
        assert_wizard_fits_at(ctx, frame, wizard, scale, MIN_CONTENT, expected);
    }

    fn assert_wizard_fits_at(
        ctx: &egui::Context,
        frame: &mut eframe::Frame,
        wizard: &mut WizardApp,
        scale: f32,
        size: [f32; 2],
        expected: &[&str],
    ) {
        wizard.settings.text_scale = scale;
        let bounds = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::from(size));
        let mut draw = || {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(bounds),
                    ..Default::default()
                },
                |ctx| eframe::App::update(wizard, ctx, frame),
            )
        };
        draw();
        draw();
        let output = draw();
        assert_eq!(ctx.zoom_factor(), scale);
        for label in expected {
            let found = output.shapes.iter().find(|clipped| {
                matches!(&clipped.shape, egui::Shape::Text(text) if text.galley.text() == *label)
            });
            let shape = found.unwrap_or_else(|| {
                let shown: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|clipped| {
                        if let egui::Shape::Text(text) = &clipped.shape {
                            Some(text.galley.text())
                        } else {
                            None
                        }
                    })
                    .collect();
                panic!("Missing {label:?} at {scale}, shown: {shown:?}")
            });
            let visible = bounds.intersect(shape.clip_rect).expand(1.0);
            assert!(
                visible.contains_rect(shape.shape.visual_bounding_rect()),
                "{label:?} clipped at {scale}: {:?} versus {visible:?}",
                shape.shape.visual_bounding_rect(),
            );
        }
    }

    #[test]
    fn setup_roadmap_and_grouped_fields_fit_at_the_sidebar_threshold() {
        for scale in TEXT_SCALES {
            let ctx = egui::Context::default();
            theme::install_fonts(&ctx);
            let mut frame = eframe::Frame::_new_kittest();
            let mut wizard = WizardApp::default();
            wizard.enter_verified_capture();
            wizard.step = 7;
            assert_wizard_fits_at(
                &ctx,
                &mut frame,
                &mut wizard,
                scale,
                [768.0, 480.0],
                &[
                    "Roadmap",
                    "Capture",
                    "Review",
                    "Save profile",
                    "Build",
                    "Step 8 of 12",
                    "Portable app name",
                    "Publisher",
                    "Main app file (.exe)",
                    "App description",
                    "Continue",
                ],
            );
        }
    }

    #[test]
    fn focused_configuration_help_wraps_inside_fixed_bar_at_minimum_size() {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        ctx.set_zoom_factor(1.5);
        let bounds = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::from(MIN_CONTENT));
        let help = "Confirm that the app’s license or publisher lets you distribute these files in your installer. PortableForge needs your confirmation before it can include them. Check the publisher’s website when the license is unclear.";
        let mut input = String::new();
        let mut draw = || {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(bounds),
                    ..Default::default()
                },
                |ctx| {
                    show_help_bar(ctx, "Default help");
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let field = ui.text_edit_singleline(&mut input);
                        field.request_focus();
                        field_help(&field, help);
                    });
                },
            )
        };
        draw();
        draw();
        let output = draw();
        let text = output.shapes.iter().find(|shape| {
            matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == help)
        }).expect("keyboard focus updates the fixed help bar");
        assert!(bounds
            .intersect(text.clip_rect)
            .expand(1.0)
            .contains_rect(text.shape.visual_bounding_rect()));
        assert!(text.shape.visual_bounding_rect().top() > bounds.bottom() - 55.0);
        assert!(
            matches!(&text.shape, egui::Shape::Text(text) if !text.galley.elided && text.galley.rows.len() > 1)
        );
    }

    #[test]
    fn hover_help_overrides_focus_then_clears_when_pointer_leaves() {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let bounds = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::from(MIN_CONTENT));
        let mut pointer = None;
        let draw = |pointer: Option<egui::Pos2>| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(bounds),
                    events: pointer.map(egui::Event::PointerMoved).into_iter().collect(),
                    ..Default::default()
                },
                |ctx| {
                    show_help_bar(ctx, "Default help");
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let focused = ui.button("Focus target");
                        focused.request_focus();
                        field_help(&focused, "Focused explanation");
                        let hovered = ui.button("Hover target");
                        field_help(&hovered, "Hovered explanation");
                    });
                },
            )
        };
        draw(pointer);
        let output = draw(pointer);
        let hover_rect = output.shapes.iter().find(|shape| {
            matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "Hover target")
        }).expect("hover target").shape.visual_bounding_rect();
        pointer = Some(hover_rect.center());
        draw(pointer);
        let hovered = draw(pointer);
        assert!(hovered.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "Hovered explanation")
        }));
        let departed = draw(Some(egui::pos2(bounds.right() - 1.0, 200.0)));
        assert!(departed.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "Focused explanation")
        }));
        assert!(!departed.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "Hovered explanation")
        }));
    }

    #[test]
    fn text_size_changes_keep_capture_controls_in_bounds() {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let mut frame = eframe::Frame::_new_kittest();
        let mut wizard = WizardApp::default();
        wizard.enter_verified_capture();
        for scale in [1.0, 1.25, 1.5, 1.0] {
            assert_wizard_fits(
                &ctx,
                &mut frame,
                &mut wizard,
                scale,
                &[
                    "Setup file",
                    "Installed app path (optional)",
                    "I trust this setup file and the installed app.",
                    "Start capture",
                ],
            );
        }
    }

    #[test]
    fn first_run_and_resume_actions_fit_together() {
        let mut wizard = WizardApp {
            resume_offer: true,
            ..Default::default()
        };
        wizard.settings.welcomed = false;
        wizard.settings.recent_installers = vec![
            r"C:\Setups\first.exe".into(),
            r"C:\Setups\second.exe".into(),
        ];
        assert_page_fits(
            1.5,
            wizard,
            &[
                "Pick up where I left off",
                "About this wizard",
                portableforge_core::guide::folder_flow::CHOOSE_FOLDER,
                "Capture with Sandboxie-Plus",
                "Open saved profiles",
                "first.exe",
                "Continue",
            ],
        );
    }

    #[test]
    fn prerequisite_results_fit_at_extra_large() {
        let mut wizard = WizardApp::default();
        wizard.enter_verified_capture();
        wizard.profile_flow.populate_prerequisites_fixture();
        assert_page_fits(
            1.5,
            wizard,
            &[
                "Check prerequisites",
                "Sandboxie-Plus 1.18.8 · Needs attention",
                "View prerequisite details",
                "Open bundled test box folder",
                "Portable test box instructions",
                "Start capture",
            ],
        );
    }

    #[test]
    fn three_wrapped_paths_are_visible_at_extra_large() {
        let mut wizard = WizardApp::default();
        wizard.profile_flow.replay_open = true;
        wizard.settings.saved_profiles = vec![SavedProfileApproval {
            folder: r"C:\Profiles\Fixture".into(),
            label: "Fixture".into(),
            manifest_sha256: "a".repeat(64),
            details: Default::default(),
        }];
        let mut paths = ["setup.exe", "PortableAppsInstaller", "FixtureInstaller"]
            .map(|name| format!(r"C:\{}\{name}", "Portable Forge Fixture\\".repeat(4)));
        paths[0] = r"C:\Users\Avery Parker\Documents\Codex\2026-11-05\h\work\sbie-profile-fixture\fixture-setup.exe".into();
        wizard.profile_flow.set_layout_paths(&paths);
        assert_page_fits(
            1.5,
            wizard,
            &[&paths[0], &paths[1], &paths[2], "Build installer"],
        );
    }

    #[test]
    fn compact_verified_pages_fit_at_every_text_preset() {
        for scale in TEXT_SCALES {
            let mut capture = WizardApp::default();
            capture.enter_verified_capture();
            assert_page_fits(
                scale,
                capture,
                &[
                    "Setup file",
                    "I trust this setup file and the installed app.",
                    "Start capture",
                ],
            );

            let mut review = WizardApp::default();
            review.enter_verified_capture();
            review.step = 6;
            review.setup_findings_open = true;
            review.review = Review::from_classifications(
                (0..8)
                    .map(|i| Classification {
                        finding: Finding::RegistryKey {
                            key: format!(r"HKEY_CURRENT_USER\Software\Fixture\Item{i}"),
                        },
                        severity: Severity::Review,
                        confidence: 1.0,
                        why: "Fixture setting".into(),
                    })
                    .collect(),
            );
            assert_page_fits(
                scale,
                review,
                &[
                    "Review captured changes",
                    "‹ Previous",
                    "Next ›",
                    r"HKEY_CURRENT_USER\Software\Fixture\Item0",
                    "Continue",
                ],
            );

            for (source, verified, next) in [
                (
                    "Sample Setup.exe",
                    true,
                    "Next · App Details → Sharing → Save Profile",
                ),
                (
                    "Sample Setup.msi",
                    false,
                    "Next · App Details → Sharing → Build Package Folder",
                ),
            ] {
                let mut overview = WizardApp::default();
                if verified {
                    overview.enter_verified_capture();
                }
                overview.step = 6;
                overview.installer_path = format!(r"C:\{}\{source}", "Long Folder ".repeat(12));
                overview.review = Review::from_classifications(vec![Classification {
                    finding: Finding::RegistryKey {
                        key: "Fixture".into(),
                    },
                    severity: Severity::Review,
                    confidence: 1.0,
                    why: "Fixture setting".into(),
                }]);
                let row = format!("Setup · {source}");
                assert_page_fits(
                    scale,
                    overview,
                    &[
                        "Review Your Package",
                        &row,
                        "View Source",
                        "Captured Findings (1)",
                        next,
                        "Review Findings",
                    ],
                );
            }

            let mut name = WizardApp::default();
            name.enter_verified_capture();
            name.step = 7;
            assert_page_fits(
                scale,
                name,
                &[
                    "Portable app name",
                    "Publisher",
                    "App homepage (HTTP or HTTPS)",
                    "App description",
                    "Continue",
                ],
            );

            let mut save = WizardApp::default();
            save.enter_verified_capture();
            save.step = 9;
            assert_page_fits(
                scale,
                save,
                &[
                    "PortableApps.com Launcher Generator folder (2.2.9)",
                    "New profile folder",
                    "Save profile",
                ],
            );

            for (busy, feedback) in [
                (true, "Building your installer…"),
                (false, "View error details"),
            ] {
                let mut replay = WizardApp::default();
                replay.profile_flow.replay_open = true;
                replay.settings.saved_profiles = vec![SavedProfileApproval {
                    folder: r"C:\Profiles\Fixture".into(),
                    label: "Fixture".into(),
                    manifest_sha256: "a".repeat(64),
                    details: Default::default(),
                }];
                let finish = replay.profile_flow.populate_layout_fixture(busy);
                assert_page_fits(
                    scale,
                    replay,
                    &[
                        "Profile",
                        "Matching setup file",
                        "New build folder",
                        "View path",
                        "Build installer",
                        feedback,
                    ],
                );
                finish();
            }
        }
    }
}
