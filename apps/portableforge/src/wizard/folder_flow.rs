use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::time::{SystemTime, UNIX_EPOCH};

use eframe::egui;
#[cfg(windows)]
use portableforge_core::artwork::{default_application_icon, render_icon, Raster};
#[cfg(all(windows, target_arch = "x86_64"))]
use portableforge_core::folder::FolderBuildControl;
use portableforge_core::folder::{self, AppProperties, FolderIssue, FolderScan, IssueSeverity};
use portableforge_core::guide::folder_flow as copy;

use super::{artwork_ui, WizardApp};

mod draft;
#[cfg(all(windows, target_arch = "x86_64"))]
mod runtime;
mod view;

use draft::{suggested_id, Draft};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Page {
    #[default]
    Review,
    Program,
    App,
    About,
    Package,
    Rights,
    Files,
    Findings,
    Done,
}

#[derive(Default)]
pub(super) struct Flow {
    pub(super) open: bool,
    source: PathBuf,
    scan: Option<Arc<FolderScan>>,
    selected: Option<PathBuf>,
    inferred_name: bool,
    artwork: artwork_ui::Editor,
    #[cfg(windows)]
    base_icon: Option<Raster>,
    icon: Option<egui::TextureHandle>,
    issues: Vec<FolderIssue>,
    draft: Draft,
    output_parent: PathBuf,
    output_suffix: String,
    launcher_tools: PathBuf,
    installer_tools: PathBuf,
    page: Page,
    program_page: usize,
    issue_page: usize,
    findings_from: Page,
    error: Option<Failure>,
    result: Option<Delivery>,
    #[cfg(all(windows, target_arch = "x86_64"))]
    runtime: runtime::TestFlow,
    status: Option<&'static str>,
    job: Option<mpsc::Receiver<Event>>,
    #[cfg(all(windows, target_arch = "x86_64"))]
    control: Option<FolderBuildControl>,
    stop_pending: bool,
}

#[derive(Clone)]
struct Delivery {
    artifact: PathBuf,
    manifest: PathBuf,
    #[cfg(all(windows, target_arch = "x86_64"))]
    manifest_sha256: String,
    #[cfg(all(windows, target_arch = "x86_64"))]
    artifact_sha256: String,
    #[cfg(all(windows, target_arch = "x86_64"))]
    app_id: String,
}

struct Selection {
    relative: PathBuf,
    properties: AppProperties,
}

struct Inspection {
    scan: Arc<FolderScan>,
    selection: Option<Selection>,
    issues: Vec<FolderIssue>,
}

enum Outcome {
    Inspected(Inspection),
    DroppedSetup(PathBuf),
    Selected(Selection, Vec<FolderIssue>),
    #[cfg(all(windows, target_arch = "x86_64"))]
    Built(Delivery),
    #[cfg(all(windows, target_arch = "x86_64"))]
    Tested(Result<super::installer_test::InstallerTest, super::installer_test::TestFailure>),
    #[cfg(all(windows, target_arch = "x86_64"))]
    Stopped,
}

enum Event {
    #[cfg(all(windows, target_arch = "x86_64"))]
    Stage(&'static str),
    Finished(Box<Result<Outcome, Failure>>),
}

struct Failure {
    message: &'static str,
    detail: String,
}

impl Failure {
    fn new(message: &'static str, detail: impl ToString) -> Self {
        Self {
            message,
            detail: detail.to_string(),
        }
    }
}

fn inspect(path: &Path) -> Result<Outcome, Failure> {
    let scan = folder::scan(path)
        .map_err(|error| Failure::new("Choose a local app folder that you can open.", error))?;
    let selection = if scan.is_prepared() {
        None
    } else {
        scan.recommendation
            .clone()
            .zip(scan.properties.clone())
            .map(|(relative, properties)| Selection {
                relative,
                properties,
            })
    };
    let issues = folder::validate(
        &scan,
        selection
            .as_ref()
            .map(|selection| selection.relative.as_path()),
    );
    Ok(Outcome::Inspected(Inspection {
        scan: Arc::new(scan),
        selection,
        issues,
    }))
}

impl Flow {
    pub(super) fn busy(&self) -> bool {
        self.job.is_some()
    }

    pub(super) fn page(&self) -> Page {
        self.page
    }

