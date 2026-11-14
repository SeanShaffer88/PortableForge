//! The wizard's verified capture, approval and rebuild controls.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use eframe::egui;
#[cfg(all(windows, target_arch = "x86_64"))]
use portableforge_core::orchestrate::{CaptureOutcome, SessionLayout};
#[cfg(all(windows, target_arch = "x86_64"))]
use portableforge_core::sbie_verified::CancellationToken;
use portableforge_core::settings::{self, CleanedCapture, SavedBuild, SavedProfileApproval};
use portableforge_core::SandboxMode;

use super::{
    dense, file_picker, picker_start_dir, setup_input, shell, theme, SetupInput, SetupKind,
    WizardApp,
};
#[cfg(all(windows, target_arch = "x86_64"))]
use super::{installer_test, sessions_base, unique_folder};

#[derive(Default)]
pub(super) struct ProfileFlow {
    pub(super) selected_capture: bool,
    pub(super) captured: bool,
    installed_path: String,
    setup_args: Vec<String>,
    setup_arg_page: usize,
    setup_folders_open: bool,
    companions: Vec<PathBuf>,
    setup_options_open: bool,
    capture_parent: String,
    capture_setup: String,
    trusted: bool,
    prerequisites: Option<Prerequisites>,
    pub(super) msi_ready: Option<Result<(), String>>,
    pub(super) cleaned_capture: Option<CleanedCapture>,
    #[cfg(all(windows, target_arch = "x86_64"))]
    cancellation: Option<CancellationToken>,
    #[cfg(all(windows, target_arch = "x86_64"))]
    candidate: Option<CandidateChoice>,
    launcher_tools: String,
    profile_destination: String,
    saved: Option<SavedProfileApproval>,
    approval_error: Option<String>,
    pub(super) replay_open: bool,
    selected_profile: usize,
    profile_page: usize,
    profile_search: String,
    profile_actions_open: bool,
    relocate_destination: String,
    pub(super) update_base: Option<SavedProfileApproval>,
    update_details: Option<String>,
    replay_setup: String,
    installer_tools: String,
    replay_destination: String,
    build: Option<SavedBuild>,
    library_tab: LibraryTab,
    result_page: usize,
    queue: Vec<QueuedBuild>,
    queue_page: usize,
    active_queue: Option<usize>,
    queue_running: bool,
    build_details: Option<(&'static str, String)>,
    build_details_page: usize,
    runtime_confirm: Option<SavedBuild>,
    job: Option<Receiver<JobEvent>>,
    status: &'static str,
    error: Option<String>,
}

impl ProfileFlow {
    pub(super) fn folder_tool_paths(&self) -> (PathBuf, PathBuf) {
        (
            entered(&self.launcher_tools),
            entered(&self.installer_tools),
        )
    }
}

enum JobEvent {
    Stage(&'static str),
    #[cfg(all(windows, target_arch = "x86_64"))]
    Choose(CandidateChoice),
    Done(Box<Result<JobResult, String>>),
}

#[cfg(all(windows, target_arch = "x86_64"))]
struct CandidateChoice {
    paths: Vec<PathBuf>,
    page: usize,
    reply: mpsc::Sender<PathBuf>,
}

struct Prerequisites {
    capture: Result<(), String>,
    launcher: Option<Result<(), String>>,
    installer: Option<Result<(), String>>,
}

enum JobResult {
    #[cfg(all(windows, target_arch = "x86_64"))]
    Captured(Box<CaptureComplete>),
    Checked(Prerequisites),
    MsiChecked(Result<(), String>),
    #[cfg(windows)]
    MsiStarted(portableforge_core::orchestrate::SessionLayout),
    #[cfg(all(windows, target_arch = "x86_64"))]
    Saved(SavedProfileApproval),
    #[cfg(all(windows, target_arch = "x86_64"))]
    Rebuilt(Box<(SavedProfileApproval, SavedBuild)>),
    #[cfg(all(windows, target_arch = "x86_64"))]
    Relocated(SavedProfileApproval, PathBuf),
    #[cfg(all(windows, target_arch = "x86_64"))]
    UpdateCompared(String),
    #[cfg(all(windows, target_arch = "x86_64"))]
    BuildDetails(&'static str, String),
}

#[cfg(all(windows, target_arch = "x86_64"))]
struct CaptureComplete {
    layout: SessionLayout,
    outcome: CaptureOutcome,
    installed: PathBuf,
    receipt: CleanedCapture,
    reopened: bool,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum LibraryTab {
    #[default]
    Profiles,
    Queue,
    Results,
}

impl LibraryTab {
    fn label(self) -> &'static str {
        match self {
            Self::Profiles => "Saved profiles",
            Self::Queue => "Build queue",
            Self::Results => "Build results",
        }
    }
}

#[derive(Clone, Copy)]
enum RecordStatus {
    Neutral,
    Active,
    Complete,
    Failed,
}

impl RecordStatus {
    fn color(self, ui: &egui::Ui) -> egui::Color32 {
        match self {
            Self::Neutral => ui.visuals().weak_text_color(),
            Self::Active => theme::info(ui),
            Self::Complete => theme::success(ui),
            Self::Failed => ui.visuals().error_fg_color,
        }
    }
}

fn table_row_height(ui: &egui::Ui) -> f32 {
    (ui.text_style_height(&egui::TextStyle::Small) + 10.0).max(28.0)
}

fn record_page(
    ui: &mut egui::Ui,
    anchor: usize,
    total: usize,
    reserved: f32,
    label: &str,
) -> std::ops::Range<usize> {
    let row = table_row_height(ui);
    let rows =
        (((ui.available_height() - reserved - row - 34.0) / row).floor() as usize).clamp(1, 20);
    let mut page = anchor / rows;
    dense::page(ui, &mut page, total, rows, label)
}

fn record_table(
    ui: &mut egui::Ui,
    id: &'static str,
    headings: [&str; 3],
    range: std::ops::Range<usize>,
    total: usize,
    selected: usize,
    row: impl Fn(usize) -> ([String; 3], RecordStatus),
) -> usize {
    let selected = if range.contains(&selected) {
        selected
    } else {
        range.start
    };
    ui.push_id(id, |ui| {
        let mut next = selected;
        ui.scope(|ui| {
            let label = headings.join(", ");
            ui.ctx().accesskit_node_builder(ui.unique_id(), |node| {
                node.set_role(egui::accesskit::Role::Grid);
                node.set_label(label.as_str());
                node.set_row_count(total);
                node.set_column_count(headings.len());
            });
            ui.spacing_mut().item_spacing.y = 0.0;
            let row_height = table_row_height(ui);
            let width = ui.available_width();
            let columns = [0.0, width * 0.45, width * 0.70, width];
            let edge = ui.visuals().widgets.noninteractive.bg_stroke;
            let selection = ui.visuals().selection;
            let painter = ui.painter().clone();
            let mut last = None;
            for item in std::iter::once(None).chain(range.clone().map(Some)) {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(width, row_height), egui::Sense::hover());
                let Some(index) = item else {
                    painter.rect_filled(rect, 0, ui.visuals().faint_bg_color);
                    paint_record_cells(
                        ui,
                        rect,
                        columns,
                        headings,
                        ui.visuals().text_color(),
                        true,
                        edge,
                    );
                    last = Some(rect);
                    continue;
                };
                let (cells, status) = row(index);
                let response =
                    ui.interact(rect, ui.make_persistent_id(index), egui::Sense::click());
                let active = index == selected;
                let description = cells.join(", ");
                response.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::SelectableLabel,
                        ui.is_enabled(),
                        active,
                        &description,
                    )
                });
                ui.ctx().accesskit_node_builder(response.id, |node| {
                    node.set_role(egui::accesskit::Role::Row);
                    node.set_row_index(index);
                    node.set_selected(active);
                });
                if active || response.hovered() {
                    let fill = if active {
                        selection.bg_fill
                    } else {
                        ui.visuals().widgets.hovered.weak_bg_fill
                    };
                    painter.rect_filled(rect, 0, fill);
                }
                paint_record_cells(
                    ui,
                    rect,
                    columns,
                    cells.each_ref().map(String::as_str),
                    status.color(ui),
                    false,
                    edge,
                );
                if active {
                    let indicator = egui::Rect::from_min_max(
                        rect.left_top(),
                        egui::pos2(rect.left() + 3.0, rect.bottom()),
                    );
                    painter.rect_filled(indicator, 0, selection.stroke.color);
                }
                if response.has_focus() {
                    theme::focus_outline(ui, &response);
                    for (key, target) in [
                        (egui::Key::ArrowUp, index.saturating_sub(1)),
                        (egui::Key::ArrowDown, (index + 1).min(total - 1)),
                        (egui::Key::Home, 0),
                        (egui::Key::End, total - 1),
                        (egui::Key::PageUp, index.saturating_sub(range.len())),
                        (egui::Key::PageDown, (index + range.len()).min(total - 1)),
                    ] {
                        if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, key)) {
                            next = target;
                            let id = ui.make_persistent_id(target);
                            ui.memory_mut(|memory| memory.request_focus(id));
                        }
                    }
                }
                if response.clicked() {
                    next = index;
                }
                dense::field_help(&response, &description);
                last = Some(rect);
            }
            if let Some(rect) = last {
                painter.hline(rect.x_range(), rect.bottom(), edge);
            }
        });
        next
    })
    .inner
}

fn paint_record_cells(
    ui: &egui::Ui,
    rect: egui::Rect,
    columns: [f32; 4],
    cells: [&str; 3],
    status: egui::Color32,
    header: bool,
    edge: egui::Stroke,
) {
    let painter = ui.painter();
    painter.hline(rect.x_range(), rect.top(), edge);
    for (index, cell) in cells.into_iter().enumerate() {
        let left = rect.left() + columns[index];
        painter.vline(left, rect.y_range(), edge);
        let text_color = if index == 2 {
            status
        } else {
            ui.visuals().text_color()
        };
        let mut text = egui::RichText::new(cell).color(text_color);
        if header {
            text = text.strong();
        }
        let galley = egui::WidgetText::from(text).into_galley(
            ui,
            Some(egui::TextWrapMode::Truncate),
            (columns[index + 1] - columns[index] - 16.0).max(1.0),
            egui::TextStyle::Small,
        );
        painter.galley(
            egui::pos2(left + 8.0, rect.center().y - galley.size().y / 2.0),
            galley,
            text_color,
        );
    }
    painter.vline(rect.right(), rect.y_range(), edge);
}

#[derive(Clone)]
struct ReplayRequest {
    setup: PathBuf,
    approval: SavedProfileApproval,
    tools: PathBuf,
    destination: PathBuf,
}

struct QueuedBuild {
    request: ReplayRequest,
    state: QueueState,
}

enum QueueState {
    Waiting,
    Building,
    #[cfg(all(windows, target_arch = "x86_64"))]
    Built(SavedBuild),
    Failed(String),
}

impl ProfileFlow {
    #[cfg(test)]
    pub(super) fn populate_prerequisites_fixture(&mut self) {
        self.prerequisites = Some(Prerequisites {
            capture: Err("The dedicated box is unavailable on this computer.".into()),
            launcher: Some(Ok(())),
            installer: None,
        });
    }

    #[cfg(test)]
    pub(super) fn set_layout_paths(&mut self, paths: &[String; 3]) {
        self.replay_setup.clone_from(&paths[0]);
        self.installer_tools.clone_from(&paths[1]);
        self.replay_destination.clone_from(&paths[2]);
    }

