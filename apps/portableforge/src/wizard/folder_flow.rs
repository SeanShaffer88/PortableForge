use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{SystemTime, UNIX_EPOCH};

use eframe::egui;
#[cfg(windows)]
use portableforge_core::artwork::{default_application_icon, render_icon, Raster};
use portableforge_core::folder::input::{
    plan_drop_with_cancel, stage_input_with_cancel, DropPlan, StagedInput,
};
#[cfg(all(windows, target_arch = "x86_64"))]
use portableforge_core::folder::FolderBuildControl;
use portableforge_core::folder::{
    self, AppProperties, FolderIssue, FolderScan, IssueSeverity, ScanError,
};
use portableforge_core::guide::folder_flow as copy;

use super::{artwork_ui, WizardApp};

mod draft;
#[cfg(all(windows, target_arch = "x86_64"))]
mod runtime;
mod saved_job;
mod view;

use draft::{suggested_id, Draft};
use saved_job::{SavedJob, SourceKind};

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

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
enum IntakeMode {
    #[default]
    Guided,
    Autonomous,
}

#[derive(Clone, Copy)]
enum DiscardStage {
    Back,
    Built,
    Close,
    Saved,
    Stopped,
}

#[derive(Default)]
pub(super) struct Flow {
    pub(super) open: bool,
    source: PathBuf,
    source_label: String,
    original_inputs: Vec<PathBuf>,
    source_kind: Option<SourceKind>,
    restoring: Option<SavedJob>,
    staged: Option<Arc<StagedInput>>,
    mode: IntakeMode,
    build_attempted: bool,
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
    intake_stop: Option<Arc<AtomicBool>>,
    discard_stage: Option<DiscardStage>,
    close_ready: bool,
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
    staged: Option<Arc<StagedInput>>,
    original: PathBuf,
    source_label: String,
    source_kind: SourceKind,
    scan: Arc<FolderScan>,
    selection: Option<Selection>,
    issues: Vec<FolderIssue>,
}

enum Outcome {
    Inspected(Inspection),
    DroppedSetup(PathBuf),
    IntakeStopped,
    StageDiscarded(DiscardStage, Result<(), Failure>),
    SavedForLater,
    Selected(Selection, Vec<FolderIssue>),
    #[cfg(all(windows, target_arch = "x86_64"))]
    Built(Delivery),
    #[cfg(all(windows, target_arch = "x86_64"))]
    Tested(Result<super::installer_test::InstallerTest, super::installer_test::TestFailure>),
    #[cfg(all(windows, target_arch = "x86_64"))]
    Stopped,
}

enum Event {
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

fn inspect(
    root: &Path,
    original: PathBuf,
    source_label: String,
    source_kind: SourceKind,
) -> Result<Inspection, Failure> {
    let scan = folder::scan(root)
        .map_err(|error| Failure::new("The selected app files could not be read.", error))?;
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
    Ok(Inspection {
        staged: None,
        original,
        source_label,
        source_kind,
        scan: Arc::new(scan),
        selection,
        issues,
    })
}

fn dispose_stage(stage: Option<Arc<StagedInput>>) -> Result<(), Failure> {
    if let Some(stage) = stage {
        let stage = Arc::try_unwrap(stage)
            .map_err(|stage| Failure::new(copy::SOURCE_CLEANUP_FAILED, stage.root().display()))?;
        stage
            .cleanup()
            .map_err(|error| Failure::new(copy::SOURCE_CLEANUP_FAILED, error))?;
    }
    Ok(())
}

fn intake_failure(error: ScanError) -> Result<Outcome, Failure> {
    let detail = error.to_string();
    const CANCELLED: &str = "Input staging stopped.";
    if detail == CANCELLED {
        Ok(Outcome::IntakeStopped)
    } else if detail.starts_with(CANCELLED) {
        Err(Failure::new(copy::SOURCE_CLEANUP_FAILED, detail))
    } else {
        Err(Failure::new(copy::INPUT_NOT_READY, detail))
    }
}

impl Flow {
    pub(super) fn busy(&self) -> bool {
        self.job.is_some() || self.discard_stage.is_some()
    }

    pub(super) fn defer_close(&mut self) -> bool {
        if self.busy() {
            return true;
        }
        if self.staged.is_some() {
            self.discard(DiscardStage::Close);
            return true;
        }
        false
    }