    pub(super) fn modal_open(&self) -> bool {
        #[cfg(all(windows, target_arch = "x86_64"))]
        if self.runtime.prompt.is_some() {
            return true;
        }
        self.artwork.is_open()
    }

    fn test_in_progress(&self) -> bool {
        #[cfg(all(windows, target_arch = "x86_64"))]
        {
            self.runtime.cancellation.is_some()
        }
        #[cfg(not(all(windows, target_arch = "x86_64")))]
        false
    }

    #[cfg(windows)]
    fn open_artwork(&mut self, opener: egui::Id) {
        self.artwork.open_folder(
            opener,
            self.base_icon
                .clone()
                .unwrap_or_else(default_application_icon),
        );
    }

    pub(super) fn show_artwork(&mut self, ctx: &egui::Context) {
        if self.artwork.show(ctx) {
            self.refresh_review_icon(ctx);
        }
    }

    fn refresh_review_icon(&mut self, ctx: &egui::Context) {
        #[cfg(windows)]
        {
            let custom = self
                .artwork
                .artwork()
                .and_then(|artwork| artwork.icon.as_ref())
                .and_then(|icon| render_icon(icon, 32).ok());
            self.icon = custom.as_ref().or(self.base_icon.as_ref()).map(|raster| {
                ctx.load_texture(
                    "folder-program-icon",
                    egui::ColorImage::from_rgba_unmultiplied(
                        [raster.width() as usize, raster.height() as usize],
                        raster.rgba(),
                    ),
                    egui::TextureOptions::LINEAR,
                )
            });
        }
        #[cfg(not(windows))]
        let _ = ctx;
    }

    pub(super) fn back(&mut self) {
        if self.busy() || self.modal_open() {
            return;
        }
        if self.page == Page::Findings {
            self.page = self.findings_from;
        } else if matches!(self.page, Page::Review | Page::Done) {
            self.open = false;
        } else {
            self.page = Page::Review;
        }
    }

    pub(super) fn toggle_findings(&mut self) {
        if !self.busy() && !self.modal_open() {
            if self.page == Page::Findings {
                self.page = self.findings_from;
            } else {
                self.findings_from = self.page;
                self.issue_page = 0;
                self.page = Page::Findings;
            }
        }
    }

    fn start(
        &mut self,
        ctx: &egui::Context,
        status: &'static str,
        run: impl FnOnce(&mpsc::Sender<Event>, &egui::Context) -> Result<Outcome, Failure>
            + Send
            + 'static,
    ) {
        debug_assert!(!self.busy());
        self.error = None;
        self.status = Some(status);
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        match std::thread::Builder::new()
            .name("portableforge-folder".into())
            .spawn(move || {
                let result = run(&tx, &ctx);
                let _ = tx.send(Event::Finished(Box::new(result)));
                ctx.request_repaint();
            }) {
            Ok(_) => self.job = Some(rx),
            Err(error) => {
                self.status = None;
                self.error = Some(Failure::new("Could not start the task. Try again.", error));
            }
        }
    }

    fn inspect(&mut self, ctx: &egui::Context, source: PathBuf, dropped: bool) {
        let launcher_tools = std::mem::take(&mut self.launcher_tools);
        let installer_tools = std::mem::take(&mut self.installer_tools);
        *self = Self {
            open: true,
            source: source.clone(),
            launcher_tools,
            installer_tools,
            ..Self::default()
        };
        self.start(ctx, copy::SCANNING, move |_, _| {
            if dropped {
                use folder::input::{classify_drop, DroppedInput};
                let kind = classify_drop(&source).map_err(|error| {
                    Failure::new("Choose a local app folder or setup file.", error)
                })?;
                if kind == DroppedInput::Setup {
                    return Ok(Outcome::DroppedSetup(source));
                }
            }
            inspect(&source)
        });
    }

    fn select(&mut self, ctx: &egui::Context, relative: PathBuf) {
        let Some(scan) = self.scan.clone() else {
            return;
        };
        self.start(ctx, copy::READING_PROGRAM, move |_, _| {
            let properties = scan.properties_for(&relative).map_err(|error| {
                Failure::new(
                    "The selected file could not be read. Scan the folder again.",
                    error,
                )
            })?;
            let issues = folder::validate(&scan, Some(&relative));
            Ok(Outcome::Selected(
                Selection {
                    relative,
                    properties,
                },
                issues,
            ))
        });
    }