    #[cfg(test)]
    pub(super) fn populate_layout_fixture(&mut self, busy: bool) -> impl FnOnce() {
        self.replay_setup = format!(r"C:\Setups\{}\app.exe", "long-folder\\".repeat(18));
        self.installer_tools = r"C:\PortableApps\Installer".into();
        self.replay_destination = r"C:\Builds\App".into();
        let sender = if busy {
            let (sender, receiver) = mpsc::channel();
            self.job = Some(receiver);
            self.status = "Building your installer…";
            Some(sender)
        } else {
            self.error = Some("The test detected an error in a file.\n".repeat(100));
            None
        };
        move || drop(sender)
    }

    pub(super) fn activity_problem(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub(super) fn activity_signal(&self) -> Option<super::activity::Notice> {
        use super::activity::Notice;
        if self.error.is_some() {
            Some(Notice::Problem(
                "The current task hit a problem. Check the suggested fix.",
            ))
        } else if self.busy() {
            Some(Notice::Progress(self.status))
        } else {
            None
        }
    }

    pub(super) fn activity_notice(&self) -> super::activity::Notice {
        use super::activity::Notice;
        self.activity_signal().unwrap_or_else(|| {
            Notice::Progress(match self.library_tab {
                LibraryTab::Profiles => "Choose a saved profile to prepare an installer.",
                LibraryTab::Queue => "Review the installers waiting to build.",
                LibraryTab::Results if self.build.is_some() => "The installer is ready.",
                LibraryTab::Results => "Review the finished builds.",
            })
        })
    }

    pub(super) fn profile_saved(&self) -> bool {
        self.saved.is_some()
    }

    pub(super) fn saved_approval_ready(&self) -> bool {
        self.saved.is_some() && self.approval_error.is_none()
    }

    pub(super) fn busy(&self) -> bool {
        self.job.is_some()
    }

    pub(super) fn review_fix_supported(
        &self,
        finding: &portableforge_core::classify::Finding,
    ) -> bool {
        #[cfg(all(windows, target_arch = "x86_64"))]
        {
            portableforge_core::wizard_package::supports_review_fix(
                finding,
                Some(Path::new(self.installed_path.trim())),
            )
        }
        #[cfg(not(all(windows, target_arch = "x86_64")))]
        {
            let _ = finding;
            false
        }
    }

    pub(super) fn exclusion_reason(
        &self,
        finding: &portableforge_core::classify::Finding,
    ) -> &'static str {
        use portableforge_core::classify::Finding;
        match finding {
            Finding::AssociationCandidate { .. } => {
                "Excluded because it would change which app opens files on this computer."
            }
            Finding::RegistryKey { .. } => {
                "Excluded because the setting is outside the supported app-specific settings area."
            }
            Finding::DirectoriesMove { .. } | Finding::FilesMove { .. } => {
                "Excluded because this location cannot be moved safely into the portable copy."
            }
            Finding::RedFlag { .. } => {
                "This system change needs review. The portable copy does not apply a fix for it."
            }
            _ => "This change has no supported portable fix.",
        }
    }

    fn start(
        &mut self,
        ctx: &egui::Context,
        status: &'static str,
        task: impl FnOnce(&dyn Fn(&'static str)) -> Result<JobResult, String> + Send + 'static,
    ) -> bool {
        self.start_with_events(ctx, status, move |tx, ctx| {
            task(&|status| {
                let _ = tx.send(JobEvent::Stage(status));
                ctx.request_repaint();
            })
        })
    }

    fn start_with_events(
        &mut self,
        ctx: &egui::Context,
        status: &'static str,
        task: impl FnOnce(&mpsc::Sender<JobEvent>, &egui::Context) -> Result<JobResult, String>
            + Send
            + 'static,
    ) -> bool {
        if self.busy() || ctx.input(|input| input.viewport().close_requested()) {
            return false;
        }
        self.status = status;
        self.error = None;
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        match std::thread::Builder::new()
            .name("portableforge-wizard-job".into())
            .spawn(move || {
                let result = task(&tx, &ctx);
                let _ = tx.send(JobEvent::Done(Box::new(result)));
                ctx.request_repaint();
            }) {
            Ok(_) => {
                self.job = Some(rx);
                true
            }
            Err(error) => {
                self.error = Some(error.to_string());
                false
            }
        }
    }

    fn poll(&mut self) -> Option<Result<JobResult, String>> {
        loop {
            match self.job.as_ref()?.try_recv() {
                Ok(JobEvent::Stage(status)) => self.status = status,
                #[cfg(all(windows, target_arch = "x86_64"))]
                Ok(JobEvent::Choose(choice)) => {
                    if !self
                        .cancellation
                        .as_ref()
                        .is_some_and(CancellationToken::is_cancelled)
                    {
                        self.candidate = Some(choice);
                    }
                }
                Ok(JobEvent::Done(result)) => {
                    self.job = None;
                    return Some(*result);
                }
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => {
                    self.job = None;
                    return Some(Err("The work stopped before reporting a result.".into()));
                }
            }
        }
    }

    pub(super) fn reset_trust(&mut self) {
        self.trusted = false;
        if !self.captured {
            self.cleaned_capture = None;
        }
    }

    pub(super) fn detach_capture(&mut self) {
        self.captured = false;
        self.reset_trust();
        self.saved = None;
        self.update_details = None;
        self.installed_path.clear();
        self.capture_setup.clear();
        self.setup_args.clear();
        self.companions.clear();
    }

    pub(super) fn can_cancel_owned_job(&self) -> bool {
        #[cfg(all(windows, target_arch = "x86_64"))]
        {
            self.cancellation.is_some()
        }
        #[cfg(not(all(windows, target_arch = "x86_64")))]
        {
            false
        }
    }

    pub(super) fn cancel_capture(&mut self) {
        #[cfg(all(windows, target_arch = "x86_64"))]
        if let Some(cancellation) = &self.cancellation {
            match cancellation.cancel() {
                Ok(()) => {
                    self.candidate = None;
                    self.status = "Stopping and removing the dedicated box…";
                }
                Err(error) => self.error = Some(error),
            }
        }
    }

    #[cfg(all(windows, target_arch = "x86_64"))]
    fn show_candidate(&mut self, ui: &mut egui::Ui) {
        let Some(choice) = &mut self.candidate else {
            return;
        };
        ui.label("Choose the app that should open when you use the portable copy.");
        let mut picked = false;
        for i in dense::page(ui, &mut choice.page, choice.paths.len(), 1, "App") {
            let path = choice.paths[i].display().to_string();
            dense::view_text(ui, "View app path", "Installed app path", &path, true);
            if ui
                .button(format!(
                    "Use {}",
                    choice.paths[i]
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                ))
                .clicked()
            {
                picked = true;
                if choice.reply.send(choice.paths[i].clone()).is_err() {
                    self.error = Some("The capture stopped before receiving your choice.".into());
                }
            }
        }
        if picked {
            self.candidate = None;
        }
    }

    fn show_feedback(&self, ui: &mut egui::Ui) {
        if self.busy() {
            ui.horizontal_wrapped(|ui| {
                ui.spinner();
                ui.colored_label(theme::info(ui), self.status);
            });
        }
        if let Some(error) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, "× Operation failed");
            dense::view_text(ui, "View error details", "Problem details", error, true);
        }
    }

    fn show_setup_arguments(&mut self, ui: &mut egui::Ui) {
        if self.setup_args.is_empty() {
            self.setup_args.push(String::new());
        }
        let index = dense::page(
            ui,
            &mut self.setup_arg_page,
            self.setup_args.len(),
            1,
            "Argument",
        )
        .start;
        let label = ui.label("Setup argument");
        let id = ui.make_persistent_id(("setup-argument", index));
        // A pasted list replaces the current row with one argument per line.
        let mut pasted = None;
        if ui.memory(|memory| memory.has_focus(id)) {
            ui.input_mut(|input| {
                input.events.retain(|event| {
                    if let egui::Event::Paste(text) = event {
                        if text.contains('\n') {
                            pasted = Some(text.clone());
                            return false;
                        }
                    }
                    true
                });
            });
        }
        let response = ui
            .add(
                egui::TextEdit::singleline(&mut self.setup_args[index])
                    .id(id)
                    .desired_width(f32::INFINITY),
            )
            .labelled_by(label.id);
        dense::field_help(
            &response,
            "Paste one setup argument per line to add several arguments at once.",
        );
        let changed = response.changed();
        if let Some(pasted) = pasted {
            self.setup_args
                .splice(index..=index, pasted.lines().map(str::to_owned));
            self.reset_trust();
        } else if changed {
            self.reset_trust();
        }
        let mut added = false;
        ui.horizontal_wrapped(|ui| {
            if ui.button("Add argument").clicked() {
                self.setup_arg_page = self.setup_args.len();
                self.setup_args.push(String::new());
                added = true;
            }
            if ui
                .add_enabled(
                    !self.setup_args[index].is_empty() || self.setup_args.len() > 1,
                    egui::Button::new("Remove argument"),
                )
                .clicked()
            {
                self.setup_args.remove(index);
                self.reset_trust();
            }
            let text = self.setup_args.join("\n");
            dense::view_text(
                ui,
                "View arguments",
                "Setup arguments",
                &text,
                !text.is_empty(),
            );
            if ui
                .add_enabled(!text.is_empty(), egui::Button::new("Clear arguments"))
                .clicked()
            {
                self.setup_args.clear();
                self.setup_arg_page = 0;
                self.reset_trust();
            }
        });
        if added {
            let id = ui.make_persistent_id(("setup-argument", self.setup_arg_page));
            ui.memory_mut(|memory| memory.request_focus(id));
        }
    }

    fn show_companions(&mut self, ui: &mut egui::Ui) {
        let count = self.companions.len();
        if count == 0 {
            ui.label("Companion files stay beside setup inside the box.");
        } else {
            ui.label(format!("Companion files · {count}"));
        }
        ui.horizontal_wrapped(|ui| {
            if let Some(paths) = file_picker::choose(
                &ui.button("Choose companion files…"),
                "Choose files used by setup",
                file_picker::Mode::Files(&[]),
                None,
            ) {
                self.companions = paths;
                self.reset_trust();
            }
            if !self.companions.is_empty() {
                let paths = self
                    .companions
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join("\n");
                dense::view_text(ui, "View companion files", "Companion files", &paths, true);
                if ui.button("Clear companion files").clicked() {
                    self.companions.clear();
                    self.reset_trust();
                }
            }
        });
    }

    fn show_prerequisites(&self, ui: &mut egui::Ui) {
        let Some(checks) = &self.prerequisites else {
            return;
        };
        let (status, tone) = if checks.capture.is_ok() {
            ("Ready", RecordStatus::Complete)
        } else {
            ("Needs attention", RecordStatus::Failed)
        };
        let mut details = Vec::new();
        if let Err(error) = &checks.capture {
            details.push(format!("Sandboxie-Plus 1.18.8 · {status}\n{error}"));
        }
        for (tool, result) in [
            ("Launcher Generator", checks.launcher.as_ref()),
            ("Installer maker", checks.installer.as_ref()),
        ] {
            details.push(match result {
                Some(Ok(())) => format!("{tool} · Ready"),
                Some(Err(error)) => format!("{tool} · Needs attention\n{error}"),
                None => format!("{tool} · Choose folder"),
            });
        }
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(tone.color(ui), format!("Sandboxie-Plus 1.18.8 · {status}"));
            dense::view_text(
                ui,
                "View prerequisite details",
                "Prerequisites",
                &details.join("\n\n"),
                true,
            );
        });
    }
}