    pub(super) fn take_close_request(&mut self) -> bool {
        std::mem::take(&mut self.close_ready)
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

    fn test_stopping(&self) -> bool {
        #[cfg(all(windows, target_arch = "x86_64"))]
        {
            self.runtime.stopping
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
            self.discard(DiscardStage::Back);
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

    fn reset_source(&mut self, source: PathBuf) {
        let launcher_tools = std::mem::take(&mut self.launcher_tools);
        let installer_tools = std::mem::take(&mut self.installer_tools);
        *self = Self {
            open: true,
            source,
            mode: self.mode,
            launcher_tools,
            installer_tools,
            ..Self::default()
        };
    }

    fn discard(&mut self, after: DiscardStage) {
        if self.staged.is_some() {
            self.discard_stage = Some(after);
        } else {
            self.after_discard(after);
        }
    }

    fn after_discard(&mut self, after: DiscardStage) {
        match after {
            DiscardStage::Back => {
                self.error = None;
                self.open = false;
            }
            DiscardStage::Built => {}
            DiscardStage::Close => self.close_ready = true,
            DiscardStage::Saved => {
                self.open = false;
                self.status = Some("App job saved.");
            }
            DiscardStage::Stopped => self.status = Some(copy::SOURCE_STOPPED),
        }
    }

    fn discard_failed(&mut self) {
        if self.page != Page::Done {
            self.scan = None;
            self.selected = None;
        }
    }

    fn inspect(
        &mut self,
        ctx: &egui::Context,
        paths: Vec<PathBuf>,
        allow_setup: bool,
        saved_kind: Option<SourceKind>,
    ) {
        let old_stage = self.staged.take();
        let source = if paths.len() == 1 {
            paths[0].clone()
        } else {
            PathBuf::new()
        };
        self.reset_source(source);
        if self.mode == IntakeMode::Autonomous {
            self.original_inputs = paths.clone();
        }
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        self.start(ctx, copy::CHECKING_SELECTION, move |tx, ctx| {
            dispose_stage(old_stage)?;
            let show_stage = |status| {
                let _ = tx.send(Event::Stage(status));
                ctx.request_repaint();
            };
            let cancelled = || worker_stop.load(Ordering::Acquire);
            let plan = match plan_drop_with_cancel(paths, cancelled) {
                Ok(plan) => plan,
                Err(error) => return intake_failure(error),
            };
            let source_kind = SourceKind::from_plan(&plan);
            if saved_kind.is_some_and(|expected| Some(expected) != source_kind) {
                return Err(Failure::new(
                    "The original app files changed. Choose them again to start a new job.",
                    "The current source type does not match the saved app source.",
                ));
            }
            let plan = match plan {
                DropPlan::Setup(path) if allow_setup => return Ok(Outcome::DroppedSetup(path)),
                DropPlan::Setup(_) => {
                    return Err(Failure::new(
                        copy::INPUT_NOT_READY,
                        "Use the setup picker for one setup file. For loose app files, select all files for the app together.",
                    ));
                }
                DropPlan::Folder(path) => {
                    show_stage(copy::SCANNING);
                    let label = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy().into_owned();
                    return inspect(&path, path.clone(), label, SourceKind::Folder)
                        .map(Outcome::Inspected);
                }
                plan => plan,
            };
            let loose_files = matches!(plan, DropPlan::Files(_));
            show_stage(copy::STAGING_SOURCE);
            let staged = match stage_input_with_cancel(plan, cancelled) {
                Ok(staged) => Arc::new(staged),
                Err(error) => return intake_failure(error),
            };
            if cancelled() {
                dispose_stage(Some(staged))?;
                return Ok(Outcome::IntakeStopped);
            }
            show_stage(copy::SCANNING);
            let original = staged.original().to_path_buf();
            let label = if loose_files {
                format!("Selected files in {}", staged.label())
            } else {
                original.file_name().unwrap_or(original.as_os_str()).to_string_lossy().into_owned()
            };
            match inspect(staged.root(), original, label, source_kind.expect("staged app source")) {
                Ok(mut inspection) => {
                    inspection.staged = Some(staged);
                    Ok(Outcome::Inspected(inspection))
                }
                Err(failure) => {
                    dispose_stage(Some(staged))?;
                    Err(failure)
                }
            }
        });
        if self.busy() {
            self.intake_stop = Some(stop);
        }
    }

    fn select(&mut self, ctx: &egui::Context, relative: PathBuf) {
        let Some(scan) = self.scan.clone() else {
            return;
        };
        let staged = self.staged.clone();
        self.start(ctx, copy::READING_PROGRAM, move |_, _| {
            let _staged = staged;
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
        if self.mode == IntakeMode::Autonomous {
            self.draft.language = Some(draft::Language::English);
        }
        self.artwork = artwork_ui::Editor::default();
        let properties = selection.properties;
        self.draft.name = if properties.name_inferred && self.staged.is_some() {
            selection
                .relative
                .file_stem()
                .map_or(properties.name, |name| name.to_string_lossy().into_owned())
        } else {
            properties.name
        };
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

    fn restore_after_scan(
        &mut self,
        ctx: &egui::Context,
        saved: SavedJob,
        recommendation: Option<Selection>,
    ) {
        let scan = self.scan.as_ref().expect("fresh inspection");
        if !scan.complete
            || scan.is_prepared()
            || scan
                .issues
                .iter()
                .any(|issue| issue.severity == IssueSeverity::Error)
        {
            return;
        }
        let eligible = saved.selected.as_ref().filter(|path| {
            scan.candidates.iter().any(|candidate| {
                &candidate.relative_path == *path
                    && matches!(
                        candidate.architecture,
                        portableforge_core::snapshot::PeArch::X86
                            | portableforge_core::snapshot::PeArch::X64
                    )
            })
        });
        if let Some(path) = eligible.cloned() {
            if let Some(selection) = recommendation.filter(|selection| selection.relative == path) {
                self.accept_selection(ctx, selection);
                self.restore_choices(saved);
            } else {
                self.page = Page::Program;
                self.select(ctx, path);
                if self.busy() {
                    self.restoring = Some(saved);
                }
            }
            return;
        }

        let has_candidates = !scan.candidates.is_empty();
        self.restore_locations(&saved);
        if saved.selected.is_some() {
            self.status =
                Some("The saved start file is unavailable. Choose the file that starts the app.");
            if has_candidates {
                self.page = Page::Program;
            }
        } else if let Some(selection) = recommendation {
            self.accept_selection(ctx, selection);
        } else if has_candidates {
            self.page = Page::Program;
        }
    }

    fn restore_locations(&mut self, saved: &SavedJob) {
        self.launcher_tools.clone_from(&saved.launcher_tools);
        self.installer_tools.clone_from(&saved.installer_tools);
        self.output_parent.clone_from(&saved.output_parent);
    }

    fn restore_choices(&mut self, saved: SavedJob) {
        self.restore_locations(&saved);
        self.draft = saved.draft;
        self.draft.rights = false;
        self.inferred_name = false;
    }

    fn can_save_for_later(&self) -> bool {
        self.mode == IntakeMode::Autonomous
            && matches!(self.page, Page::Review | Page::Program)
            && self.source_kind.is_some()
            && self
                .scan
                .as_ref()
                .is_some_and(|scan| scan.complete && !scan.is_prepared())
            && !self.busy()
            && !self.modal_open()
    }

    pub(super) fn poll(&mut self, ctx: &egui::Context) -> Option<PathBuf> {
        if let Some(after) = self.discard_stage.take() {
            let stage = self.staged.take();
            self.start(ctx, copy::CLEANING_SOURCE, move |_, _| {
                Ok(Outcome::StageDiscarded(after, dispose_stage(stage)))
            });
            if !self.busy() {
                self.discard_failed();
            }
        }
        let job = self.job.as_ref()?;
        let result = loop {
            match job.try_recv() {
                Ok(Event::Stage(status)) => {
                    if !self.stop_pending && !self.test_stopping() {
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
        let intake_cancelled = self
            .intake_stop
            .take()
            .is_some_and(|stop| stop.load(Ordering::Acquire));
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
        if intake_cancelled {
            self.restoring = None;
            match result {
                Ok(Outcome::Inspected(inspection)) => {
                    self.staged = inspection.staged;
                    self.discard(DiscardStage::Stopped);
                }
                Ok(_) => self.status = Some(copy::SOURCE_STOPPED),
                Err(failure) => self.error = Some(failure),
            }
            ctx.request_repaint();
            return None;
        }
        match result {
            Ok(Outcome::DroppedSetup(path)) => {
                self.open = false;
                return Some(path);
            }
            Ok(Outcome::IntakeStopped) => self.status = Some(copy::SOURCE_STOPPED),
            Ok(Outcome::StageDiscarded(after, Ok(()))) => self.after_discard(after),
            Ok(Outcome::StageDiscarded(_, Err(failure))) => {
                self.discard_failed();
                self.error = Some(failure);
            }
            Ok(Outcome::SavedForLater) => self.discard(DiscardStage::Saved),
            Ok(Outcome::Inspected(inspection)) => {
                self.source = inspection.original;
                self.source_label = inspection.source_label;
                self.source_kind = Some(inspection.source_kind);
                self.output_parent = if inspection.source_kind == SourceKind::Files {
                    self.source.clone()
                } else {
                    self.source
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_default()
                };
                self.staged = inspection.staged;
                self.output_suffix = SystemTime::now().duration_since(UNIX_EPOCH).map_or_else(
                    |_| std::process::id().to_string(),
                    |elapsed| format!("{:x}", elapsed.as_nanos() & 0x0000_ffff_ffff_ffff),
                );
                self.issues = inspection.issues;
                self.scan = Some(inspection.scan);
                if let Some(saved) = self.restoring.take() {
                    self.restore_after_scan(ctx, saved, inspection.selection);
                } else if let Some(selection) = inspection.selection {
                    self.accept_selection(ctx, selection);
                } else if self.mode == IntakeMode::Autonomous
                    && self
                        .blocker()
                        .is_some_and(|(page, _)| page == Page::Program)
                {
                    self.page = Page::Program;
                }
            }
            Ok(Outcome::Selected(selection, issues)) => {
                self.issues = issues;
                self.accept_selection(ctx, selection);
                if let Some(saved) = self.restoring.take() {
                    self.restore_choices(saved);
                }
            }
            #[cfg(all(windows, target_arch = "x86_64"))]
            Ok(Outcome::Built(delivery)) => {
                self.result = Some(delivery);
                self.page = Page::Done;
                self.discard(DiscardStage::Built);
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
            Err(failure) => {
                self.restoring = None;
                self.error = Some(failure);
            }
        }
        if self.discard_stage.is_some() {
            ctx.request_repaint();
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

    fn autonomous_ready(&self) -> bool {
        self.mode == IntakeMode::Autonomous
            && !self.build_attempted
            && self.result.is_none()
            && self.build_ready()
    }

    pub(super) fn advance_autonomously(&mut self, ctx: &egui::Context) {
        if self.autonomous_ready()
            && !ctx.input(|input| input.viewport().close_requested())
            && ctx.memory(|memory| memory.top_modal_layer().is_none())
        {
            self.build(ctx);
        }
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
        let staged = self.staged.clone();
        let control = FolderBuildControl::default();
        self.control = Some(control.clone());
        self.build_attempted = true;
        self.start(ctx, "Checking the app files…", move |tx, ctx| {
            let _staged = staged;
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
        if self.stop_pending {
            return false;
        }
        if self.intake_stop.is_some() {
            return true;
        }
        #[cfg(all(windows, target_arch = "x86_64"))]
        {
            self.control.is_some()
        }
        #[cfg(not(all(windows, target_arch = "x86_64")))]
        false
    }

    pub(super) fn stop(&mut self) {
        if let Some(stop) = &self.intake_stop {
            stop.store(true, Ordering::Release);
            self.stop_pending = true;
            self.status = Some(copy::SOURCE_STOPPING);
            return;
        }
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
    fn save_app_job(&mut self, ctx: &egui::Context) {
        let flow = &mut self.folder_flow;
        if !flow.can_save_for_later() {
            return;
        }
        let path = saved_job::path(&self.settings_path);
        let revision = match saved_job::revision(&path) {
            Ok(revision) => revision,
            Err(error) => {
                flow.error = Some(Failure::new("Could not access the saved job.", error));
                return;
            }
        };
        let replace = revision.is_some();
        if replace
            && !confirm_saved_job(
                "Replace saved job?",
                "Save this job in place of the currently saved app job?",
            )
        {
            return;
        }
        let job = SavedJob::from_flow(flow, flow.source_kind.expect("inspected source"));
        flow.start(ctx, saved_job::SAVING, move |_, _| {
            saved_job::save(&path, revision, &job).map_err(|error| {
                Failure::new(
                    "Could not save the app job. Open Show Details for the reason.",
                    error,
                )
            })?;
            Ok(Outcome::SavedForLater)
        });
    }

    fn resume_app_job(&mut self, ctx: &egui::Context) {
        if self.folder_flow.busy() || self.folder_flow.modal_open() || self.profile_flow.busy() {
            return;
        }
        match saved_job::load(&saved_job::path(&self.settings_path)) {
            Ok(job) => {
                let paths = job.inputs.clone();
                self.folder_flow.mode = IntakeMode::Autonomous;
                self.folder_flow.inspect(ctx, paths, false, Some(job.kind));
                if self.folder_flow.busy() {
                    self.folder_flow.restoring = Some(job);
                }
            }
            Err(error) => {
                self.folder_flow.error = Some(Failure::new(
                    "The saved job could not be opened. Remove it or choose the app files again.",
                    error,
                ));
            }
        }
    }

    fn remove_app_job(&mut self) {
        let path = saved_job::path(&self.settings_path);
        let result = saved_job::revision(&path).and_then(|revision| {
            if let Some(revision) = revision {
                if confirm_saved_job(
                    "Remove saved job?",
                    "Remove the saved choices? Your app files stay in place.",
                ) {
                    saved_job::remove(&path, revision)?;
                }
            }
            Ok(())
        });
        self.folder_flow.error = result.err().map(|error| {
            Failure::new(
                "Could not remove the saved job. Open Show Details for the reason.",
                error,
            )
        });
        self.folder_flow.status = None;
    }

    pub(super) fn choose_app_folder(&mut self, ctx: &egui::Context) {
        let dialog = rfd::FileDialog::new().set_title("Choose the unpacked app folder");
        if let Some(path) = dialog.pick_folder() {
            self.inspect_app_inputs(ctx, vec![path], false);
        }
    }

    pub(super) fn choose_app_files(&mut self, ctx: &egui::Context) {
        let dialog = rfd::FileDialog::new().set_title("Choose a ZIP or files for one app");
        if let Some(paths) = dialog.pick_files() {
            self.inspect_app_inputs(ctx, paths, false);
        }
    }

    pub(super) fn inspect_dropped_files(
        &mut self,
        ctx: &egui::Context,
        files: Vec<egui::DroppedFile>,
    ) {
        if files.is_empty()
            || self.folder_flow.busy()
            || self.folder_flow.modal_open()
            || self.profile_flow.busy()
        {
            return;
        }
        if let Some(paths) = files
            .into_iter()
            .map(|file| file.path)
            .collect::<Option<Vec<_>>>()
        {
            self.inspect_app_inputs(ctx, paths, true);
        } else {
            let old_stage = self.folder_flow.staged.take();
            self.folder_flow.reset_source(PathBuf::new());
            self.folder_flow
                .start(ctx, copy::CHECKING_SELECTION, move |_, _| {
                    dispose_stage(old_stage)?;
                    Err(Failure::new(
                        copy::SAVE_DROPPED_FILES,
                        "One or more dropped items did not have a local file path.",
                    ))
                });
        }
    }

    fn inspect_app_inputs(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>, allow_setup: bool) {
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
        self.folder_flow.inspect(ctx, paths, allow_setup, None);
    }
}

fn confirm_saved_job(title: &str, description: &str) -> bool {
    rfd::MessageDialog::new()
        .set_title(title)
        .set_description(description)
        .set_buttons(rfd::MessageButtons::YesNo)
        .show()
        == rfd::MessageDialogResult::Yes
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
            source_label: "Widget.zip".into(),
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
    fn autonomous_requires_a_ready_review_and_never_retries_or_remembers_permission() {
        let mut wizard = populated();
        let flow = &mut wizard.folder_flow;
        assert_eq!(flow.mode, IntakeMode::Guided);
        assert!(!flow.autonomous_ready());
        flow.mode = IntakeMode::Autonomous;
        flow.draft.rights = false;
        assert!(!flow.autonomous_ready());
        flow.draft.rights = true;
        let selected = flow.selected.take();
        assert!(!flow.autonomous_ready());
        flow.selected = selected;
        flow.page = Page::App;
        assert!(!flow.autonomous_ready());
        flow.page = Page::Review;
        assert_eq!(
            flow.autonomous_ready(),
            cfg!(all(windows, target_arch = "x86_64"))
        );
        flow.build_attempted = true;
        assert!(!flow.autonomous_ready());
        flow.reset_source(PathBuf::from("new source"));
        assert_eq!(flow.mode, IntakeMode::Autonomous);
        assert!(!flow.draft.rights);
        assert!(!flow.autonomous_ready());
    }

    #[test]
    fn intake_cancellation_preserves_a_cleanup_error() {
        let failure = intake_failure(ScanError {
            message: "Input staging stopped.\nCould not remove staging at the temporary path."
                .into(),
        });
        assert!(matches!(
            failure,
            Err(Failure {
                message: copy::SOURCE_CLEANUP_FAILED,
                ..
            })
        ));
    }

    fn inert_desktop_pe() -> [u8; 368] {
        let mut pe = [0; 368];
        pe[..2].copy_from_slice(b"MZ");
        pe[60] = 64;
        pe[64..68].copy_from_slice(b"PE\0\0");
        pe[68..70].copy_from_slice(&0x8664u16.to_le_bytes());
        pe[70] = 1;
        pe[84] = 240;
        pe[86] = 2;
        pe[88..90].copy_from_slice(&0x20bu16.to_le_bytes());
        pe[156] = 2;
        pe
    }

    struct AppJobFixture {
        root: PathBuf,
        folder: PathBuf,
        app: PathBuf,
        config: PathBuf,
        zip: PathBuf,
    }

    impl AppJobFixture {
        fn new() -> Self {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root =
                std::env::temp_dir().join(format!("pf-app-job-{}-{stamp}", std::process::id()));
            let folder = root.join("App");
            let app = folder.join("Widget.exe");
            let config = folder.join("settings.ini");
            let zip = root.join("Widget.zip");
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(&app, inert_desktop_pe()).unwrap();
            std::fs::write(&config, "example=true").unwrap();
            portableforge_core::build::write_package_zip(&folder, &zip).unwrap();
            Self {
                root,
                folder,
                app,
                config,
                zip,
            }
        }

        fn wizard(&self) -> WizardApp {
            WizardApp {
                settings_path: self.root.join("state/wizard-settings.json"),
                settings: Default::default(),
                resume_offer: false,
                ..Default::default()
            }
        }
    }

    impl Drop for AppJobFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn finish_folder_work(flow: &mut Flow, ctx: &egui::Context) {
        let expires = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while flow.busy() {
            assert!(std::time::Instant::now() < expires, "Folder job timed out");
            assert!(
                flow.poll(ctx).is_none(),
                "Saved app job became a setup task"
            );
            std::thread::yield_now();
        }
    }

    #[test]
    fn autonomous_jobs_save_fresh_zip_or_exact_loose_selection_and_recheck_rights() {
        for kind in [SourceKind::Zip, SourceKind::Files] {
            let fixture = AppJobFixture::new();
            let ctx = egui::Context::default();
            let paths = match kind {
                SourceKind::Zip => vec![fixture.zip.clone()],
                SourceKind::Files => vec![fixture.app.clone(), fixture.config.clone()],
                SourceKind::Folder => unreachable!(),
            };
            std::fs::write(fixture.folder.join("Excluded.txt"), "keep out").unwrap();
            let mut wizard = fixture.wizard();
            wizard.settings.draft = Some(portableforge_core::settings::WizardDraft {
                step: "pick".into(),
                installer_path: "example setup.exe".into(),
                ..Default::default()
            });
            portableforge_core::settings::save(&wizard.settings_path, &wizard.settings).unwrap();
            let setup_draft = std::fs::read(&wizard.settings_path).unwrap();
            wizard.folder_flow.mode = IntakeMode::Autonomous;
            wizard.inspect_app_inputs(&ctx, paths.clone(), false);
            finish_folder_work(&mut wizard.folder_flow, &ctx);
            let flow = &mut wizard.folder_flow;
            assert!(flow.can_save_for_later());
            assert_eq!(flow.source_kind, Some(kind));
            flow.draft.name = "Reviewed Widget".into();
            flow.draft.version = "4.1".into();
            flow.draft.rights = true;
            flow.build_attempted = true;
            flow.launcher_tools = fixture.root.join("Launcher");
            flow.installer_tools = fixture.root.join("Installer");
            flow.output_parent = fixture.root.join("Output");
            let first_stage = flow.staged.as_ref().unwrap().root().to_owned();
            wizard.save_app_job(&ctx);
            finish_folder_work(&mut wizard.folder_flow, &ctx);
            assert!(!wizard.folder_flow.open);
            assert!(!first_stage.exists());

            let file = saved_job::path(&wizard.settings_path);
            let persisted = saved_job::load(&file).unwrap();
            assert_eq!(persisted.inputs, paths);
            assert_eq!(persisted.kind, kind);
            assert!(!persisted.draft.rights);
            assert_eq!(std::fs::read(&wizard.settings_path).unwrap(), setup_draft);
            let mut json: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
            json["draft"]["rights"] = serde_json::Value::Bool(true);
            std::fs::write(&file, serde_json::to_vec(&json).unwrap()).unwrap();
            let mut reopened = fixture.wizard();
            assert!(!reopened.folder_flow.open);
            reopened.resume_app_job(&ctx);
            finish_folder_work(&mut reopened.folder_flow, &ctx);
            let flow = &mut reopened.folder_flow;
            let stage = flow.staged.as_ref().unwrap().root().to_owned();
            assert_ne!(first_stage, stage);
            assert!(stage.join("settings.ini").is_file());
            assert!(!stage.join("Excluded.txt").exists());
            assert_eq!(flow.selected.as_deref(), Some(Path::new("Widget.exe")));
            assert_eq!(flow.draft.name, "Reviewed Widget");
            assert_eq!(flow.draft.version, "4.1");
            assert_eq!(flow.launcher_tools, fixture.root.join("Launcher"));
            assert_eq!(flow.installer_tools, fixture.root.join("Installer"));
            assert_eq!(flow.output_parent, fixture.root.join("Output"));
            assert!(!flow.draft.rights);
            assert!(!flow.build_attempted);
            assert!(!flow.autonomous_ready());
            assert!(reopened.installer_path.is_empty());
            assert!(file.is_file());
            reopened.folder_flow.back();
            finish_folder_work(&mut reopened.folder_flow, &ctx);
            assert!(!stage.exists());
        }
    }

    #[test]
    fn saved_app_job_rejects_missing_or_reclassified_sources_and_reopens_ambiguous_choice() {
        let fixture = AppJobFixture::new();
        let ctx = egui::Context::default();
        let mut wizard = fixture.wizard();
        let extra = fixture.folder.join("Other.exe");
        std::fs::write(&extra, inert_desktop_pe()).unwrap();
        wizard.folder_flow.mode = IntakeMode::Autonomous;
        wizard.inspect_app_inputs(&ctx, vec![fixture.folder.clone()], false);
        finish_folder_work(&mut wizard.folder_flow, &ctx);
        assert_eq!(wizard.folder_flow.page, Page::Program);
        assert!(wizard.folder_flow.selected.is_none());
        wizard.folder_flow.select(&ctx, "Other.exe".into());
        finish_folder_work(&mut wizard.folder_flow, &ctx);
        wizard.folder_flow.draft.name = "Other Reviewed".into();
        wizard.save_app_job(&ctx);
        finish_folder_work(&mut wizard.folder_flow, &ctx);

        let mut reopened = fixture.wizard();
        reopened.resume_app_job(&ctx);
        finish_folder_work(&mut reopened.folder_flow, &ctx);
        assert_eq!(
            reopened.folder_flow.selected.as_deref(),
            Some(Path::new("Other.exe"))
        );
        assert_eq!(reopened.folder_flow.draft.name, "Other Reviewed");

        std::fs::remove_file(&extra).unwrap();
        reopened.resume_app_job(&ctx);
        finish_folder_work(&mut reopened.folder_flow, &ctx);
        assert!(reopened.folder_flow.selected.is_none());
        assert_eq!(reopened.folder_flow.page, Page::Program);
        assert_ne!(reopened.folder_flow.draft.name, "Other Reviewed");
        assert!(!reopened.folder_flow.build_ready());

        std::fs::rename(&fixture.folder, fixture.root.join("Unavailable")).unwrap();
        reopened.resume_app_job(&ctx);
        finish_folder_work(&mut reopened.folder_flow, &ctx);
        assert!(reopened.folder_flow.error.is_some());
        assert!(reopened.folder_flow.scan.is_none());
        assert!(saved_job::path(&reopened.settings_path).is_file());

        let setup = fixture.root.join("DifferentSetup.exe");
        std::fs::write(&setup, "inert").unwrap();
        let job_file = saved_job::path(&reopened.settings_path);
        let mut job = saved_job::load(&job_file).unwrap();
        job.inputs = vec![setup];
        saved_job::save(&job_file, saved_job::revision(&job_file).unwrap(), &job).unwrap();
        reopened.resume_app_job(&ctx);
        finish_folder_work(&mut reopened.folder_flow, &ctx);
        assert!(reopened.folder_flow.error.is_some());
        assert!(reopened.folder_flow.scan.is_none());
        assert!(reopened.installer_path.is_empty());
        assert!(job_file.is_file());
    }

    #[test]
    fn stale_or_invalid_saved_job_writes_keep_the_current_snapshot() {
        let fixture = AppJobFixture::new();
        let path = saved_job::path(&fixture.wizard().settings_path);
        let flow = Flow {
            original_inputs: vec![fixture.zip.clone()],
            ..Default::default()
        };
        let mut job = SavedJob::from_flow(&flow, SourceKind::Zip);
        saved_job::save(&path, None, &job).unwrap();
        let first = saved_job::revision(&path).unwrap().unwrap();
        job.draft.name = "new revision".into();
        saved_job::save(&path, Some(first), &job).unwrap();
        let current = std::fs::read(&path).unwrap();
        assert!(saved_job::save(&path, Some(first), &job).is_err());
        assert!(saved_job::remove(&path, first).is_err());
        job.selected = Some("../outside.exe".into());
        let replacement = saved_job::revision(&path).unwrap().unwrap();
        assert!(saved_job::save(&path, Some(replacement), &job).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), current);
        saved_job::remove(&path, replacement).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn raw_drop_routes_folder_setup_zip_and_entire_loose_selection() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("pf-ui-drop-{}-{stamp}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let folder = root.join("Widget.exe");
        std::fs::create_dir(&folder).unwrap();
        let application = folder.join("Widget.exe");
        std::fs::write(&application, inert_desktop_pe()).unwrap();
        let companion = folder.join("settings.ini");
        std::fs::write(&companion, b"example=true").unwrap();
        let zip = root.join("Widget.zip");
        portableforge_core::build::write_package_zip(&folder, &zip).unwrap();
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
        let drop_input = |paths: &[Option<&Path>]| egui::RawInput {
            dropped_files: paths
                .iter()
                .map(|path| egui::DroppedFile {
                    path: path.map(Path::to_path_buf),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let mut drive = |wizard: &mut WizardApp, input: egui::RawInput| {
            let output = ctx.run(input, |ctx| eframe::App::update(wizard, ctx, &mut frame));
            let mut commands = output.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .clone();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while wizard.folder_flow.busy() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "drop worker timed out"
                );
                std::thread::yield_now();
                commands.extend(
                    ctx.run(egui::RawInput::default(), |ctx| {
                        eframe::App::update(wizard, ctx, &mut frame);
                    })
                    .viewport_output[&egui::ViewportId::ROOT]
                        .commands
                        .iter()
                        .cloned(),
                );
            }
            commands
        };
        drive(&mut wizard, drop_input(&[Some(&folder)]));
        assert!(wizard.folder_flow.open);
        assert!(wizard.folder_flow.scan.is_some());
        assert!(wizard.installer_path.is_empty());
        wizard.folder_flow.back();
        drive(&mut wizard, drop_input(&[Some(&setup)]));
        assert!(!wizard.folder_flow.open);
        assert_eq!(Path::new(&wizard.installer_path), &setup);

        drive(&mut wizard, drop_input(&[Some(&application), None]));
        assert_eq!(Path::new(&wizard.installer_path), &setup);
        assert!(wizard.folder_flow.scan.is_none());
        assert_eq!(
            wizard.folder_flow.error.as_ref().unwrap().message,
            copy::SAVE_DROPPED_FILES
        );
        drive(&mut wizard, drop_input(&[Some(&zip)]));
        assert_eq!(wizard.folder_flow.source, zip);
        assert_eq!(wizard.folder_flow.source_label, "Widget.zip");
        assert_eq!(wizard.folder_flow.output_parent, root);
        assert_eq!(
            wizard.folder_flow.selected.as_deref(),
            Some(Path::new("Widget.exe"))
        );
        let stage = wizard
            .folder_flow
            .staged
            .as_ref()
            .unwrap()
            .root()
            .to_owned();
        assert!(stage.join("settings.ini").is_file());
        let mut close = egui::RawInput::default();
        close
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .events
            .push(egui::ViewportEvent::Close);
        let commands = drive(&mut wizard, close);
        assert!(commands.contains(&egui::ViewportCommand::CancelClose));
        assert!(commands.contains(&egui::ViewportCommand::Close));
        assert!(!stage.exists());

        drive(
            &mut wizard,
            drop_input(&[Some(&application), Some(&companion)]),
        );
        assert_eq!(wizard.folder_flow.output_parent, folder);
        let stage = wizard
            .folder_flow
            .staged
            .as_ref()
            .unwrap()
            .root()
            .to_owned();
        assert_eq!(
            std::fs::read(stage.join("settings.ini")).unwrap(),
            b"example=true"
        );
        wizard.folder_flow.back();
        drive(&mut wizard, egui::RawInput::default());
        assert!(!stage.exists());
        assert_eq!(std::fs::read(&companion).unwrap(), b"example=true");

        std::fs::remove_file(zip).unwrap();
        std::fs::remove_file(application).unwrap();
        std::fs::remove_file(companion).unwrap();
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
        wizard.inspect_dropped_files(
            &ctx,
            vec![egui::DroppedFile {
                path: Some(source.join("Another")),
                ..Default::default()
            }],
        );
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
            let fixture = AppJobFixture::new();
            let mut wizard = populated();
            wizard.settings_path = fixture.wizard().settings_path;
            let path = saved_job::path(&wizard.settings_path);
            let snapshot = Flow {
                original_inputs: vec![fixture.zip.clone()],
                ..Default::default()
            };
            saved_job::save(
                &path,
                None,
                &SavedJob::from_flow(&snapshot, SourceKind::Zip),
            )
            .unwrap();
            wizard.settings.text_scale = scale;
            let ctx = egui::Context::default();
            super::super::theme::install_fonts(&ctx);
            let bounds = egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::Vec2::from(super::super::dense::MIN_CONTENT),
            );
            let mut frame = eframe::Frame::_new_kittest();
            wizard.folder_flow.open = false;
            wizard.resume_offer = true;
            wizard.settings.welcomed = true;
            {
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
                for label in [
                    copy::GUIDED,
                    copy::AUTONOMOUS,
                    copy::CHOOSE_FILES,
                    copy::CHOOSE_FOLDER,
                    saved_job::RESUME,
                    saved_job::REMOVE,
                    "Pick up where I left off",
                ] {
                    let shape = output.shapes.iter().find(|shape| {
                        matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == label)
                    }).unwrap_or_else(|| panic!("Missing intake action {label:?} at {scale}"));
                    assert!(bounds
                        .intersect(shape.clip_rect)
                        .expand(1.0)
                        .contains_rect(shape.shape.visual_bounding_rect()));
                }
            }
            wizard.folder_flow.open = true;
            wizard.folder_flow.mode = IntakeMode::Autonomous;
            wizard.folder_flow.source_kind = Some(SourceKind::Zip);
            wizard.folder_flow.build_attempted = true;
            for (page, labels) in [
                (
                    Page::Review,
                    &[
                        "Review Your Package",
                        "App Files · Widget.zip",
                        #[cfg(windows)]
                        "Edit Package Artwork…",
                        "Build Installer",
                        saved_job::SAVE,
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