    fn accept_selection(&mut self, ctx: &egui::Context, selection: Selection) {
        if self.selected.as_ref() == Some(&selection.relative) {
            self.page = Page::Review;
            return;
        }
        self.draft = Draft::default();
        self.artwork = artwork_ui::Editor::default();
        let properties = selection.properties;
        self.draft.name = properties.name;
        self.draft.id = suggested_id(&self.draft.name);
        self.draft.publisher = properties.publisher.unwrap_or_default();
        self.draft.version = properties.version.unwrap_or_default();
        self.inferred_name = properties.name_inferred;
        #[cfg(windows)]
        {
            self.base_icon = Some(properties.icon.unwrap_or_else(default_application_icon));
        }
        self.refresh_review_icon(ctx);
        self.selected = Some(selection.relative);
        self.page = Page::Review;
    }

    pub(super) fn poll(&mut self, ctx: &egui::Context) -> Option<PathBuf> {
        let job = self.job.as_ref()?;
        let result = loop {
            match job.try_recv() {
                #[cfg(all(windows, target_arch = "x86_64"))]
                Ok(Event::Stage(status)) => {
                    if !self.stop_pending && !self.runtime.stopping {
                        self.status = Some(status);
                    }
                }
                Ok(Event::Finished(result)) => break *result,
                Err(mpsc::TryRecvError::Empty) => return None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    break Err(Failure::new(
                        "The task stopped unexpectedly. Try again.",
                        "Worker disconnected.",
                    ));
                }
            }
        };
        self.job = None;
        #[cfg(all(windows, target_arch = "x86_64"))]
        let was_testing = self.test_in_progress();
        #[cfg(all(windows, target_arch = "x86_64"))]
        {
            self.control = None;
            self.runtime.cancellation = None;
            self.runtime.stopping = false;
        }
        self.stop_pending = false;
        self.status = None;
        match result {
            Ok(Outcome::DroppedSetup(path)) => {
                self.open = false;
                return Some(path);
            }
            Ok(Outcome::Inspected(inspection)) => {
                self.output_parent = inspection
                    .scan
                    .root
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_default();
                self.output_suffix = SystemTime::now().duration_since(UNIX_EPOCH).map_or_else(
                    |_| std::process::id().to_string(),
                    |elapsed| format!("{:x}", elapsed.as_nanos() & 0x0000_ffff_ffff_ffff),
                );
                self.issues = inspection.issues;
                self.scan = Some(inspection.scan);
                if let Some(selection) = inspection.selection {
                    self.accept_selection(ctx, selection);
                }
            }
            Ok(Outcome::Selected(selection, issues)) => {
                self.issues = issues;
                self.accept_selection(ctx, selection);
            }
            #[cfg(all(windows, target_arch = "x86_64"))]
            Ok(Outcome::Built(delivery)) => {
                self.result = Some(delivery);
                self.page = Page::Done;
            }
            #[cfg(all(windows, target_arch = "x86_64"))]
            Ok(Outcome::Tested(result)) => {
                self.error = None;
                self.runtime.outcome = Some(result);
            }
            #[cfg(all(windows, target_arch = "x86_64"))]
            Ok(Outcome::Stopped) => self.status = Some(copy::STOPPED),
            #[cfg(all(windows, target_arch = "x86_64"))]
            Err(failure) if was_testing => {
                self.runtime.outcome = Some(Err(super::installer_test::TestFailure::Failed(
                    failure.detail,
                )));
            }
            Err(failure) => self.error = Some(failure),
        }
        None
    }

    fn output(&self) -> PathBuf {
        self.output_parent
            .join(format!("{}-{}", self.draft.id, self.output_suffix))
    }