fn path_field(ui: &mut egui::Ui, label: &str, value: &mut String, kind: FieldKind<'_>) -> bool {
    let heading = ui.label(label);
    let browse = match kind {
        FieldKind::Typed => None,
        FieldKind::NewFolder(..) => Some("Choose parent…"),
        _ => Some("Browse…"),
    };
    let mut changed = false;
    ui.horizontal(|ui| {
        let width = browse.map_or(ui.available_width() - 12.0, |name| {
            dense::width_with_button(ui, name)
        });
        let (response, view) = dense::path_edit(ui, value, width);
        changed = response.labelled_by(heading.id).changed();
        if let Some(browse) = browse {
            let browse_button = ui.button(browse);
            dense::field_help(&browse_button, label);
            let mode = match kind {
                FieldKind::File => file_picker::Mode::File(&["exe"]),
                FieldKind::MatchingSetup => file_picker::Mode::File(&["exe", "msi"]),
                _ => file_picker::Mode::Folder,
            };
            let start = browse_button
                .clicked()
                .then(|| picker_start_dir(value))
                .flatten();
            if let Some(path) = file_picker::choose(&browse_button, label, mode, start)
                .and_then(|paths| paths.into_iter().next())
            {
                let picked = match kind {
                    FieldKind::NewFolder(name, queue) => fresh_child_path(&path, name, queue),
                    _ => Ok(path),
                };
                match picked {
                    Ok(path) => {
                        *value = path.to_string_lossy().into_owned();
                        changed = true;
                    }
                    Err(error) => {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                    }
                }
            }
        }
        dense::view_text(ui, "View path", label, value, view);
    });
    changed
}

enum FieldKind<'a> {
    File,
    MatchingSetup,
    Folder,
    NewFolder(&'a str, &'a [QueuedBuild]),
    Typed,
}

fn entered(value: &str) -> PathBuf {
    PathBuf::from(value.trim())
}

fn output_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

fn destination_key(path: &Path) -> PathBuf {
    let parent = output_parent(path);
    std::fs::canonicalize(parent)
        .unwrap_or_else(|_| parent.to_path_buf())
        .join(path.file_name().unwrap_or_default())
}

fn queue_uses_destination(queue: &[QueuedBuild], destination: &Path) -> bool {
    queue.iter().any(|job| {
        let used = &job.request.destination;
        if cfg!(windows) {
            destination
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_uppercase()
                == used
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_uppercase()
                && destination_key(destination)
                    .to_string_lossy()
                    .to_uppercase()
                    == destination_key(used).to_string_lossy().to_uppercase()
        } else {
            destination.file_name() == used.file_name()
                && destination_key(destination) == destination_key(used)
        }
    })
}

fn fresh_child_path(parent: &Path, name: &str, queue: &[QueuedBuild]) -> Result<PathBuf, String> {
    for index in 1.. {
        let path = parent.join(if index == 1 {
            name.to_owned()
        } else {
            format!("{name} ({index})")
        });
        match path.symlink_metadata() {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if !queue_uses_destination(queue, &path) {
                    return Ok(path);
                }
            }
            Err(error) => return Err(format!("Could not check the new folder: {error}")),
        }
    }
    unreachable!()
}

