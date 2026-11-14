//! Confirmation and status for testing the installer just built from a folder.

use eframe::egui;
use portableforge_core::guide::folder_flow as copy;
use portableforge_core::sbie_verified::CancellationToken;

use super::super::dense;
use super::super::installer_test::{self, BuildIdentity, InstallerTest, TestFailure};
use super::{Delivery, Event, Failure, Flow, Outcome, Page};

const TEST_DIALOG_ID: &str = "folder-installer-test";

#[derive(Default)]
pub(super) struct TestFlow {
    pub(super) prompt: Option<Confirmation>,
    return_focus: bool,
    pub(super) cancellation: Option<CancellationToken>,
    pub(super) stopping: bool,
    pub(super) outcome: Option<Result<InstallerTest, TestFailure>>,
}

pub(super) struct Confirmation {
    focus_first: bool,
}

impl Delivery {
    fn test_identity(&self) -> Result<BuildIdentity<'_>, TestFailure> {
        let directory = self.artifact.parent().ok_or_else(|| {
            TestFailure::Identity("The built installer folder is missing.".into())
        })?;
        if self.manifest.parent() != Some(directory)
            || self.manifest.file_name().and_then(|name| name.to_str())
                != Some("build-manifest.json")
        {
            return Err(TestFailure::Identity(
                "The build receipt does not belong to the selected installer folder.".into(),
            ));
        }
        Ok(BuildIdentity {
            directory,
            artifact: &self.artifact,
            manifest_sha256: &self.manifest_sha256,
            artifact_sha256: &self.artifact_sha256,
            app_id: &self.app_id,
        })
    }
}

impl Flow {
    pub(super) fn open_test_confirmation(&mut self) {
        if self.page == Page::Done && self.result.is_some() && !self.busy() && !self.modal_open() {
            self.runtime.prompt = Some(Confirmation { focus_first: true });
        }
    }

    pub(super) fn restore_test_focus(&mut self, ui: &egui::Ui, button: &egui::Response) {
        if !self.runtime.return_focus || !button.enabled() || !ui.is_rect_visible(button.rect) {
            return;
        }
        let modal = ui.memory(|memory| memory.top_modal_layer());
        if modal.is_none() {
            // The result can render a different widget ID after the test.
            button.request_focus();
            self.runtime.return_focus = false;
            ui.ctx().request_repaint();
        } else if modal.is_some_and(|layer| layer.id == egui::Id::new(TEST_DIALOG_ID)) {
            // egui reports the modal layer from the previous frame.
            ui.ctx().request_repaint();
        }
    }