    fn blocker(&self) -> Option<(Page, &'static str)> {
        if !cfg!(all(windows, target_arch = "x86_64")) {
            return Some((Page::Review, "Folder builds need 64-bit Windows."));
        }
        let scan = self.scan.as_ref()?;
        if scan.is_prepared() {
            return Some((Page::Review, copy::PREPARED_HELP));
        }
        if !scan.complete
            || scan
                .issues
                .iter()
                .any(|issue| issue.severity == IssueSeverity::Error)
        {
            return Some((
                Page::Findings,
                "Review the folder problems before building.",
            ));
        }
        if self.selected.is_none() {
            if scan.candidates.is_empty() {
                return Some((
                    Page::Review,
                    "Choose a folder containing the file that starts the app.",
                ));
            }
            return Some((Page::Program, "Choose the file that starts the app."));
        }
        if scan.candidates.iter().any(|candidate| {
            Some(&candidate.relative_path) == self.selected.as_ref()
                && !matches!(
                    candidate.architecture,
                    portableforge_core::snapshot::PeArch::X86
                        | portableforge_core::snapshot::PeArch::X64
                )
        }) {
            return Some((Page::Program, "Choose a start file for x86 or x64 Windows."));
        }
        if self
            .issues
            .iter()
            .any(|issue| issue.severity == IssueSeverity::Error)
        {
            return Some((
                Page::Findings,
                "Review the folder problems before building.",
            ));
        }
        if let Some(blocker) = self.draft.blocker() {
            return Some(blocker);
        }
        if self.launcher_tools.as_os_str().is_empty() || self.installer_tools.as_os_str().is_empty()
        {
            return Some((Page::Files, "Choose the two PortableApps.com tool folders."));
        }
        if self.output_parent.as_os_str().is_empty() {
            return Some((Page::Files, "Choose where to save the installer."));
        }
        None
    }

    pub(super) fn build_ready(&self) -> bool {
        self.page == Page::Review
            && !self.busy()
            && !self.modal_open()
            && self.scan.is_some()
            && self.blocker().is_none()
    }

    pub(super) fn show_build_action(&self) -> bool {
        self.page == Page::Review
            && self.scan.as_ref().is_some_and(|scan| !scan.is_prepared())
            && !self.busy()
    }

    #[cfg(all(windows, target_arch = "x86_64"))]
    pub(super) fn build(&mut self, ctx: &egui::Context) {
        if !self.build_ready() {
            return;
        }
        let (Some(scan), Some(selected)) = (self.scan.clone(), self.selected.clone()) else {
            return;
        };
        let Some(metadata) = self.draft.metadata(&selected) else {
            return;
        };
        let launcher_tools = self.launcher_tools.clone();
        let installer_tools = self.installer_tools.clone();
        let output = self.output();
        let artwork = self.artwork.artwork().cloned();
        let control = FolderBuildControl::default();
        self.control = Some(control.clone());
        self.start(ctx, "Checking the app folder…", move |tx, ctx| {
            use portableforge_core::folder::{build_with_artwork, FolderBuildRequest};
            let request = FolderBuildRequest {
                scan: &scan,
                selected: &selected,
                metadata: &metadata,
                launcher_tools: &launcher_tools,
                installer_tools: &installer_tools,
                output: &output,
            };
            let mut stage = "";
            match build_with_artwork(request, artwork.as_ref(), &control, |status| {
                stage = status;
                let _ = tx.send(Event::Stage(status));
                ctx.request_repaint();
            }) {
                Ok(report) => Ok(Outcome::Built(Delivery {
                    artifact: report.artifact,
                    manifest: report.manifest,
                    manifest_sha256: report.manifest_sha256,
                    artifact_sha256: report.sha256,
                    app_id: metadata.app_id,
                })),
                Err(error) if error == "Build stopped. No installer was published." => {
                    Ok(Outcome::Stopped)
                }
                Err(error) => {
                    let message = if error.starts_with("The application folder changed") {
                        "The app folder changed. Choose it again to refresh the review."
                    } else if error.starts_with("Choose a new output folder")
                        || error.starts_with("Could not publish to")
                    {
                        "Choose a new output location where you can save files."
                    } else if error.starts_with("Keep the application, launcher builder") {
                        "Choose separate folders for the app, build tools and output."
                    } else if stage == "Verifying the PortableApps.com builders" {
                        "Check the PortableApps.com tool folders and try again."
                    } else {
                        "The build could not finish. Open Show Details for the reason."
                    };
                    Err(Failure::new(message, error))
                }
            }
        });
        if !self.busy() {
            self.control = None;
        }
    }

    #[cfg(not(all(windows, target_arch = "x86_64")))]
    pub(super) fn build(&mut self, _ctx: &egui::Context) {}