impl WizardApp {
    pub(super) fn verified_metadata_error(&self) -> Option<&'static str> {
        if !self.use_verified_capture() {
            return None;
        }
        #[cfg(all(windows, target_arch = "x86_64"))]
        {
            if self.publisher.trim().is_empty() {
                return Some("Enter the app publisher.");
            }
            let homepage = self.homepage.trim();
            let is_web_url = homepage.split_once("://").is_some_and(|(scheme, _)| {
                scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
            });
            if !is_web_url || !portableforge_core::wizard_package::valid_homepage(homepage) {
                return Some("Enter the app homepage as a full HTTP or HTTPS address.");
            }
            let description = self.description.trim();
            if description.is_empty() {
                return Some("Enter a short app description.");
            }
            if description.chars().count() > 512 {
                return Some("Keep the description to 512 characters or fewer.");
            }
        }
        None
    }

    pub(super) fn use_verified_capture(&self) -> bool {
        self.profile_flow.selected_capture
            || (!self.uses_guided_msi()
                && (self.session_mode == Some(SandboxMode::Sandboxie)
                    || self
                        .preflight
                        .as_ref()
                        .is_some_and(|report| report.best_mode == SandboxMode::Sandboxie)))
    }

    pub(super) fn enter_verified_capture(&mut self) {
        self.profile_flow.selected_capture = true;
        self.step = 2;
        self.show_details = false;
    }

    pub(super) fn poll_profile_work(&mut self, ctx: &egui::Context) {
        let Some(result) = self.profile_flow.poll() else {
            return;
        };
        #[cfg(all(windows, target_arch = "x86_64"))]
        {
            self.profile_flow.cancellation = None;
            self.profile_flow.candidate = None;
        }
        let active_queue = self.profile_flow.active_queue.take();
        match result {
            #[cfg(all(windows, target_arch = "x86_64"))]
            Ok(JobResult::Captured(capture)) => {
                self.artwork = Default::default();
                let CaptureComplete {
                    layout,
                    outcome,
                    installed,
                    receipt,
                    reopened,
                } = *capture;
                self.layout = Some(layout);
                self.session_mode = Some(SandboxMode::Sandboxie);
                self.profile_flow.captured = true;
                self.profile_flow.cleaned_capture = Some(receipt);
                self.main_exe = installed
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into();
                self.profile_flow.installed_path = installed.to_string_lossy().into();
                self.hints_applied = false;
                self.apply_capture(outcome);
                for item in &mut self.review.items {
                    if !self
                        .profile_flow
                        .review_fix_supported(&item.classification.finding)
                    {
                        if let portableforge_core::review::Decision::Fix { on } = &mut item.decision
                        {
                            *on = false;
                        }
                    }
                }
                self.step = 6;
                self.show_details = false;
                self.review_needs_attention = !self.review.all_acknowledged();
                self.log.push(if reopened {
                    "Reopened the cleaned capture. Review the choices again before saving.".into()
                } else {
                    "Capture complete. The dedicated box was removed.".into()
                });
                self.mark_dirty();
                self.persist_now();
                if self.settings_dirty {
                    self.log.push("The capture is ready. This computer could not save the draft for reopening.".into());
                }
                self.start_update_comparison(ctx);
            }
            Ok(JobResult::Checked(prerequisites)) => {
                self.profile_flow.prerequisites = Some(prerequisites);
            }
            Ok(JobResult::MsiChecked(result)) => self.profile_flow.msi_ready = Some(result),
            #[cfg(windows)]
            Ok(JobResult::MsiStarted(layout)) => {
                self.layout = Some(layout);
                self.session_mode = Some(SandboxMode::WindowsSandbox);
                self.log.push(
                    "Windows Sandbox is starting. Wait for the clean note before opening setup."
                        .into(),
                );
            }
            #[cfg(all(windows, target_arch = "x86_64"))]
            Ok(JobResult::Saved(approval)) => {
                self.profile_flow.saved = Some(approval);
                self.settings.draft = None;
                self.settings_dirty = true;
                self.step = 11;
                self.show_details = false;
                self.save_approval();
            }
            #[cfg(all(windows, target_arch = "x86_64"))]
            Ok(JobResult::Rebuilt(result)) => {
                let (approval, build) = *result;
                if let Err(error) = settings::save_build(
                    &self.settings_path,
                    &mut self.settings,
                    &approval,
                    build.clone(),
                ) {
                    self.profile_flow.error = Some(format!(
                        "The installer is ready. The build could not be remembered: {error}"
                    ));
                }
                if let Some(index) = active_queue {
                    self.profile_flow.queue[index].state = QueueState::Built(build);
                } else {
                    self.profile_flow.build = Some(build);
                    self.profile_flow.library_tab = LibraryTab::Results;
                }
            }
            #[cfg(all(windows, target_arch = "x86_64"))]
            Ok(JobResult::Relocated(approval, destination)) => {
                if let Err(error) = settings::relocate_approval(
                    &self.settings_path,
                    &mut self.settings,
                    &approval,
                    destination.clone(),
                ) {
                    self.profile_flow.error = Some(format!("The profile was copied to {}. Its new location could not be remembered: {error}", destination.display()));
                }
            }
            #[cfg(all(windows, target_arch = "x86_64"))]
            Ok(JobResult::UpdateCompared(details)) => {
                self.profile_flow.update_details = Some(details)
            }
            #[cfg(all(windows, target_arch = "x86_64"))]
            Ok(JobResult::BuildDetails(title, details)) => {
                self.profile_flow.build_details_page = 0;
                self.profile_flow.build_details = Some((title, details))
            }
            Err(error) => {
                if let Some(index) = active_queue {
                    self.profile_flow.queue[index].state = QueueState::Failed(error);
                } else {
                    self.profile_flow.error = Some(error);
                }
            }
        }
        self.advance_queue(ctx);
    }

    fn save_approval(&mut self) {
        let Some(approval) = self.profile_flow.saved.clone() else {
            return;
        };
        match settings::save_approval(&self.settings_path, &mut self.settings, approval) {
            Ok(()) => {
                self.settings_dirty = false;
                self.last_save = std::time::Instant::now();
                self.profile_flow.approval_error = None;
                self.log
                    .push("Saved the profile and remembered its approval.".into());
            }
            Err(error) => {
                self.settings_dirty = false;
                self.profile_flow.approval_error = Some(error);
            }
        }
    }

    pub(super) fn show_verified_capture(&mut self, ui: &mut egui::Ui) {
        if self.profile_flow.captured {
            ui.label("Capture complete. The dedicated box was removed.");
            return;
        }
        let busy = self.profile_flow.busy();
        if busy {
            #[cfg(all(windows, target_arch = "x86_64"))]
            {
                self.profile_flow.show_candidate(ui);
                if self.profile_flow.cancellation.is_some() && ui.button("Cancel capture").clicked()
                {
                    self.profile_flow.cancel_capture();
                }
            }
            self.profile_flow.show_feedback(ui);
            return;
        }
        ui.scope(|ui| {
            if path_field(ui, "Setup file", &mut self.installer_path, FieldKind::File) {
                self.setup_changed();
            }
            if path_field(
                ui,
                "Installed app path (optional)",
                &mut self.profile_flow.installed_path,
                FieldKind::Typed,
            ) {
                self.profile_flow.reset_trust();
            }
            ui.horizontal_wrapped(|ui| {
                if ui.button("Setup options…").clicked() {
                    self.profile_flow.setup_options_open = true;
                }
                if ui.button("Check prerequisites").clicked() {
                    self.check_prerequisites(ui.ctx());
                }
                if let Some(blocker) = self.capture_blocker() {
                    ui.label(egui::RichText::new(blocker).weak());
                }
            });
            let trust_label = if self.profile_flow.companions.is_empty() {
                "I trust this setup file and the installed app."
            } else {
                "I trust setup, the chosen companion files and the installed app."
            };
            ui.checkbox(&mut self.profile_flow.trusted, trust_label);
        });
        self.profile_flow.show_prerequisites(ui);
        if self
            .profile_flow
            .prerequisites
            .as_ref()
            .is_some_and(|checks| checks.capture.is_err())
        {
            self.show_bundled_test_box_help(ui);
        }
        self.profile_flow.show_feedback(ui);
        if self.profile_flow.setup_options_open {
            self.show_setup_options(ui.ctx());
        }
    }

    pub(super) fn capture_ready(&self) -> bool {
        !self.profile_flow.busy() && self.capture_blocker().is_none()
    }

    fn capture_blocker(&self) -> Option<&'static str> {
        match setup_input(&self.installer_path) {
            SetupInput::Ready {
                kind: SetupKind::Exe,
                ..
            } => {}
            SetupInput::Ready {
                kind: SetupKind::Msi,
                ..
            } => return Some("Use Windows Sandbox for MSI setup."),
            SetupInput::Empty => return Some("Choose an EXE setup file."),
            input => return input.problem(),
        }
        match &self.profile_flow.prerequisites {
            None => Some("Check prerequisites before capture."),
            Some(checks) if checks.capture.is_err() => Some("Resolve the dedicated box check."),
            _ if !self.profile_flow.trusted => Some("Confirm that you trust setup and the app."),
            _ => None,
        }
    }

    pub(super) fn check_prerequisites(&mut self, ctx: &egui::Context) {
        let launcher = entered(&self.profile_flow.launcher_tools);
        let installer = entered(&self.profile_flow.installer_tools);
        self.profile_flow
            .start(ctx, "Checking prerequisites…", move |_| {
                Ok(JobResult::Checked(check_tools(&launcher, &installer)))
            });
    }

    fn show_setup_options(&mut self, ctx: &egui::Context) {
        let msi = self.uses_guided_msi();
        let response = egui::Modal::new(egui::Id::new("verified-setup-options")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 48.0).clamp(200.0, 460.0));
            ui.heading("Setup options");
            if !msi {
                ui.horizontal(|ui| {
                    ui.selectable_value(
                        &mut self.profile_flow.setup_folders_open,
                        false,
                        "Setup files",
                    );
                    ui.selectable_value(&mut self.profile_flow.setup_folders_open, true, "Folders");
                });
            }
            if !msi && self.profile_flow.setup_folders_open {
                self.show_setup_folders(ui);
            } else {
                if msi {
                    ui.label("Enter MSI options in the setup window inside Windows Sandbox.");
                } else {
                    self.profile_flow.show_setup_arguments(ui);
                }
                self.profile_flow.show_companions(ui);
            }
            ui.button("Done").clicked()
        });
        if response.inner || response.should_close() {
            self.profile_flow.setup_options_open = false;
        }
    }

    fn show_setup_folders(&mut self, ui: &mut egui::Ui) {
        path_field(
            ui,
            "Capture folder",
            &mut self.profile_flow.capture_parent,
            FieldKind::Folder,
        );
        let launcher = path_field(
            ui,
            "Launcher Generator folder",
            &mut self.profile_flow.launcher_tools,
            FieldKind::Folder,
        );
        let installer = path_field(
            ui,
            "Installer maker folder",
            &mut self.profile_flow.installer_tools,
            FieldKind::Folder,
        );
        if launcher || installer {
            self.profile_flow.prerequisites = None;
        }
    }

    pub(super) fn start_verified_capture(&mut self, ctx: &egui::Context) {
        if !self.capture_ready() {
            return;
        }
        #[cfg(all(windows, target_arch = "x86_64"))]
        {
            let cancellation = match CancellationToken::new() {
                Ok(token) => token,
                Err(error) => {
                    self.profile_flow.error = Some(error);
                    return;
                }
            };
            let input = CaptureInputs {
                setup: entered(&self.installer_path),
                installed: (!self.profile_flow.installed_path.trim().is_empty())
                    .then(|| entered(&self.profile_flow.installed_path)),
                parent: (!self.profile_flow.capture_parent.trim().is_empty())
                    .then(|| entered(&self.profile_flow.capture_parent)),
                args: self
                    .profile_flow
                    .setup_args
                    .iter()
                    .filter(|line| !line.is_empty())
                    .map(std::ffi::OsString::from)
                    .collect(),
                companions: self.profile_flow.companions.clone(),
            };
            let worker_token = cancellation.clone();
            if self.profile_flow.start_with_events(
                ctx,
                "Preparing the dedicated box…",
                move |tx, ctx| capture_job(input, tx, ctx, &worker_token),
            ) {
                self.profile_flow.cancellation = Some(cancellation);
                self.profile_flow.capture_setup = self.installer_path.trim().to_owned();
                self.profile_flow.cleaned_capture = None;
                self.mark_dirty();
            }
        }
        #[cfg(not(all(windows, target_arch = "x86_64")))]
        let _ = ctx;
    }

    pub(super) fn show_msi_safety(&mut self, ui: &mut egui::Ui) {
        match self.profile_flow.msi_ready.as_ref() {
            Some(Ok(())) => {
                ui.label("Windows Sandbox · Available to start");
            }
            Some(Err(error)) => {
                ui.label("Windows Sandbox · Needs attention");
                dense::view_text(
                    ui,
                    "View prerequisite details",
                    "Windows Sandbox",
                    error,
                    true,
                );
            }
            None => {
                ui.label("Check whether this computer can open Windows Sandbox.");
            }
        }
        if ui
            .add_enabled(
                !self.profile_flow.busy(),
                egui::Button::new("Check Windows Sandbox"),
            )
            .clicked()
        {
            self.profile_flow
                .start(ui.ctx(), "Checking Windows Sandbox…", move |_| {
                    #[cfg(windows)]
                    let result = crate::guided_msi_available();
                    #[cfg(not(windows))]
                    let result = Err("Guided MSI capture requires Windows Sandbox.".into());
                    Ok(JobResult::MsiChecked(result))
                });
        }
        self.profile_flow.show_feedback(ui);
    }

    pub(super) fn show_msi_capture(&mut self, ui: &mut egui::Ui) {
        let busy = self.profile_flow.busy();
        if self.layout.is_some() {
            ui.label("Wait for the clean note before opening the MSI in Windows Sandbox.");
            if ui.button("Check on the test box").clicked() {
                self.check_test_box();
            }
            return;
        }
        ui.add_enabled_ui(!busy, |ui| {
            if ui.button("Setup options…").clicked() {
                self.profile_flow.setup_options_open = true;
            }
            ui.checkbox(
                &mut self.profile_flow.trusted,
                "I trust this MSI and the selected companion files.",
            );
        });
        self.profile_flow.show_feedback(ui);
        if self.profile_flow.setup_options_open && !busy {
            self.show_setup_options(ui.ctx());
        }
    }

    pub(super) fn msi_capture_ready(&self) -> bool {
        !self.profile_flow.busy()
            && self.profile_flow.trusted
            && self
                .profile_flow
                .msi_ready
                .as_ref()
                .is_some_and(Result::is_ok)
    }

    pub(super) fn start_msi_capture(&mut self, ctx: &egui::Context) {
        if !self.msi_capture_ready() || self.layout.is_some() {
            return;
        }
        #[cfg(windows)]
        {
            let setup = entered(&self.installer_path);
            let companions = self.profile_flow.companions.clone();
            let memory = self.sandbox_memory_mb;
            self.profile_flow
                .start(ctx, "Preparing Windows Sandbox…", move |_| {
                    crate::guided_msi_available()?;
                    let agent = std::env::current_exe()
                        .map_err(|error| error.to_string())?
                        .with_file_name("portableforge-agent.exe");
                    let stamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_err(|error| error.to_string())?
                        .as_nanos();
                    let id = format!("Guided-MSI-{}-{stamp}", std::process::id());
                    let layout = crate::prepare_guided_msi_capture(
                        &super::sessions_base(),
                        &id,
                        &setup,
                        &companions,
                        &agent,
                        memory,
                    )?;
                    crate::launch_guided_msi_capture(&layout)?;
                    Ok(JobResult::MsiStarted(layout))
                });
        }
        #[cfg(not(windows))]
        let _ = ctx;
    }

    pub(super) fn restore_cleaned_capture(&mut self, ctx: &egui::Context, capture: CleanedCapture) {
        self.enter_verified_capture();
        self.profile_flow.capture_setup = self.installer_path.trim().to_owned();
        self.profile_flow.cleaned_capture = Some(capture.clone());
        self.profile_flow
            .start(ctx, "Checking the saved capture…", move |_| {
                #[cfg(all(windows, target_arch = "x86_64"))]
                {
                    load_cleaned_capture(&capture.folder, Some(&capture.receipt_sha256))
                }
                #[cfg(not(all(windows, target_arch = "x86_64")))]
                {
                    let _ = capture;
                    Err(
                        "Opening a verified capture needs 64-bit Windows on an x64 computer."
                            .into(),
                    )
                }
            });
    }

    pub(super) fn show_profile_save(&mut self, ui: &mut egui::Ui) {
        if self.profile_flow.saved.is_some() {
            ui.label("The reviewed profile is ready.");
            return;
        }
        let busy = self.profile_flow.busy();
        ui.add_enabled_ui(!busy, |ui| {
            path_field(
                ui,
                "PortableApps.com Launcher Generator folder (2.2.9)",
                &mut self.profile_flow.launcher_tools,
                FieldKind::Folder,
            );
            ui.label("Select the folder where the Launcher Generator is installed.");
            path_field(
                ui,
                "New profile folder",
                &mut self.profile_flow.profile_destination,
                FieldKind::NewFolder("PortableForge Profile", &[]),
            );
            ui.horizontal_wrapped(|ui| {
                let edit = ui.add_enabled(
                    self.profile_flow.captured && self.layout.is_some(),
                    egui::Button::new("Edit package artwork…"),
                );
                if edit.clicked() {
                    if let Some(layout) = &self.layout {
                        self.artwork.open(edit.id, None);
                        self.artwork.load_captured_icon(
                            ui.ctx(),
                            layout.root.clone(),
                            PathBuf::from(self.main_exe.trim()),
                        );
                    }
                }
                ui.weak(if self.artwork.artwork().is_some() {
                    "Custom artwork"
                } else {
                    "Default artwork"
                });
            });
        });
        self.profile_flow.show_feedback(ui);
    }

    pub(super) fn profile_save_ready(&self) -> bool {
        !self.profile_flow.busy()
            && !self.artwork.is_open()
            && self.profile_flow.captured
            && !self.profile_flow.launcher_tools.trim().is_empty()
            && !self.profile_flow.profile_destination.trim().is_empty()
            && (self.profile_flow.update_base.is_none()
                || self.profile_flow.update_details.is_some())
    }

    pub(super) fn start_profile_save(&mut self, ctx: &egui::Context) {
        if !self.profile_save_ready() {
            return;
        }
        let Some(layout) = &self.layout else {
            return;
        };
        let meta = match self.package_metadata() {
            Ok(meta) => meta,
            Err(error) => {
                self.profile_flow.error = Some(error);
                return;
            }
        };
        let session = layout.root.clone();
        let review = self.review.clone();
        let tools = entered(&self.profile_flow.launcher_tools);
        let destination = entered(&self.profile_flow.profile_destination);
        let artwork = self.artwork.artwork().cloned();
        self.profile_flow
            .start(ctx, "Preparing and saving your profile…", move |_| {
                save_job(
                    &session,
                    &meta,
                    &review,
                    &tools,
                    &destination,
                    artwork.as_ref(),
                )
            });
    }

    pub(super) fn show_saved_profile(&mut self, ui: &mut egui::Ui) {
        if let Some(approval) = &self.profile_flow.saved {
            let path = &approval.folder;
            let path_text = path.display().to_string();
            if dense::path_needs_view(ui, &path_text, ui.available_width()) {
                dense::view_text(ui, "View profile path", "Profile folder", &path_text, true);
            } else {
                ui.label(path_text);
            }
            dense::view_text(
                ui,
                "View profile folder",
                "Profile folder",
                &path.display().to_string(),
                true,
            );
        }
        if let Some(error) = self.profile_flow.approval_error.clone() {
            ui.colored_label(
                ui.visuals().error_fg_color,
                "The profile was saved. This computer could not remember its approval.",
            );
            if ui.button("Retry remembering the approval").clicked() {
                self.save_approval();
            }
            dense::view_text(ui, "View error details", "Approval problem", &error, true);
            if let Some(approval) = &self.profile_flow.saved {
                if ui.button("Copy the approval fingerprint").clicked() {
                    ui.ctx().copy_text(approval.manifest_sha256.clone());
                }
            }
        }
        self.profile_flow.show_feedback(ui);
    }

    pub(super) fn open_profile_replay(&mut self, from_capture: bool) {
        self.settings.saved_profiles = settings::load(&self.settings_path).saved_profiles;
        self.profile_flow.replay_open = true;
        self.profile_flow.selected_profile = self
            .profile_flow
            .saved
            .as_ref()
            .and_then(|saved| {
                self.settings.saved_profiles.iter().position(|profile| {
                    profile.folder == saved.folder
                        && profile.manifest_sha256 == saved.manifest_sha256
                })
            })
            .unwrap_or(0);
        self.profile_flow.profile_page = self.profile_flow.selected_profile;
        self.profile_flow.replay_setup = if from_capture {
            self.profile_flow.capture_setup.clone()
        } else {
            self.installer_path.clone()
        };
        self.profile_flow.replay_destination.clear();
        self.profile_flow.build = None;
        self.profile_flow.library_tab = LibraryTab::Profiles;
        self.profile_flow.profile_search.clear();
        self.profile_flow.result_page = 0;
        self.profile_flow.error = None;
    }

    pub(super) fn show_profile_replay(&mut self, ctx: &egui::Context) {
        let title = self.profile_flow.library_tab.label();
        self.show_header(
            ctx,
            if dense::room_for_sidebar(ctx) {
                ""
            } else {
                title
            },
            None,
        );
        let busy = self.profile_flow.busy();
        let readiness = self.replay_request();
        let request = readiness.as_ref().ok();
        dense::show_help_bar(ctx, "Point to a setting or move to it with Tab for help.");
        egui::TopBottomPanel::bottom("profile-replay-back").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let back = ui.add_enabled(!busy, egui::Button::new("Back"));
                if !busy {
                    shell::register_region(ui, shell::Region::Footer, back.id);
                }
                if back.clicked() {
                    self.profile_flow.replay_open = false;
                }
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| match self.profile_flow.library_tab {
                        LibraryTab::Profiles => {
                            if dense::primary_action(
                                ui,
                                !busy && request.is_some(),
                                "Build installer",
                            )
                            .clicked()
                            {
                                if let Some(request) = request {
                                    self.start_rebuild(ctx, request.clone(), None);
                                }
                            }
                            if ui
                                .add_enabled(
                                    !busy && request.is_some(),
                                    egui::Button::new("Add to queue"),
                                )
                                .clicked()
                            {
                                if let Some(request) = request {
                                    self.enqueue(request.clone());
                                }
                            }
                            if !busy {
                                if let Err(blocker) = readiness {
                                    let reason = ui.add(
                                        egui::Label::new(egui::RichText::new(blocker).weak())
                                            .truncate(),
                                    );
                                    dense::field_help(&reason, blocker);
                                }
                            }
                        }
                        LibraryTab::Queue => {
                            let waiting = self
                                .profile_flow
                                .queue
                                .iter()
                                .any(|job| matches!(job.state, QueueState::Waiting));
                            if self.profile_flow.queue_running {
                                if ui.button("Pause after this build").clicked() {
                                    self.profile_flow.queue_running = false;
                                }
                            } else if dense::primary_action(
                                ui,
                                !busy && waiting,
                                "Build queued apps",
                            )
                            .clicked()
                            {
                                self.profile_flow.queue_running = true;
                                self.advance_queue(ctx);
                            }
                        }
                        LibraryTab::Results => {
                            if ui
                                .add_enabled(!busy, egui::Button::new("Build another"))
                                .clicked()
                            {
                                self.profile_flow.library_tab = LibraryTab::Profiles;
                            }
                        }
                    },
                );
            });
        });
        let tabs = [
            (
                LibraryTab::Profiles,
                "Choose a reviewed app. Its saved profile contains the app files and the packaging choices you approved.",
            ),
            (
                LibraryTab::Queue,
                "Review installers waiting to build. You can pause the queue after the current build or retry a failed job.",
            ),
            (
                LibraryTab::Results,
                "Find finished installers, open their build details, or start a test in the isolated test box.",
            ),
        ];
        dense::task_sidebar(ctx, |ui| {
            let mut registered = false;
            for (tab, help) in tabs {
                let response = dense::roadmap_item(
                    ui,
                    tab.label(),
                    self.profile_flow.library_tab == tab,
                    !busy,
                    help,
                );
                if !registered && response.enabled() {
                    shell::register_region(ui, shell::Region::Sidebar, response.id);
                    registered = true;
                }
                if response.clicked() {
                    self.profile_flow.library_tab = tab;
                }
            }
        });
        self.show_activity_side(ctx);
        egui::CentralPanel::default()
            .frame(dense::content_frame(ctx))
            .show(ctx, |ui| {
                theme::paint_work_area(ui);
                dense::compact_body(ui);
                if !dense::room_for_sidebar(ctx) {
                    ui.add_enabled_ui(!busy, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            for (tab, help) in tabs {
                                let response = ui.selectable_value(
                                    &mut self.profile_flow.library_tab,
                                    tab,
                                    tab.label(),
                                );
                                theme::focus_outline(ui, &response);
                                dense::field_help(&response, help);
                            }
                        });
                    });
                }
                if dense::room_for_sidebar(ctx) {
                    dense::heading(ui, self.profile_flow.library_tab.label(), "");
                }
                self.profile_flow.show_feedback(ui);
                match self.profile_flow.library_tab {
                    LibraryTab::Profiles => self.show_library_profiles(ui, busy),
                    LibraryTab::Queue => self.show_queue(ui),
                    LibraryTab::Results => self.show_build_results(ui),
                }
            });
        if self.profile_flow.profile_actions_open && !busy {
            self.show_profile_actions(ctx);
        }
        self.show_build_dialogs(ctx);
    }

    fn begin_profile_update(&mut self, ctx: &egui::Context, approval: SavedProfileApproval) {
        self.artwork = Default::default();
        self.profile_flow.update_base = Some(approval);
        self.profile_flow.update_details = None;
        self.profile_flow.profile_actions_open = false;
        self.profile_flow.replay_open = false;
        self.profile_flow.captured = false;
        self.profile_flow.saved = None;
        self.profile_flow.approval_error = None;
        self.profile_flow.cleaned_capture = None;
        self.profile_flow.reset_trust();
        self.profile_flow.installed_path.clear();
        self.profile_flow.capture_setup.clear();
        self.profile_flow.setup_args.clear();
        self.profile_flow.companions.clear();
        self.profile_flow.profile_destination.clear();
        self.installer_path.clear();
        self.app_name.clear();
        self.publisher.clear();
        self.homepage.clear();
        self.description.clear();
        self.main_exe.clear();
        self.package_version = "1.0.0.0".into();
        self.review = Default::default();
        self.review_page = 0;
        self.review_needs_attention = false;
        self.sharing = None;
        self.layout = None;
        self.session_mode = None;
        self.resume_offer = false;
        self.enter_verified_capture();
        self.mark_dirty();
        self.check_prerequisites(ctx);
    }

    fn start_update_comparison(&mut self, ctx: &egui::Context) {
        let (Some(approval), Some(layout)) = (self.profile_flow.update_base.clone(), &self.layout)
        else {
            return;
        };
        let session = layout.root.clone();
        let review = self.review.clone();
        self.profile_flow.start(
            ctx,
            "Comparing the update with the saved profile…",
            move |_| compare_update_job(&approval, &session, &review),
        );
    }

    pub(super) fn show_update_review(&mut self, ui: &mut egui::Ui) {
        let Some(approval) = &self.profile_flow.update_base else {
            return;
        };
        ui.label(format!("Update for {}", approval.label));
        if let Some(details) = &self.profile_flow.update_details {
            dense::view_text(
                ui,
                "View update comparison",
                "Saved profile and new capture",
                details,
                true,
            );
        } else if !self.profile_flow.busy() && ui.button("Compare with saved profile").clicked() {
            self.start_update_comparison(ui.ctx());
        }
        self.profile_flow.show_feedback(ui);
    }

    fn show_build_dialogs(&mut self, ctx: &egui::Context) {
        if let Some((title, details)) = &self.profile_flow.build_details {
            let response = egui::Modal::new(egui::Id::new("build-details")).show(ctx, |ui| {
                let viewport = ctx.content_rect();
                let width = (viewport.width() - 48.0).clamp(200.0, 500.0);
                let height = (viewport.height() * 0.45).clamp(90.0, 280.0);
                ui.set_width(width);
                ui.heading(*title);
                ui.allocate_ui_with_layout(
                    egui::vec2(width, height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| dense::diagnostic(ui, details, &mut self.profile_flow.build_details_page),
                );
                ui.horizontal(|ui| {
                    if ui.button("Copy full text").clicked() {
                        ctx.copy_text(details.clone());
                    }
                    ui.button("Done").clicked()
                })
                .inner
            });
            if response.inner || response.should_close() {
                self.profile_flow.build_details = None;
            }
        }
        if let Some(build) = self.profile_flow.runtime_confirm.clone() {
            let response = egui::Modal::new(egui::Id::new("confirm-installer-test")).show(ctx, |ui| {
                ui.set_max_width(440.0);
                ui.heading("Test the built installer");
                ui.label("Complete installation in the fresh Sandboxie-Plus box. Leave any option to open the app unchecked. Change a setting during the first launch and check it after reopening, then close the app.");
                ui.horizontal(|ui| (ui.button("Cancel").clicked(), dense::primary_action(ui, true, "Start test").clicked())).inner
            });
            if response.inner.0 || response.should_close() {
                self.profile_flow.runtime_confirm = None;
            } else if response.inner.1 {
                self.profile_flow.runtime_confirm = None;
                self.start_runtime_test(ctx, build);
            }
        }
    }

    fn start_runtime_test(&mut self, ctx: &egui::Context, build: SavedBuild) {
        #[cfg(all(windows, target_arch = "x86_64"))]
        {
            match CancellationToken::new() {
                Ok(cancellation) => {
                    let worker_token = cancellation.clone();
                    if self.profile_flow.start(
                        ctx,
                        "Complete setup, then close each app launch when ready.",
                        move |_| runtime_job(&build, &worker_token),
                    ) {
                        self.profile_flow.cancellation = Some(cancellation);
                    }
                }
                Err(error) => self.profile_flow.error = Some(error),
            }
        }
        #[cfg(not(all(windows, target_arch = "x86_64")))]
        {
            let _ = (ctx, build);
            self.profile_flow.error = Some(
                "The dedicated installer test needs 64-bit Windows on an x64 computer.".into(),
            );
        }
    }

    fn show_library_profiles(&mut self, ui: &mut egui::Ui, busy: bool) {
        if self.settings.saved_profiles.is_empty() {
            ui.label("Capture, review and save a profile before rebuilding.");
            return;
        }
        ui.add_enabled_ui(!busy, |ui| {
            let width = ui.available_width();
            if width >= 880.0 {
                let side = (width * 0.30).min(360.0);
                let main = width - side - ui.spacing().item_spacing.x;
                let height = ui.available_height().max(0.0);
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(main, height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(main);
                            self.show_profile_records(ui, 0.0);
                        },
                    );
                    ui.allocate_ui_with_layout(
                        egui::vec2(side, height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(side);
                            egui::Frame::new()
                                .fill(ui.visuals().faint_bg_color)
                                .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                                .inner_margin(egui::Margin::same(8))
                                .show(ui, |ui| self.show_replay_form(ui));
                        },
                    );
                });
            } else {
                let reserve = 3.0
                    * (ui.text_style_height(&egui::TextStyle::Body)
                        + ui.spacing().interact_size.y
                        + 2.0 * ui.spacing().item_spacing.y)
                    + 36.0;
                self.show_profile_records(ui, reserve);
                self.show_replay_form(ui);
            }
        });
    }

    fn show_profile_records(&mut self, ui: &mut egui::Ui, reserve: f32) {
        ui.horizontal(|ui| {
            let label = ui.label("Search saved profiles");
            let actions = "Profile actions…";
            let search_width = dense::width_with_button(ui, actions) - 8.0;
            let search = ui
                .add(
                    egui::TextEdit::singleline(&mut self.profile_flow.profile_search)
                        .desired_width(search_width),
                )
                .labelled_by(label.id);
            if ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::F)) {
                search.request_focus();
            }
            if search.changed() {
                self.profile_flow.profile_page = 0;
            }
            if ui
                .add_enabled(
                    self.profile_flow.selected_profile < self.settings.saved_profiles.len(),
                    egui::Button::new(actions),
                )
                .clicked()
            {
                self.profile_flow.profile_actions_open = true;
            }
        });
        let query = self.profile_flow.profile_search.trim().to_lowercase();
        let matches = self
            .settings
            .saved_profiles
            .iter()
            .enumerate()
            .filter_map(|(index, profile)| profile_matches(profile, &query).then_some(index))
            .collect::<Vec<_>>();
        if matches.is_empty() {
            self.profile_flow.selected_profile = usize::MAX;
            ui.label("No saved profiles match your search.");
            return;
        }
        let range = record_page(
            ui,
            self.profile_flow.profile_page,
            matches.len(),
            reserve,
            "Profiles",
        );
        let selected = matches
            .iter()
            .position(|index| *index == self.profile_flow.selected_profile)
            .unwrap_or(range.start);
        let index = record_table(
            ui,
            "saved-profiles",
            ["Profile", "Version", "Last build"],
            range,
            matches.len(),
            selected,
            |index| {
                let approval = &self.settings.saved_profiles[matches[index]];
                let last = approval
                    .details
                    .builds
                    .first()
                    .map_or("No builds", |build| build.completed_utc.as_str());
                (
                    [
                        approval.label.clone(),
                        approval.details.version.clone(),
                        last.into(),
                    ],
                    RecordStatus::Neutral,
                )
            },
        );
        if self.profile_flow.selected_profile != matches[index] {
            self.profile_flow.build = None;
            self.profile_flow.result_page = 0;
        }
        self.profile_flow.selected_profile = matches[index];
        self.profile_flow.profile_page = index;
    }

    fn show_replay_form(&mut self, ui: &mut egui::Ui) {
        if self.profile_flow.selected_profile == usize::MAX {
            return;
        }
        path_field(
            ui,
            "Matching setup file",
            &mut self.profile_flow.replay_setup,
            FieldKind::MatchingSetup,
        );
        if path_field(
            ui,
            "PortableApps.com Installer folder (3.9.19)",
            &mut self.profile_flow.installer_tools,
            FieldKind::Folder,
        ) {
            self.profile_flow.prerequisites = None;
        }
        path_field(
            ui,
            "New build folder",
            &mut self.profile_flow.replay_destination,
            FieldKind::NewFolder("PortableForge Installer", &self.profile_flow.queue),
        );
    }

    fn show_profile_actions(&mut self, ctx: &egui::Context) {
        let Some(approval) = self
            .settings
            .saved_profiles
            .get(self.profile_flow.selected_profile)
            .cloned()
        else {
            self.profile_flow.profile_actions_open = false;
            return;
        };
        let response = egui::Modal::new(egui::Id::new("saved-profile-actions")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 48.0).clamp(200.0, 460.0));
            ui.add(egui::Label::new(egui::RichText::new(&approval.label).heading()).wrap());
            ui.horizontal_wrapped(|ui| {
                let path = approval.folder.display().to_string();
                dense::view_text(ui, "View profile path", "Profile folder", &path, true);
                if !approval.details.setup_sha256.is_empty() {
                    dense::view_text(
                        ui,
                        "View setup fingerprint",
                        "Setup fingerprint",
                        &approval.details.setup_sha256,
                        true,
                    );
                }
            });
            ui.horizontal_wrapped(|ui| {
                if ui.button("Verify profile details").clicked() {
                    let selected = approval.clone();
                    self.profile_flow
                        .start(ctx, "Checking the saved profile…", move |_| {
                            inspect_job(&selected)
                        });
                    self.profile_flow.profile_actions_open = false;
                }
                if ui.button("Capture an update").clicked() {
                    self.begin_profile_update(ctx, approval.clone());
                }
            });
            path_field(
                ui,
                "New profile location",
                &mut self.profile_flow.relocate_destination,
                FieldKind::NewFolder("PortableForge Profile", &[]),
            );
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        !self.profile_flow.relocate_destination.trim().is_empty(),
                        egui::Button::new("Relocate verified profile"),
                    )
                    .clicked()
                {
                    let selected = approval.clone();
                    let destination = entered(&self.profile_flow.relocate_destination);
                    self.profile_flow.start(
                        ctx,
                        "Verifying and relocating your profile…",
                        move |_| relocate_job(selected, &destination),
                    );
                    self.profile_flow.profile_actions_open = false;
                }
                ui.button("Done").clicked()
            })
            .inner
        });
        if response.inner || response.should_close() {
            self.profile_flow.profile_actions_open = false;
        }
    }

    fn replay_request(&self) -> Result<ReplayRequest, &'static str> {
        let approval = self
            .settings
            .saved_profiles
            .get(self.profile_flow.selected_profile)
            .ok_or("Choose a saved profile.")?;
        match setup_input(&self.profile_flow.replay_setup) {
            SetupInput::Ready { .. } => {}
            SetupInput::Empty => return Err("Choose the matching setup file."),
            input => return Err(input.problem().unwrap_or("Check the setup file.")),
        }
        let setup = entered(&self.profile_flow.replay_setup);
        let tools = entered(&self.profile_flow.installer_tools);
        if tools.as_os_str().is_empty() {
            return Err("Choose the Installer folder.");
        }
        if !tools.is_dir() {
            return Err("Installer folder was not found.");
        }
        let destination = entered(&self.profile_flow.replay_destination);
        if destination.file_name().is_none() {
            return Err("Choose a new build folder.");
        }
        if !output_parent(&destination).is_dir() {
            return Err("Choose an existing parent folder.");
        }
        if !matches!(destination.symlink_metadata(), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
            || queue_uses_destination(&self.profile_flow.queue, &destination)
        {
            return Err("Choose an unused build folder.");
        }
        Ok(ReplayRequest {
            setup,
            approval: approval.clone(),
            tools,
            destination,
        })
    }

    fn edit_queued_build(&mut self, index: usize) {
        let job = &self.profile_flow.queue[index];
        let Some(profile) = self.settings.saved_profiles.iter().position(|approval| {
            approval.folder == job.request.approval.folder
                && approval.manifest_sha256 == job.request.approval.manifest_sha256
        }) else {
            self.profile_flow.error =
                Some("The saved profile for this attempt is unavailable.".into());
            return;
        };
        self.profile_flow.selected_profile = profile;
        self.profile_flow.profile_page = profile;
        self.profile_flow.profile_search.clear();
        self.profile_flow.replay_setup = job.request.setup.to_string_lossy().into_owned();
        let tools = job.request.tools.to_string_lossy();
        if self.profile_flow.installer_tools != tools {
            self.profile_flow.installer_tools = tools.into_owned();
            self.profile_flow.prerequisites = None;
        }
        self.profile_flow.replay_destination.clear();
        self.profile_flow.build = None;
        self.profile_flow.result_page = 0;
        self.profile_flow.library_tab = LibraryTab::Profiles;
        self.profile_flow.error = None;
        self.log.push(format!(
            "Editing build inputs after a failed attempt for {}.",
            job.request.approval.label
        ));
    }

    fn enqueue(&mut self, request: ReplayRequest) {
        if queue_uses_destination(&self.profile_flow.queue, &request.destination) {
            self.profile_flow.error =
                Some("Choose a new build folder for each queue entry.".into());
            return;
        }
        self.profile_flow.queue.push(QueuedBuild {
            request,
            state: QueueState::Waiting,
        });
        self.profile_flow.queue_page = self.profile_flow.queue.len() - 1;
        self.profile_flow.library_tab = LibraryTab::Queue;
        self.profile_flow.replay_destination.clear();
        self.profile_flow.error = None;
    }

    fn start_rebuild(
        &mut self,
        ctx: &egui::Context,
        request: ReplayRequest,
        queue_index: Option<usize>,
    ) {
        if self
            .profile_flow
            .start(ctx, "Building your installer…", move |_| {
                rebuild_job(request)
            })
        {
            self.profile_flow.active_queue = queue_index;
            if let Some(index) = queue_index {
                self.profile_flow.queue[index].state = QueueState::Building;
            }
        } else {
            self.profile_flow.queue_running = false;
        }
    }

    fn advance_queue(&mut self, ctx: &egui::Context) {
        if !self.profile_flow.queue_running || self.profile_flow.busy() {
            return;
        }
        if let Some(index) = self
            .profile_flow
            .queue
            .iter()
            .position(|item| matches!(item.state, QueueState::Waiting))
        {
            self.start_rebuild(
                ctx,
                self.profile_flow.queue[index].request.clone(),
                Some(index),
            );
        } else {
            self.profile_flow.queue_running = false;
        }
    }

    fn show_queue(&mut self, ui: &mut egui::Ui) {
        if self.profile_flow.queue.is_empty() {
            ui.label("Choose a saved profile and add it to the queue.");
            return;
        }
        let range = record_page(
            ui,
            self.profile_flow.queue_page,
            self.profile_flow.queue.len(),
            108.0,
            "Queued apps",
        );
        let index = record_table(
            ui,
            "queued-builds",
            ["Profile", "Build folder", "Status"],
            range,
            self.profile_flow.queue.len(),
            self.profile_flow.queue_page,
            |index| {
                let job = &self.profile_flow.queue[index];
                let (label, tone) = match job.state {
                    QueueState::Waiting => ("○ Waiting", RecordStatus::Neutral),
                    QueueState::Building => ("◌ Building", RecordStatus::Active),
                    #[cfg(all(windows, target_arch = "x86_64"))]
                    QueueState::Built(_) => ("✓ Installer ready", RecordStatus::Complete),
                    QueueState::Failed(_) => ("× Build failed", RecordStatus::Failed),
                };
                (
                    [
                        job.request.approval.label.clone(),
                        filename(&job.request.destination),
                        label.into(),
                    ],
                    tone,
                )
            },
        );
        self.profile_flow.queue_page = index;
        let busy = self.profile_flow.busy();
        let mut edit = false;
        let mut retry_request = None;
        let mut action_error = None;
        {
            let job = &self.profile_flow.queue[index];
            let destination = job.request.destination.display().to_string();
            dense::view_text(ui, "View build folder", "Build folder", &destination, true);
            match &job.state {
                QueueState::Waiting | QueueState::Building => {}
                #[cfg(all(windows, target_arch = "x86_64"))]
                QueueState::Built(build) => {
                    if ui.button("View this build").clicked() {
                        self.profile_flow.selected_profile = self
                            .settings
                            .saved_profiles
                            .iter()
                            .position(|profile| {
                                profile.folder == job.request.approval.folder
                                    && profile.manifest_sha256
                                        == job.request.approval.manifest_sha256
                            })
                            .unwrap_or(usize::MAX);
                        self.profile_flow.build = Some(build.clone());
                        self.profile_flow.library_tab = LibraryTab::Results;
                    }
                }
                QueueState::Failed(error) => {
                    ui.horizontal_wrapped(|ui| {
                        dense::view_text(ui, "View build log", "Build problem", error, true);
                        let work = job.request.destination.join("work");
                        dense::view_text(
                            ui,
                            "View build working folder",
                            "Build work folder",
                            &work.display().to_string(),
                            work.is_dir(),
                        );
                        edit = ui
                            .add_enabled(!busy, egui::Button::new("Edit build inputs"))
                            .clicked();
                        let retry =
                            ui.add_enabled(!busy, egui::Button::new("Retry in a new folder…"));
                        if let Some(parent) = file_picker::choose(
                            &retry,
                            "Choose a parent for the retry",
                            file_picker::Mode::Folder,
                            None,
                        )
                        .and_then(|paths| paths.into_iter().next())
                        {
                            match fresh_child_path(
                                &parent,
                                "PortableForge Retry",
                                &self.profile_flow.queue,
                            ) {
                                Ok(destination) => {
                                    let mut retry = job.request.clone();
                                    retry.destination = destination;
                                    retry_request = Some(retry);
                                }
                                Err(error) => action_error = Some(error),
                            }
                        }
                    });
                }
            }
        }
        if edit {
            self.edit_queued_build(index);
        }
        if let Some(request) = retry_request {
            self.enqueue(request);
        }
        if let Some(error) = action_error {
            self.profile_flow.error = Some(error);
        }
    }

    fn show_build_results(&mut self, ui: &mut egui::Ui) {
        let busy = self.profile_flow.busy();
        let mut history: Vec<_> = self
            .settings
            .saved_profiles
            .get(self.profile_flow.selected_profile)
            .map(|profile| profile.details.builds.iter().collect())
            .unwrap_or_default();
        let current = self.profile_flow.build.clone();
        let selected = current
            .as_ref()
            .map(|build| {
                if let Some(index) = history.iter().position(|saved| *saved == build) {
                    index
                } else {
                    history.insert(0, build);
                    0
                }
            })
            .unwrap_or(self.profile_flow.result_page);
        if !history.is_empty() {
            let range = record_page(ui, selected, history.len(), 125.0, "Saved builds");
            let index = record_table(
                ui,
                "saved-builds",
                ["Installer", "Built", "Status"],
                range,
                history.len(),
                selected,
                |index| {
                    let build = &history[index];
                    (
                        [
                            filename(&build.artifact),
                            build.completed_utc.clone(),
                            "✓ Build recorded".into(),
                        ],
                        RecordStatus::Complete,
                    )
                },
            );
            self.profile_flow.result_page = index;
            self.profile_flow.build = Some(history[index].clone());
        }
        let Some(build) = self.profile_flow.build.clone() else {
            ui.label("Build a saved profile to see its installer and verification details.");
            return;
        };
        let path = build.artifact.display().to_string();
        if dense::path_needs_view(ui, &path, ui.available_width()) {
            dense::view_text(ui, "View path", "Installer path", &path, true);
        } else {
            ui.label(path);
        }
        let previous = history
            .iter()
            .copied()
            .find(|other| other.directory != build.directory)
            .cloned();
        ui.add_enabled_ui(!busy, |ui| {
            ui.horizontal_wrapped(|ui| {
                dense::view_text(
                    ui,
                    "View installer folder",
                    "Installer folder",
                    &build.directory.display().to_string(),
                    true,
                );
                if ui.button("View build details").clicked() {
                    let selected = build.clone();
                    self.profile_flow.start(
                        ui.ctx(),
                        "Verifying the build receipt…",
                        move |_| build_details_job(&selected),
                    );
                }
                if ui
                    .add_enabled(previous.is_some(), egui::Button::new("Compare builds"))
                    .clicked()
                {
                    if let Some(previous) = previous {
                        let selected = build.clone();
                        self.profile_flow.start(
                            ui.ctx(),
                            "Checking both build receipts…",
                            move |_| compare_builds_job(&selected, &previous),
                        );
                    }
                }
                if dense::primary_action(ui, true, "Test installer").clicked() {
                    self.profile_flow.runtime_confirm = Some(build.clone());
                }
            });
        });
        if self.profile_flow.can_cancel_owned_job() && ui.button("Cancel test").clicked() {
            self.profile_flow.cancel_capture();
        }
    }
}

