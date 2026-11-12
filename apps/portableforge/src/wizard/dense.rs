//! Compact layouts shared by the wizard and the profile rebuild page.

use eframe::egui;

use super::{color, guide, picker_start_dir, WizardApp};

pub(super) const MIN_CONTENT: [f32; 2] = [640.0, 480.0];

pub(super) fn content_frame(ctx: &egui::Context) -> egui::Frame {
    egui::Frame::central_panel(&ctx.style()).inner_margin(egui::Margin::symmetric(16, 8))
}

pub(super) fn heading(ui: &mut egui::Ui, title: &str, description: &str) {
    ui.heading(title);
    ui.label(egui::RichText::new(description).color(color(guide::theme::TEXT_MUTED)));
}

pub(super) fn primary(label: &str) -> egui::Button<'_> {
    egui::Button::new(
        egui::RichText::new(label)
            .strong()
            .color(color(guide::theme::TEXT)),
    )
    .fill(color(super::CRIMSON))
    .min_size(egui::vec2(150.0, 30.0))
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

pub(super) fn text_field(ui: &mut egui::Ui, label: &str, value: &mut String) -> bool {
    let heading = ui.label(label);
    ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY))
        .labelled_by(heading.id)
        .changed()
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
            color(guide::theme::TEXT),
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
        let font = egui::TextStyle::Body.resolve(ui.style());
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
    let response = ui.add(editor.desired_width(field_width));
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
        egui::TopBottomPanel::top("wizard-header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("PortableForge")
                        .strong()
                        .color(color(super::ALERT_TEXT)),
                );
                if let Some((index, total)) = step {
                    ui.label(egui::RichText::new(format!("Step {index} of {total}")).small());
                } else {
                    ui.label(egui::RichText::new(section).small());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    for (label, value) in
                        [("Extra large", 1.5f32), ("Large", 1.25), ("Standard", 1.0)]
                    {
                        if ui
                            .selectable_label(self.settings.text_scale() == value, label)
                            .on_hover_text("Set the wizard text size.")
                            .clicked()
                        {
                            self.settings.text_scale = value;
                            self.settings_dirty = true;
                        }
                    }
                });
            });
            if let Some((index, total)) = step {
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 2.0),
                    egui::Sense::hover(),
                );
                let mut fill = rect;
                fill.max.x = rect.min.x + rect.width() * index as f32 / total as f32;
                ui.painter()
                    .rect_filled(rect, 0, color(guide::theme::HAIRLINE));
                ui.painter().rect_filled(fill, 0, color(super::CRIMSON));
            }
        });
    }

    pub(super) fn show_navigation(&mut self, ctx: &egui::Context, step: &str, verified: bool) {
        egui::TopBottomPanel::bottom("wizard-actions").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if self.step > 0
                    && ui
                        .add_enabled(
                            self.can_go_back() && !self.show_details,
                            egui::Button::new("Back").min_size(egui::vec2(68.0, 30.0)),
                        )
                        .clicked()
                {
                    self.go_back();
                }
                if ui
                    .add_enabled(
                        !self.profile_flow.busy(),
                        egui::Button::new(if self.show_details {
                            "Back to step"
                        } else {
                            "Status and details"
                        }),
                    )
                    .clicked()
                {
                    self.detail_page = 0;
                    self.show_details = !self.show_details;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (label, ready) = if self.show_details {
                        ("", false)
                    } else if verified && step == "clean" && !self.profile_flow.captured {
                        ("Start capture", self.capture_ready())
                    } else if verified && step == "build" && !self.profile_flow.profile_saved() {
                        ("Save profile", self.profile_save_ready())
                    } else if self.step + 1 == guide::STEPS.len() {
                        ("Finish", self.can_advance())
                    } else {
                        ("Continue", self.can_advance())
                    };
                    if !label.is_empty() && ui.add_enabled(ready, primary(label)).clicked() {
                        match label {
                            "Start capture" => self.start_verified_capture(ctx),
                            "Save profile" => self.start_profile_save(ctx),
                            _ => {
                                self.advance();
                            }
                        }
                    }
                });
            });
            if let Some(last) = self.log.last().filter(|line| {
                !self.show_details
                    && !self.profile_flow.busy()
                    && !line.starts_with("Moved to: ")
                    && line.as_str() != "Welcome to PortableForge."
            }) {
                let summary = if last.chars().count() <= 126 {
                    last
                } else {
                    "Run status updated. Open Status and details to read it."
                };
                ui.label(
                    egui::RichText::new(summary)
                        .small()
                        .color(color(guide::theme::TEXT_MUTED)),
                );
            }
        });
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
                color(super::ALERT_TEXT),
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
        wizard.settings.text_scale = scale;
        let bounds = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::from(MIN_CONTENT));
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
    fn text_size_changes_keep_capture_controls_in_bounds() {
        let ctx = egui::Context::default();
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
                "Capture with Sandboxie-Plus",
                "Rebuild from a saved profile",
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
                "Launcher Generator · Ready",
                "Installer maker · Choose folder",
                "View prerequisite details",
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
                        "Saved profile",
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