    pub(in crate::wizard) fn show_test_confirmation(&mut self, ctx: &egui::Context) {
        let Some(prompt) = &mut self.runtime.prompt else {
            return;
        };

        let response = egui::Modal::new(egui::Id::new(TEST_DIALOG_ID)).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 48.0).clamp(200.0, 460.0));
            ui.heading(copy::TEST_CONFIRM);
            ui.label(copy::TEST_TRUST_HELP);
            ui.label(copy::TEST_SETUP_HELP);
            ui.label(copy::TEST_LAUNCH_HELP);
            ui.separator();
            ui.horizontal(|ui| {
                let cancel = ui.button("Cancel");
                if prompt.focus_first {
                    cancel.request_focus();
                    prompt.focus_first = false;
                }
                let start = ui
                    .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        dense::primary_action(ui, true, copy::TEST_START).clicked()
                    })
                    .inner;
                (cancel.clicked(), start)
            })
            .inner
        });
        let cancelled = response.inner.0 || response.should_close();
        if cancelled || response.inner.1 {
            self.runtime.return_focus = true;
            self.runtime.prompt = None;
            if !cancelled {
                self.start_test(ctx);
            }
            ctx.request_repaint();
        }
    }

    fn start_test(&mut self, ctx: &egui::Context) {
        if self.busy() || ctx.input(|input| input.viewport().close_requested()) {
            return;
        }
        let Some(delivery) = self.result.clone() else {
            return;
        };
        self.runtime.outcome = None;
        let cancellation = match CancellationToken::new() {
            Ok(cancellation) => cancellation,
            Err(error) => {
                self.runtime.outcome = Some(Err(TestFailure::Startup(error)));
                return;
            }
        };
        let worker_token = cancellation.clone();
        self.start(ctx, copy::TEST_CHECKING, move |tx, ctx| {
            let tested = delivery.test_identity().and_then(|identity| {
                installer_test::run(identity, &worker_token, || {
                    let _ = tx.send(Event::Stage(copy::TEST_USE_WINDOWS));
                    ctx.request_repaint();
                })
            });
            Ok(Outcome::Tested(tested))
        });
        if self.busy() {
            self.runtime.cancellation = Some(cancellation);
        } else if let Some(failure) = self.error.take() {
            self.runtime.outcome = Some(Err(TestFailure::Startup(failure.detail)));
        }
    }

    pub(super) fn cancel_test(&mut self) {
        if self.runtime.stopping {
            return;
        }
        if let Some(cancellation) = &self.runtime.cancellation {
            match cancellation.cancel() {
                Ok(()) => {
                    self.runtime.stopping = true;
                    self.status = Some(copy::TEST_STOPPING);
                }
                Err(error) => {
                    self.error = Some(Failure::new("Could not cancel the test. Try again.", error));
                }
            }
        }
    }

    pub(super) fn show_test_outcome(&self, ui: &mut egui::Ui) -> bool {
        let Some(outcome) = &self.runtime.outcome else {
            return false;
        };
        let details = match outcome {
            Ok(test) => {
                dense::heading(ui, copy::TEST_FINISHED, copy::TEST_FINISHED_HELP);
                ui.label(test.data_message());
                ui.weak(copy::UNVERIFIED);
                std::borrow::Cow::Owned(test.details())
            }
            Err(error) => {
                let (title, help) = match error {
                    TestFailure::Identity(_) => (copy::TEST_FAILED, copy::TEST_IDENTITY_HELP),
                    TestFailure::Startup(_) => (copy::TEST_FAILED, copy::TEST_STARTUP_HELP),
                    TestFailure::Failed(_) => (copy::TEST_FAILED, copy::TEST_FAILURE_HELP),
                    TestFailure::Cleanup(_) => (copy::TEST_FAILED, copy::TEST_CLEANUP_HELP),
                    TestFailure::Cancelled(_) => (copy::TEST_CANCELLED, copy::TEST_CANCELLED_HELP),
                };
                dense::heading(ui, title, help);
                ui.weak(copy::UNVERIFIED);
                std::borrow::Cow::Borrowed(error.detail())
            }
        };
        dense::view_text(
            ui,
            copy::TEST_DETAILS,
            "Installer Test Details",
            &details,
            true,
        );
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wizard::{theme, WizardApp};
    use portableforge_core::sbie_verified::{RuntimeDataStatus, RuntimeReport};
    use std::path::PathBuf;
    use std::sync::mpsc;

    struct Frame {
        ctx: egui::Context,
        frame: eframe::Frame,
        bounds: egui::Rect,
    }

    impl Frame {
        fn new() -> Self {
            let ctx = egui::Context::default();
            theme::install_fonts(&ctx);
            Self {
                ctx,
                frame: eframe::Frame::_new_kittest(),
                bounds: egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::Vec2::from(dense::MIN_CONTENT),
                ),
            }
        }

        fn draw(&mut self, wizard: &mut WizardApp, mut input: egui::RawInput) -> egui::FullOutput {
            input.screen_rect = Some(self.bounds);
            let output = self.ctx.run(input, |ctx| {
                eframe::App::update(wizard, ctx, &mut self.frame)
            });
            if let Some(update) = &output.platform_output.accesskit_update {
                assert!(
                    update.nodes.iter().any(|(id, _)| *id == update.focus),
                    "AccessKit focused absent node {:?}",
                    update.focus
                );
            }
            output
        }

        fn click(&mut self, wizard: &mut WizardApp, label: &str) {
            let output = self.draw(wizard, Default::default());
            let pos = find(&output, label).shape.visual_bounding_rect().center();
            for pressed in [true, false] {
                self.draw(
                    wizard,
                    egui::RawInput {
                        events: vec![
                            egui::Event::PointerMoved(pos),
                            egui::Event::PointerButton {
                                pos,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ],
                        ..Default::default()
                    },
                );
            }
        }

        fn fit(&mut self, wizard: &mut WizardApp, labels: &[&str]) {
            self.draw(wizard, Default::default());
            self.draw(wizard, Default::default());
            let output = self.draw(wizard, Default::default());
            for label in labels {
                let shape = find(&output, label);
                assert!(
                    self.bounds
                        .intersect(shape.clip_rect)
                        .expand(1.0)
                        .contains_rect(shape.shape.visual_bounding_rect()),
                    "{label} clipped at zoom {}",
                    self.ctx.zoom_factor(),
                );
            }
        }

        fn assert_test_focus(&mut self, wizard: &mut WizardApp) {
            let mut focused_test_action = false;
            for _ in 0..3 {
                let update = self
                    .draw(wizard, Default::default())
                    .platform_output
                    .accesskit_update
                    .expect("AccessKit was enabled");
                let focused = update
                    .nodes
                    .iter()
                    .find(|(id, _)| *id == update.focus)
                    .expect("focused node is present");
                focused_test_action |= focused.1.label() == Some(copy::TEST_INSTALLER);
            }
            assert!(
                focused_test_action,
                "focus did not return to Test Installer"
            );
        }
    }

    fn find<'a>(output: &'a egui::FullOutput, label: &str) -> &'a egui::epaint::ClippedShape {
        output
            .shapes
            .iter()
            .find(|shape| {
                matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == label)
            })
            .unwrap_or_else(|| panic!("Missing {label:?}"))
    }

    fn fixture() -> WizardApp {
        let directory =
            crate::wizard::unique_folder(&std::env::temp_dir(), "pf-ui-missing-test").unwrap();
        WizardApp {
            folder_flow: Flow {
                open: true,
                page: Page::Done,
                result: Some(Delivery {
                    artifact: directory.join("WidgetPortable_1.2.0_English.paf.exe"),
                    manifest: directory.join("build-manifest.json"),
                    manifest_sha256: "a".repeat(64),
                    artifact_sha256: "b".repeat(64),
                    app_id: "WidgetPortable".into(),
                }),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn completed_test() -> InstallerTest {
        InstallerTest::checked(
            RuntimeReport {
                installer_sha256: "approved".into(),
                install_exit: 0,
                launch_exits: [0, 0],
                data_files_created_or_changed: 1,
                data_files_deleted: 0,
                data_files_unchanged_on_reopen: 1,
                data_status: RuntimeDataStatus::ObservedChangesRetained,
                cleanup_verified: true,
            },
            PathBuf::from(r"C:\Tests\Installer-Test"),
            "approved",
        )
        .unwrap()
    }

    #[test]
    fn confirmation_cancel_does_no_work_and_start_keeps_the_failed_build_visible() {
        let mut wizard = fixture();
        let mut frame = Frame::new();
        frame.ctx.enable_accesskit();
        frame.click(&mut wizard, copy::TEST_INSTALLER);
        assert!(wizard.folder_flow.modal_open());
        frame.click(&mut wizard, "Cancel");
        assert!(!wizard.folder_flow.modal_open());
        assert!(!wizard.folder_flow.busy());
        assert!(wizard.folder_flow.runtime.outcome.is_none());
        frame.assert_test_focus(&mut wizard);

        frame.click(&mut wizard, copy::TEST_INSTALLER);
        frame.click(&mut wizard, copy::TEST_START);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while wizard.folder_flow.busy() {
            assert!(
                std::time::Instant::now() < deadline,
                "test worker timed out"
            );
            std::thread::yield_now();
            frame.draw(&mut wizard, Default::default());
        }
        assert!(matches!(
            wizard.folder_flow.runtime.outcome,
            Some(Err(TestFailure::Identity(_)))
        ));
        assert!(wizard.folder_flow.result.is_some());
        frame.assert_test_focus(&mut wizard);
        frame.fit(&mut wizard, &[copy::TEST_FAILED, "View Output Folder"]);
    }

    #[test]
    fn running_test_keeps_result_locked_and_cancels_through_native_token() {
        let mut wizard = fixture();
        let mut frame = Frame::new();
        let (tx, rx) = mpsc::channel();
        wizard.folder_flow.job = Some(rx);
        let token = CancellationToken::new().unwrap();
        wizard.folder_flow.runtime.cancellation = Some(token.clone());
        wizard.folder_flow.back();
        assert!(wizard.folder_flow.open);

        let mut close = egui::RawInput::default();
        close
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .events
            .push(egui::ViewportEvent::Close);
        let output = frame.draw(&mut wizard, close);
        assert!(output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .contains(&egui::ViewportCommand::CancelClose));
        frame.click(&mut wizard, copy::TEST_CANCEL);
        assert!(token.is_cancelled());
        assert!(wizard.folder_flow.busy());
        let outcome = Outcome::Tested(Err(TestFailure::Cancelled(
            "The dedicated box was cleaned up.".into(),
        )));
        assert!(tx.send(Event::Finished(Box::new(Ok(outcome)))).is_ok());
        frame.draw(&mut wizard, Default::default());
        assert!(!wizard.folder_flow.busy());
        assert!(wizard.folder_flow.result.is_some());
        frame.fit(&mut wizard, &[copy::TEST_CANCELLED, "View Output Folder"]);
    }

    #[test]
    fn successful_test_restores_focus_to_the_rendered_test_action() {
        let mut wizard = fixture();
        let mut frame = Frame::new();
        frame.ctx.enable_accesskit();
        frame.draw(&mut wizard, Default::default());

        let (tx, rx) = mpsc::channel();
        wizard.folder_flow.job = Some(rx);
        wizard.folder_flow.runtime.cancellation = Some(CancellationToken::new().unwrap());
        wizard.folder_flow.runtime.return_focus = true;
        frame.draw(&mut wizard, Default::default());

        let outcome = Outcome::Tested(Ok(completed_test()));
        assert!(tx.send(Event::Finished(Box::new(Ok(outcome)))).is_ok());
        let output = frame.draw(&mut wizard, Default::default());
        find(&output, copy::TEST_FINISHED);
        frame.assert_test_focus(&mut wizard);
    }

    #[test]
    fn test_prompt_and_outcomes_fit_at_every_supported_text_size() {
        for scale in [1.0, 1.25, 1.5] {
            let mut wizard = fixture();
            let mut frame = Frame::new();
            wizard.settings.text_scale = scale;
            wizard.folder_flow.open_test_confirmation();
            frame.fit(
                &mut wizard,
                &[copy::TEST_CONFIRM, copy::TEST_START, "Cancel"],
            );
            wizard.folder_flow.runtime.prompt = None;
            for failure in [
                TestFailure::Startup("Sandboxie-Plus is unavailable.".into()),
                TestFailure::Cleanup("The owned box still exists.".into()),
            ] {
                wizard.folder_flow.runtime.outcome = Some(Err(failure));
                frame.fit(
                    &mut wizard,
                    &[copy::TEST_FAILED, copy::TEST_DETAILS, "View Output Folder"],
                );
                let output = frame.draw(&mut wizard, Default::default());
                assert!(!output.shapes.iter().any(|shape| {
                    matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == copy::TEST_FINISHED)
                }));
            }
            wizard.folder_flow.runtime.outcome = Some(Ok(completed_test()));
            frame.fit(
                &mut wizard,
                &[
                    copy::TEST_FINISHED,
                    copy::TEST_DETAILS,
                    "View Output Folder",
                ],
            );
        }
    }
}