fn filename(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

fn check_tools(launcher: &Path, installer: &Path) -> Prerequisites {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        use portableforge_core::wizard_package::{
            verify_installer_toolchain, verify_launcher_toolchain,
        };
        Prerequisites {
            capture: portableforge_core::sbie_verified::preflight(),
            launcher: (!launcher.as_os_str().is_empty())
                .then(|| verify_launcher_toolchain(launcher)),
            installer: (!installer.as_os_str().is_empty())
                .then(|| verify_installer_toolchain(installer)),
        }
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        let _ = (launcher, installer);
        Prerequisites {
            capture: Err("Verified quick capture needs 64-bit Windows on an x64 computer.".into()),
            launcher: None,
            installer: None,
        }
    }
}

#[cfg(all(windows, target_arch = "x86_64"))]
struct CaptureInputs {
    setup: PathBuf,
    installed: Option<PathBuf>,
    parent: Option<PathBuf>,
    args: Vec<std::ffi::OsString>,
    companions: Vec<PathBuf>,
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn capture_job(
    inputs: CaptureInputs,
    tx: &mpsc::Sender<JobEvent>,
    ctx: &egui::Context,
    cancellation: &CancellationToken,
) -> Result<JobResult, String> {
    use portableforge_core::sbie_verified::{
        capture_with_options, preflight, CaptureRequest, CaptureStage,
    };
    preflight()?;
    let parent = match inputs.parent {
        Some(parent) => parent,
        None => {
            let default_parent = sessions_base();
            std::fs::create_dir_all(&default_parent).map_err(|error| error.to_string())?;
            default_parent
        }
    };
    let output = unique_folder(&parent, "Verified")?;
    let root = capture_with_options(
        CaptureRequest {
            setup: &inputs.setup,
            installed: inputs.installed.as_deref(),
            output: &output,
            setup_args: &inputs.args,
            companions: &inputs.companions,
            timeout: std::time::Duration::from_secs(600),
        },
        |state| {
            let _ = tx.send(JobEvent::Stage(match state {
                CaptureStage::Preparing => "Preparing the dedicated box…",
                CaptureStage::Installing => "Complete setup in its window.",
                CaptureStage::SelectingApplication => "Choose the app installed by setup.",
                CaptureStage::Exercising => "Try the app, then close it.",
                CaptureStage::CleaningUp => "Checking capture and removing the dedicated box…",
            }));
            ctx.request_repaint();
        },
        |paths| {
            let (reply, choice) = mpsc::channel();
            tx.send(JobEvent::Choose(CandidateChoice {
                paths: paths.to_vec(),
                page: 0,
                reply,
            }))
            .map_err(|_| "The wizard closed before choosing an installed app.")?;
            ctx.request_repaint();
            if cancellation.is_cancelled() {
                return Err("App selection was cancelled.".into());
            }
            choice
                .recv()
                .map_err(|_| "App selection was cancelled.".into())
        },
        cancellation,
    )?;
    load_cleaned_capture(&root, None)
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn load_cleaned_capture(root: &Path, expected: Option<&str>) -> Result<JobResult, String> {
    let (installed, receipt_sha256) =
        portableforge_core::sbie_verified::verify_capture_application(root, expected)?;
    let installed = installed.ok_or(
        "This older capture receipt has no selected app. Start a new capture to review the app.",
    )?;
    let layout = crate::session_layout_for(root);
    let outcome = portableforge_core::orchestrate::read_capture(&layout)?;
    let receipt = CleanedCapture {
        folder: root.to_path_buf(),
        receipt_sha256,
    };
    Ok(JobResult::Captured(Box::new(CaptureComplete {
        layout,
        outcome,
        installed,
        receipt,
        reopened: expected.is_some(),
    })))
}

fn save_job(
    session: &Path,
    meta: &portableforge_core::PafMetadata,
    review: &portableforge_core::review::Review,
    tools: &Path,
    destination: &Path,
    artwork: Option<&portableforge_core::artwork::Artwork>,
) -> Result<JobResult, String> {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        let result = portableforge_core::wizard_package::save_reviewed_profile_with_artwork(
            session,
            meta,
            review,
            tools,
            destination,
            artwork,
        )?;
        let folder = result
            .manifest
            .parent()
            .ok_or("The profile folder is missing.")?;
        Ok(JobResult::Saved(SavedProfileApproval {
            folder: folder.to_path_buf(),
            label: meta.name.clone(),
            manifest_sha256: result.manifest_sha256,
            details: settings::ProfileDetails {
                version: meta.display_version.clone(),
                setup_sha256: result.setup_sha256,
                builds: Vec::new(),
            },
        }))
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        let _ = (session, meta, review, tools, destination, artwork);
        Err("Saving this capture needs 64-bit Windows on an x64 computer.".into())
    }
}

fn rebuild_job(request: ReplayRequest) -> Result<JobResult, String> {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        use portableforge_core::{compile, wizard_package};
        let ReplayRequest {
            setup,
            approval,
            tools,
            destination,
        } = request;
        let result = wizard_package::build_reviewed_profile(
            &setup,
            &approval.folder,
            &approval.manifest_sha256,
            &tools,
            &destination,
        )?;
        let directory = result
            .artifact
            .parent()
            .ok_or("The installer folder is missing.")?;
        let verified = compile::verify_build_result(directory)?;
        if verified.sha256 != result.sha256
            || verified.app_id.as_deref() != Some(result.app_id.as_str())
            || verified
                .profile
                .as_ref()
                .map(|proof| proof.manifest_sha256.as_str())
                != Some(approval.manifest_sha256.as_str())
        {
            return Err("The installer receipt changed before it could be recorded.".into());
        }
        let build = SavedBuild {
            directory: directory.to_path_buf(),
            artifact: verified.artifact,
            manifest_sha256: verified.manifest_sha256,
            artifact_sha256: result.sha256,
            app_id: result.app_id,
            completed_utc: portableforge_core::timeutil::now_iso(),
        };
        Ok(JobResult::Rebuilt(Box::new((approval, build))))
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        let _ = (&request.setup, &request.tools);
        Err("Making this installer needs 64-bit Windows on an x64 computer.".into())
    }
}