    pub(super) fn can_stop(&self) -> bool {
        #[cfg(all(windows, target_arch = "x86_64"))]
        {
            self.control.is_some() && !self.stop_pending
        }
        #[cfg(not(all(windows, target_arch = "x86_64")))]
        false
    }

    pub(super) fn stop(&mut self) {
        #[cfg(all(windows, target_arch = "x86_64"))]
        if let Some(control) = &self.control {
            self.stop_pending = true;
            self.status = Some(if control.request_stop() {
                copy::STOPPING
            } else {
                "Saving the finished installer…"
            });
        }
    }
}

impl WizardApp {
    pub(super) fn choose_app_folder(&mut self, ctx: &egui::Context) {
        let dialog = rfd::FileDialog::new().set_title("Choose the unpacked app folder");
        if let Some(path) = dialog.pick_folder() {
            self.inspect_app_folder(ctx, path, false);
        }
    }

    pub(super) fn inspect_app_folder(&mut self, ctx: &egui::Context, path: PathBuf, dropped: bool) {
        if self.folder_flow.busy() || self.folder_flow.modal_open() || self.profile_flow.busy() {
            return;
        }
        let (launcher, installer) = self.profile_flow.folder_tool_paths();
        if self.folder_flow.launcher_tools.as_os_str().is_empty() {
            self.folder_flow.launcher_tools = launcher;
        }
        if self.folder_flow.installer_tools.as_os_str().is_empty() {
            self.folder_flow.installer_tools = installer;
        }
        self.folder_flow.inspect(ctx, path, dropped);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use portableforge_core::folder::{ExeCandidate, FolderIssue};
    use portableforge_core::snapshot::PeArch;

    fn populated() -> WizardApp {
        let source =
            PathBuf::from(r"C:\Apps With Spaces").join("Long Folder Name ".repeat(7).trim());
        let selected = PathBuf::from("bin")
            .join("Nested Folder ".repeat(7).trim())
            .join("Widget.exe");
        let scan = FolderScan {
            root: source.clone(),
            entries: vec![],
            issues: vec![],
            complete: true,
            candidates: vec![ExeCandidate {
                relative_path: selected.clone(),
                architecture: PeArch::X86,
                reasons: vec![],
                score: 5,
            }],
            recommendation: Some(selected.clone()),
            properties: None,
        };
        let draft = Draft {
            name: "Widget".into(),
            publisher: "Widget Publisher".into(),
            homepage: "https://example.com/widget".into(),
            description: "An app for checking package creation.".into(),
            version: "2.4.1.0".into(),
            id: "WidgetPortable".into(),
            language: Some(draft::Language::English),
            rights: true,
            ..Default::default()
        };
        let folder_flow = Flow {
            open: true,
            source,
            scan: Some(Arc::new(scan)),
            selected: Some(selected),
            issues: vec![FolderIssue {
                code: "absolute_configuration_path",
                severity: IssueSeverity::Warning,
                relative_path: Some(PathBuf::from("settings.ini")),
                message: "This settings file may refer to a fixed drive or network share. Review whether the app needs that location.".into(),
            }],
            draft,
            output_parent: PathBuf::from(r"C:\Users\Tester").join("Output Location ".repeat(6).trim()),
            output_suffix: "f3b217".into(),
            launcher_tools: PathBuf::from(r"C:\Tools\Launcher"),
            installer_tools: PathBuf::from(r"C:\Tools\Installer"),
            ..Default::default()
        };
        WizardApp {
            folder_flow,
            ..Default::default()
        }
    }

    fn selection(relative: &str) -> Selection {
        Selection {
            relative: relative.into(),
            properties: AppProperties {
                name: "Read from File".into(),
                name_inferred: false,
                publisher: Some("Original Publisher".into()),
                version: Some("1.0.0.0".into()),
                #[cfg(windows)]
                icon: None,
            },
        }
    }

    #[test]
    fn same_program_preserves_review_and_changed_program_clears_permission() {
        let ctx = egui::Context::default();
        let mut flow = Flow::default();
        flow.accept_selection(&ctx, selection("first.exe"));
        flow.draft.name = "Reviewed name".into();
        flow.draft.id = "ReviewedPortable".into();
        flow.draft.rights = true;
        flow.accept_selection(&ctx, selection("first.exe"));
        assert_eq!(flow.draft.name, "Reviewed name");
        assert_eq!(flow.draft.id, "ReviewedPortable");
        assert!(flow.draft.rights);
        flow.accept_selection(&ctx, selection("bin/second.exe"));
        assert_eq!(flow.draft.name, "Read from File");
        assert!(!flow.draft.rights);
    }

    #[test]
    fn build_action_requires_complete_facts_and_explicit_permission() {
        let mut wizard = populated();
        let flow = &mut wizard.folder_flow;
        flow.draft.rights = false;
        assert!(!flow.build_ready());
        flow.draft.rights = true;
        flow.draft.version = "unknown".into();
        assert!(!flow.build_ready());
        flow.draft.version = "2.4.1".into();
        let meta = flow
            .draft
            .metadata(flow.selected.as_deref().unwrap())
            .unwrap();
        assert_eq!(meta.package_version, "2.4.1.0");
        assert!(meta.main_exe.starts_with(r"bin\"));
        assert_eq!(
            flow.build_ready(),
            cfg!(all(windows, target_arch = "x86_64"))
        );
        flow.issues[0].severity = IssueSeverity::Error;
        assert!(!flow.build_ready());
    }

    #[test]
    fn dropping_a_folder_or_setup_routes_to_the_correct_workflow() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("pf-ui-drop-{}-{stamp}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let folder = root.join("Widget.exe");
        std::fs::create_dir(&folder).unwrap();
        let setup = root.join("WidgetSetup.exe");
        std::fs::write(&setup, b"inert setup fixture").unwrap();
        let mut wizard = WizardApp {
            settings: Default::default(),
            settings_path: root.join("wizard-settings.json"),
            resume_offer: false,
            ..Default::default()
        };
        let ctx = egui::Context::default();
        super::super::theme::install_fonts(&ctx);
        let mut frame = eframe::Frame::_new_kittest();
        let mut drive_drop = |wizard: &mut WizardApp, path: &Path| {
            let _ = ctx.run(
                egui::RawInput {
                    dropped_files: vec![egui::DroppedFile {
                        path: Some(path.to_path_buf()),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                |ctx| eframe::App::update(wizard, ctx, &mut frame),
            );
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while wizard.folder_flow.busy() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "drop worker timed out"
                );
                std::thread::yield_now();
                let _ = ctx.run(egui::RawInput::default(), |ctx| {
                    eframe::App::update(wizard, ctx, &mut frame);
                });
            }
        };
        drive_drop(&mut wizard, &folder);
        assert!(wizard.folder_flow.open);
        assert!(wizard.folder_flow.scan.is_some());
        assert!(wizard.installer_path.is_empty());
        wizard.folder_flow.back();
        drive_drop(&mut wizard, &setup);
        assert!(!wizard.folder_flow.open);
        assert_eq!(Path::new(&wizard.installer_path), setup);
        std::fs::remove_dir(folder).unwrap();
        std::fs::remove_file(setup).unwrap();
        if wizard.settings_path.is_file() {
            std::fs::remove_file(wizard.settings_path).unwrap();
        }
        std::fs::remove_dir(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn folder_artwork_is_applied_only_on_confirmation_and_tracks_the_selected_program() {
        let mut wizard = populated();
        let ctx = egui::Context::default();
        super::super::theme::install_fonts(&ctx);
        let bounds = egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::Vec2::from(super::super::dense::MIN_CONTENT),
        );
        let mut frame = eframe::Frame::_new_kittest();
        let mut draw = |wizard: &mut WizardApp, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(bounds),
                    events,
                    ..Default::default()
                },
                |ctx| eframe::App::update(wizard, ctx, &mut frame),
            )
        };
        let mut click = |wizard: &mut WizardApp, label: &str| {
            let output = draw(wizard, vec![]);
            let pos = output
                .shapes
                .iter()
                .find(|shape| {
                    matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == label)
                })
                .unwrap_or_else(|| panic!("Missing action {label:?}"))
                .shape
                .visual_bounding_rect()
                .center();
            for pressed in [true, false] {
                draw(
                    wizard,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
        };

        click(&mut wizard, "Edit Package Artwork…");
        assert!(wizard.folder_flow.modal_open());
        assert!(!wizard.folder_flow.build_ready());
        let source = wizard.folder_flow.source.clone();
        wizard.inspect_app_folder(&ctx, source.join("Another"), true);
        assert_eq!(wizard.folder_flow.source, source);
        wizard.folder_flow.back();
        assert!(wizard.folder_flow.open);

        click(&mut wizard, "Installer Left Pane");
        click(&mut wizard, "Use app icon");
        assert!(wizard.folder_flow.artwork.artwork().is_none());
        click(&mut wizard, "Apply Artwork");
        click(&mut wizard, "Edit Package Artwork…");
        click(&mut wizard, "Installer Left Pane");
        click(&mut wizard, "Use default left pane");
        click(&mut wizard, "Cancel");
        let flow = &mut wizard.folder_flow;
        assert!(!flow.modal_open());
        assert!(flow.artwork.artwork().unwrap().sidebar.is_some());
        assert!(wizard.artwork.artwork().is_none());

        let selected = flow.selected.clone().unwrap();
        flow.accept_selection(&ctx, selection(selected.to_str().unwrap()));
        flow.page = Page::App;
        flow.back();
        assert!(flow.artwork.artwork().unwrap().sidebar.is_some());
        flow.accept_selection(&ctx, selection("bin/another.exe"));
        assert!(flow.artwork.artwork().is_none());
    }

    #[test]
    fn folder_views_fit_at_minimum_size_at_all_text_sizes() {
        for scale in [1.0, 1.25, 1.5] {
            let mut wizard = populated();
            wizard.settings.text_scale = scale;
            let ctx = egui::Context::default();
            super::super::theme::install_fonts(&ctx);
            let bounds = egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::Vec2::from(super::super::dense::MIN_CONTENT),
            );
            let mut frame = eframe::Frame::_new_kittest();
            for (page, labels) in [
                (
                    Page::Review,
                    &[
                        "Review Your App",
                        #[cfg(windows)]
                        "Edit Package Artwork…",
                        "Build Installer",
                        copy::RIGHTS,
                    ][..],
                ),
                (
                    Page::Program,
                    &["Choose the File That Starts the App", "Currently selected"][..],
                ),
                (Page::App, &["App Name", "Publisher", "Version Number"][..]),
                (Page::About, &["App Website", "Short Description"][..]),
                (
                    Page::Package,
                    &["Package Name", "Installer Language", "Multilingual"][..],
                ),
                (
                    Page::Rights,
                    &["Open source", "Requires administrator access"][..],
                ),
                (
                    Page::Files,
                    &[
                        "Choose Launcher Folder…",
                        "Choose Installer Folder…",
                        "Change Save Location…",
                    ][..],
                ),
                (
                    Page::Findings,
                    &["Folder Findings", "Review before moving the app"][..],
                ),
                (
                    Page::Done,
                    &[
                        "Installer Built",
                        "Build Receipt",
                        "Open Output Folder",
                        #[cfg(all(windows, target_arch = "x86_64"))]
                        copy::TEST_INSTALLER,
                    ][..],
                ),
            ] {
                wizard.folder_flow.page = page;
                if page == Page::Done {
                    wizard.folder_flow.result = Some(Delivery {
                        artifact: wizard
                            .folder_flow
                            .output()
                            .join("WidgetPortable_2.4.1.paf.exe"),
                        manifest: wizard.folder_flow.output().join("build-manifest.json"),
                        #[cfg(all(windows, target_arch = "x86_64"))]
                        manifest_sha256: "a".repeat(64),
                        #[cfg(all(windows, target_arch = "x86_64"))]
                        artifact_sha256: "b".repeat(64),
                        #[cfg(all(windows, target_arch = "x86_64"))]
                        app_id: "WidgetPortable".into(),
                    });
                }
                let mut draw = || {
                    ctx.run(
                        egui::RawInput {
                            screen_rect: Some(bounds),
                            ..Default::default()
                        },
                        |ctx| eframe::App::update(&mut wizard, ctx, &mut frame),
                    )
                };
                draw();
                draw();
                let output = draw();
                for &label in labels {
                    let shape = output.shapes.iter().find(|shape| {
                        matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == label)
                    }).unwrap_or_else(|| panic!("Missing {label:?} on {page:?} at {scale}"));
                    let visible = bounds.intersect(shape.clip_rect).expand(1.0);
                    assert!(
                        visible.contains_rect(shape.shape.visual_bounding_rect()),
                        "{label:?} clipped on {page:?} at {scale}"
                    );
                }
            }
        }
    }
}