fn profile_matches(approval: &SavedProfileApproval, query: &str) -> bool {
    [
        approval.label.as_str(),
        &approval.details.version,
        &approval.details.setup_sha256,
    ]
    .iter()
    .any(|text| text.to_lowercase().contains(query))
}

fn inspect_job(approval: &SavedProfileApproval) -> Result<JobResult, String> {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        let result =
            portableforge_core::profile::inspect(&approval.folder, &approval.manifest_sha256)?;
        let details = serde_json::to_string_pretty(&result).map_err(|error| error.to_string())?;
        Ok(JobResult::BuildDetails("Verified profile details", details))
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        let _ = approval;
        Err("Verification needs 64-bit Windows on an x64 computer.".into())
    }
}

fn relocate_job(approval: SavedProfileApproval, destination: &Path) -> Result<JobResult, String> {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        let result = portableforge_core::profile::relocate(
            &approval.folder,
            &approval.manifest_sha256,
            destination,
        )?;
        let folder = result
            .manifest
            .parent()
            .ok_or("The new profile folder is missing.")?
            .to_path_buf();
        Ok(JobResult::Relocated(approval, folder))
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        let _ = (approval, destination);
        Err("Moving a profile needs 64-bit Windows on an x64 computer.".into())
    }
}

fn compare_update_job(
    approval: &SavedProfileApproval,
    session: &Path,
    review: &portableforge_core::review::Review,
) -> Result<JobResult, String> {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        let comparison = portableforge_core::profile::compare_capture(
            &approval.folder,
            &approval.manifest_sha256,
            session,
            review,
        )?;
        let mut report = format!(
            "Setup file · {}\nSetup arguments · {}\nCompanion files · {}\nApp files changed · {}\n\nSaved choices\n{}\n\nNew findings\n{}",
            change(comparison.previous_setup_sha256 == comparison.current_setup_sha256),
            change(!comparison.setup_arguments_changed),
            change(!comparison.setup_companions_changed),
            comparison.payload_changes.len(),
            comparison.previous_review.summary(),
            comparison.current_review.summary(),
        );
        let review_changes =
            review_changes(&comparison.previous_review, &comparison.current_review);
        report.push_str(&format!(
            "\n\nReview findings changed · {}",
            review_changes.len()
        ));
        for line in review_changes {
            report.push('\n');
            report.push_str(&line);
        }
        report.push_str("\n\nApp file changes");
        for item in comparison.payload_changes {
            let status = match (item.previous_sha256, item.current_sha256) {
                (None, Some(_)) => "Added",
                (Some(_), None) => "Removed",
                _ => "Changed",
            };
            report.push_str(&format!("\n{status} · {}", item.relative_path));
        }
        Ok(JobResult::UpdateCompared(report))
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        let _ = (approval, session, review);
        Err("Comparing an update needs 64-bit Windows on an x64 computer.".into())
    }
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn review_changes(
    previous: &portableforge_core::review::Review,
    current: &portableforge_core::review::Review,
) -> Vec<String> {
    use portableforge_core::classify::Finding;
    use std::collections::BTreeMap;
    let mut rows = BTreeMap::<_, (Vec<_>, Vec<_>)>::new();
    for (before, review) in [(true, previous), (false, current)] {
        for item in &review.items {
            let key = match &item.classification.finding {
                Finding::DirectoriesMove { host_pattern, .. } => {
                    ("Settings folder", host_pattern.clone())
                }
                Finding::FilesMove {
                    host_pattern,
                    data_target,
                } => ("Settings file", format!("{host_pattern}\\{data_target}")),
                Finding::RegistryKey { key } => ("App setting", key.clone()),
                Finding::AssociationCandidate { key } => ("File association", key.clone()),
                Finding::UninstallMetadata { key } => ("Uninstall entry", key.clone()),
                Finding::AppFile { path } => ("App file", path.clone()),
                Finding::FileWriteRewrite { path, .. } => ("File path update", path.clone()),
                Finding::RedFlag { kind, detail } => ("Warning", format!("{kind} · {detail}")),
            };
            let (old, new) = rows.entry(key).or_default();
            if before {
                old.push(&item.classification);
            } else {
                new.push(&item.classification);
            }
        }
    }
    rows.into_iter()
        .filter_map(|((label, location), (old, new))| {
            let status = if old.is_empty() {
                "Added"
            } else if new.is_empty() {
                "Removed"
            } else if old != new {
                "Changed"
            } else {
                return None;
            };
            Some(format!("{status} · {label} · {location}"))
        })
        .collect()
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn change(same: bool) -> &'static str {
    if same {
        "Same"
    } else {
        "Changed"
    }
}

fn build_details_job(build: &SavedBuild) -> Result<JobResult, String> {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        let result = installer_test::BuildIdentity::from(build).verify()?;
        let details = serde_json::to_string_pretty(&result).map_err(|error| error.to_string())?;
        Ok(JobResult::BuildDetails("Verified build details", details))
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        let _ = build;
        Err("Reading build details needs 64-bit Windows on an x64 computer.".into())
    }
}

fn compare_builds_job(left: &SavedBuild, right: &SavedBuild) -> Result<JobResult, String> {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        let comparison =
            portableforge_core::compile::compare_build_results(&left.directory, &right.directory)?;
        installer_test::BuildIdentity::from(left).check(&comparison.left)?;
        installer_test::BuildIdentity::from(right).check(&comparison.right)?;
        let details = [
            ("Setup", comparison.same_setup),
            ("Package", comparison.same_package),
            ("App files", comparison.same_payload),
            ("Settings rules", comparison.same_configuration),
            ("Package icons", comparison.same_icon_set),
            ("Installer icon", Some(comparison.same_primary_icon)),
            ("Installer maker files", comparison.same_installer_support),
            (
                "Staged file times before normalization",
                comparison.same_staged_file_times,
            ),
            (
                "Compiler programs",
                Some(comparison.same_pinned_executables),
            ),
            (
                "Compiler exit codes",
                Some(comparison.same_compiler_exit_codes),
            ),
            ("Installer file", Some(comparison.same_artifact)),
        ]
        .iter()
        .map(|(label, same)| {
            format!(
                "{label} · {}",
                same.map_or("Unavailable in older receipts", change)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
        let details = format!(
            "{details}\n\nFirst installer SHA-256\n{}\n\nSecond installer SHA-256\n{}",
            comparison.left.sha256, comparison.right.sha256
        );
        Ok(JobResult::BuildDetails("Build comparison", details))
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        let _ = (left, right);
        Err("Comparing builds needs 64-bit Windows on an x64 computer.".into())
    }
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn runtime_job(build: &SavedBuild, cancellation: &CancellationToken) -> Result<JobResult, String> {
    let result = installer_test::run(build.into(), cancellation, || {})
        .map_err(|error| error.detail().to_owned())?;
    Ok(JobResult::BuildDetails(
        "Installer test result",
        result.details(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library_fixture() -> WizardApp {
        let mut wizard = WizardApp::default();
        wizard.settings.welcomed = true;
        wizard.profile_flow.replay_open = true;
        let builds = (0..8)
            .map(|index| SavedBuild {
                directory: PathBuf::from(format!("Build-{index}")),
                artifact: PathBuf::from(format!("Fixture_{index}.paf.exe")),
                manifest_sha256: "a".repeat(64),
                artifact_sha256: "b".repeat(64),
                app_id: "Fixture".into(),
                completed_utc: "2026-11-12 11:00 UTC".into(),
            })
            .collect();
        let approval = SavedProfileApproval {
            folder: "Fixture-Profile".into(),
            label: "Fixture".into(),
            manifest_sha256: "c".repeat(64),
            details: settings::ProfileDetails {
                version: "1.0".into(),
                builds,
                ..Default::default()
            },
        };
        wizard.settings.saved_profiles = (0..4)
            .map(|index| SavedProfileApproval {
                label: format!("Fixture {index}"),
                ..approval.clone()
            })
            .collect();
        wizard.profile_flow.replay_destination = r"C:\Builds\Fixture Portable".into();
        for index in 0..8 {
            wizard.profile_flow.queue.push(QueuedBuild {
                request: ReplayRequest {
                    setup: "Setup.exe".into(),
                    approval: approval.clone(),
                    tools: "Tools".into(),
                    destination: format!("Build-{index}").into(),
                },
                state: if index == 0 {
                    QueueState::Failed("The build input changed.".into())
                } else {
                    QueueState::Waiting
                },
            });
        }
        wizard
    }

    #[test]
    fn library_records_fit_at_every_text_preset() {
        for scale in settings::TEXT_SCALES {
            for (tab, expected) in [
                (
                    LibraryTab::Profiles,
                    &[
                        "Profile",
                        "Matching setup file",
                        "Choose parent…",
                        r"C:\Builds\Fixture Portable",
                        "Build installer",
                    ][..],
                ),
                (
                    LibraryTab::Queue,
                    &[
                        "× Build failed",
                        "View build log",
                        "Edit build inputs",
                        "Retry in a new folder…",
                        "Build queued apps",
                    ][..],
                ),
                (
                    LibraryTab::Results,
                    &["✓ Build recorded", "View build details", "Test installer"][..],
                ),
            ] {
                for size in [
                    dense::MIN_CONTENT,
                    [960.0, 540.0],
                    super::super::window_scale::CANVAS,
                ] {
                    assert_library_controls_fit(tab, expected, size, scale);
                    if size[0] >= 768.0 {
                        assert_library_controls_fit(
                            tab,
                            &["Roadmap", "Saved profiles", "Build queue", "Build results"],
                            size,
                            scale,
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn failed_build_reopens_its_approved_inputs_without_reusing_output() {
        let mut wizard = library_fixture();
        wizard.profile_flow.library_tab = LibraryTab::Queue;
        let first = wizard.profile_flow.queue[0].request.clone();
        wizard.profile_flow.replay_destination = "Previous form destination".into();
        wizard.edit_queued_build(0);
        let state = &wizard.profile_flow;
        assert!(state.library_tab == LibraryTab::Profiles);
        assert_eq!(
            wizard.settings.saved_profiles[state.selected_profile].manifest_sha256,
            first.approval.manifest_sha256
        );
        assert_eq!(entered(&state.replay_setup), first.setup);
        assert_eq!(entered(&state.installer_tools), first.tools);
        assert!(state.replay_destination.is_empty());
        assert_eq!(state.queue[0].request.destination, first.destination);
        assert!(
            matches!(&state.queue[0].state, QueueState::Failed(error) if error == "The build input changed.")
        );
    }

    fn assert_library_controls_fit(tab: LibraryTab, labels: &[&str], size: [f32; 2], scale: f32) {
        let mut wizard = library_fixture();
        wizard.profile_flow.library_tab = tab;
        assert_controls_fit(wizard, labels, size, scale);
    }

    #[test]
    fn paged_profile_dialogs_keep_their_actions_in_view() {
        for scale in settings::TEXT_SCALES {
            let mut setup = library_fixture();
            setup.profile_flow.replay_open = false;
            setup.enter_verified_capture();
            setup.profile_flow.setup_options_open = true;
            setup.profile_flow.setup_args =
                (0..40).map(|index| format!("--argument={index}")).collect();
            setup.profile_flow.companions = (0..32)
                .map(|index| PathBuf::from(format!("Part-{index}.cab")))
                .collect();
            assert_controls_fit(
                setup,
                &[
                    "Setup argument",
                    "Next ›",
                    "View arguments",
                    "View companion files",
                    "Done",
                ],
                dense::MIN_CONTENT,
                scale,
            );

            let mut actions = library_fixture();
            actions.profile_flow.profile_actions_open = true;
            actions.settings.saved_profiles[0].details.setup_sha256 = "d".repeat(64);
            assert_controls_fit(
                actions,
                &[
                    "View setup fingerprint",
                    "Capture an update",
                    "Relocate verified profile",
                    "Done",
                ],
                dense::MIN_CONTENT,
                scale,
            );

            let mut build = library_fixture();
            build.profile_flow.build_details =
                Some(("Build details", "A verified build detail.\n".repeat(100)));
            assert_controls_fit(
                build,
                &["Build details", "Next ›", "Copy full text", "Done"],
                dense::MIN_CONTENT,
                scale,
            );
        }
    }

    #[test]
    fn save_page_exposes_artwork_without_displacing_save() {
        for scale in [1.0, 1.25, 1.5] {
            let mut wizard = WizardApp::default();
            wizard.enter_verified_capture();
            wizard.step = 9;
            wizard.profile_flow.captured = true;
            wizard.profile_flow.launcher_tools = r"C:\Tools\PortableAppsLauncherGenerator".into();
            wizard.profile_flow.profile_destination = r"C:\Builds\Profile".into();
            wizard.layout = Some(portableforge_core::orchestrate::session_layout(
                Path::new(r"C:\Captures"),
                "artwork-test",
            ));
            assert_controls_fit(
                wizard,
                &[
                    "New profile folder",
                    "Edit package artwork…",
                    "Default artwork",
                    "Save profile",
                ],
                dense::MIN_CONTENT,
                scale,
            );
        }
    }

    fn assert_controls_fit(mut wizard: WizardApp, labels: &[&str], size: [f32; 2], scale: f32) {
        wizard.settings.text_scale = scale;
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let bounds = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::from(size));
        let mut frame = eframe::Frame::_new_kittest();
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
            }).unwrap_or_else(|| panic!("Missing {label:?} at {scale} in {size:?}"));
            let visible = bounds.intersect(shape.clip_rect).expand(1.0);
            assert!(
                visible.contains_rect(shape.shape.visual_bounding_rect()),
                "{label:?} clipped at {scale} in {size:?}"
            );
        }
    }
}
