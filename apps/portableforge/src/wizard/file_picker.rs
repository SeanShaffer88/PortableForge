//! File selection and confirmations rendered inside the wizard viewport.

use std::{
    any::Any,
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use eframe::egui;

use super::dense;

const MAX_ENTRIES: usize = 4096;
const MAX_SELECTION: usize = 20_000;
const MAX_PATH_CHARS: usize = 32_767;
static LISTING_RUNNING: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy)]
pub(super) enum Mode {
    File(&'static [&'static str]),
    Files(&'static [&'static str]),
    Folder,
    Save(&'static str, &'static [&'static str]),
}

impl Mode {
    fn allows_file(self, path: &Path) -> bool {
        let extensions = match self {
            Self::File(extensions) | Self::Files(extensions) | Self::Save(_, extensions) => {
                extensions
            }
            Self::Folder => return false,
        };
        extensions.is_empty()
            || path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    extensions
                        .iter()
                        .any(|expected| extension.eq_ignore_ascii_case(expected))
                })
    }
}

#[derive(Clone)]
struct Entry {
    name: String,
    path: PathBuf,
    folder: bool,
}

#[derive(Default)]
struct Listing {
    entries: Vec<Entry>,
    limited: bool,
}

type Loading = Arc<Mutex<Option<Result<Listing, String>>>>;

#[derive(Clone)]
struct Browser {
    mode: Mode,
    directory: PathBuf,
    location: String,
    filename: String,
    selected: BTreeSet<PathBuf>,
    entries: Arc<Listing>,
    loading: Option<Loading>,
    page: usize,
    error: Option<String>,
}

impl Browser {
    fn new(ctx: &egui::Context, mode: Mode, start: Option<PathBuf>) -> Self {
        let start = start
            .or_else(home)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let mut browser = Self {
            mode,
            directory: PathBuf::new(),
            location: String::new(),
            filename: match mode {
                Mode::Save(name, _) => name.to_owned(),
                _ => String::new(),
            },
            selected: BTreeSet::new(),
            entries: Arc::new(Listing::default()),
            loading: None,
            page: 0,
            error: None,
        };
        browser.go(ctx, start);
        browser
    }

    fn go(&mut self, ctx: &egui::Context, directory: PathBuf) {
        if self.loading.is_some() {
            return;
        }
        if !directory.is_absolute() {
            self.error = Some("Enter a full folder path, such as C:\\Downloads.".into());
            return;
        }
        if LISTING_RUNNING.swap(true, Ordering::AcqRel) {
            self.error =
                Some("Wait for the previous folder to finish opening, then try again.".into());
            return;
        }
        let loading = Loading::default();
        let result = Arc::clone(&loading);
        let ctx = ctx.clone();
        let path = directory.clone();
        let mode = self.mode;
        match std::thread::Builder::new()
            .name("portableforge-file-list".into())
            .spawn(move || {
                let listing = list(&path, mode)
                    .map_err(|error| format!("Could not open this folder: {error}"));
                if let Ok(mut result) = result.lock() {
                    *result = Some(listing);
                }
                LISTING_RUNNING.store(false, Ordering::Release);
                ctx.request_repaint();
            }) {
            Ok(_) => {
                self.loading = Some(loading);
                self.location = directory.display().to_string();
                self.directory = directory;
                self.selected.clear();
                self.entries = Arc::new(Listing::default());
                self.page = 0;
                self.error = None;
            }
            Err(_) => {
                LISTING_RUNNING.store(false, Ordering::Release);
                self.error = Some("Could not start browsing for files.".into());
            }
        }
    }

    fn poll(&mut self) {
        let result = self
            .loading
            .as_ref()
            .and_then(|loading| loading.lock().ok()?.take());
        if let Some(result) = result {
            self.loading = None;
            match result {
                Ok(listing) => self.entries = Arc::new(listing),
                Err(error) => self.error = Some(error),
            }
        }
    }

    fn draw(&mut self, ui: &mut egui::Ui, focus: bool) -> Option<Vec<PathBuf>> {
        self.poll();
        let label = ui.label("Folder path");
        let location = ui
            .add(
                egui::TextEdit::singleline(&mut self.location)
                    .desired_width(f32::INFINITY)
                    .char_limit(MAX_PATH_CHARS),
            )
            .labelled_by(label.id);
        dense::field_help(
            &location,
            "Paste a full folder path, then press Enter or choose Go to folder.",
        );
        if focus {
            location.request_focus();
        }
        ui.horizontal(|ui| {
            let enter =
                location.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if dense::explained(
                ui.button("Go to folder"),
                "Show the contents of the folder in the path field.",
            )
            .clicked()
                || enter
            {
                self.go(ui.ctx(), PathBuf::from(self.location.trim()));
            }
            if let Some(parent) = self.directory.parent() {
                if dense::explained(
                    ui.button("Up one folder"),
                    "Open the folder that contains this one.",
                )
                .clicked()
                {
                    self.go(ui.ctx(), parent.to_path_buf());
                }
            }
            if let Some(home) = home() {
                if dense::explained(
                    ui.button("Home folder"),
                    "Open your Windows user folder, where Downloads and Desktop usually appear.",
                )
                .clicked()
                {
                    self.go(ui.ctx(), home);
                }
            }
        });

        let listing = Arc::clone(&self.entries);
        if self.loading.is_some() {
            ui.label("Reading this folder…");
        } else if listing.entries.is_empty() && self.error.is_none() {
            ui.label("This folder has no matching files or subfolders.");
        }
        if listing.limited {
            ui.label("This folder has more entries. Open a subfolder or enter a full path.");
        }
        let rows = if ui.ctx().content_rect().height() < 560.0 {
            4
        } else {
            7
        };
        let range = dense::page(ui, &mut self.page, listing.entries.len(), rows, "Items");
        for entry in &listing.entries[range] {
            ui.push_id(&entry.path, |ui| {
                if entry.folder {
                    if dense::explained(
                        ui.add(egui::Button::new(format!("Folder · {}", entry.name)).truncate()),
                        entry.name.as_str(),
                    )
                    .clicked()
                    {
                        self.go(ui.ctx(), entry.path.clone());
                    }
                } else if matches!(self.mode, Mode::Files(_)) {
                    let mut selected = self.selected.contains(&entry.path);
                    if ui
                        .scope(|ui| {
                            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                            dense::explained(
                                ui.checkbox(&mut selected, &entry.name),
                                entry.name.as_str(),
                            )
                            .changed()
                        })
                        .inner
                    {
                        if !selected {
                            self.selected.remove(&entry.path);
                        } else if self.selected.len() < MAX_SELECTION {
                            self.selected.insert(entry.path.clone());
                        }
                    }
                } else {
                    let active = self.filename == entry.name;
                    if dense::explained(
                        ui.add(egui::Button::selectable(active, &entry.name).truncate()),
                        entry.name.as_str(),
                    )
                    .clicked()
                    {
                        self.filename.clone_from(&entry.name);
                        self.error = None;
                    }
                }
            });
        }
        if !matches!(self.mode, Mode::Folder) {
            let text = if matches!(self.mode, Mode::Save(..)) {
                "File name"
            } else {
                "File name or full path"
            };
            let label = ui.label(text);
            let filename = ui
                .add(
                    egui::TextEdit::singleline(&mut self.filename)
                        .desired_width(f32::INFINITY)
                        .char_limit(MAX_PATH_CHARS),
                )
                .labelled_by(label.id);
            let help = if matches!(self.mode, Mode::Save(..)) {
                "Enter a new file name. Keep the extension shown in the suggested name."
            } else {
                "Choose a file above or paste its full path. Check the boxes to choose multiple files."
            };
            dense::field_help(&filename, help);
        }
        if matches!(self.mode, Mode::Files(_)) && !self.selected.is_empty() {
            ui.label(format!("{} selected", self.selected.len()));
        }
        if let Some(error) = &self.error {
            dense::explained(
                ui.add(
                    egui::Label::new(egui::RichText::new(error).color(ui.visuals().error_fg_color))
                        .truncate(),
                ),
                error,
            );
        }
        let action = match self.mode {
            Mode::Folder => "Choose this folder",
            Mode::Save(..) => "Save file",
            Mode::File(..) => "Choose file",
            Mode::Files(..) => "Choose files",
        };
        if dense::primary_action(ui, self.loading.is_none(), action).clicked() {
            match self.pick() {
                Ok(paths) => return Some(paths),
                Err(error) => self.error = Some(error.into()),
            }
        }
        None
    }

    fn pick(&self) -> Result<Vec<PathBuf>, &'static str> {
        if let Mode::Folder = self.mode {
            return self
                .directory
                .is_dir()
                .then(|| vec![self.directory.clone()])
                .ok_or("Open an existing folder first.");
        }
        if !self.selected.is_empty() {
            return Ok(self.selected.iter().cloned().collect());
        }
        let path = self.filename.trim();
        if path.is_empty() {
            return Err("Select a file or enter its name.");
        }
        if path.chars().count() > MAX_PATH_CHARS {
            return Err("The file path is too long.");
        }
        let input = Path::new(path);
        let path = if input.is_absolute() {
            input.to_owned()
        } else {
            self.directory.join(input)
        };
        if matches!(self.mode, Mode::Save(..)) {
            if !self.mode.allows_file(&path) {
                return Err("Use the file extension shown in the suggested name.");
            }
            if !path.parent().is_some_and(Path::is_dir) {
                return Err("Choose an existing folder for the file.");
            }
            if path.symlink_metadata().is_ok() {
                return Err("That file already exists. Choose a new name.");
            }
        } else if !path.is_file() || !self.mode.allows_file(&path) {
            return Err("Choose an existing file with a supported extension.");
        }
        Ok(vec![path])
    }
}

fn list(directory: &Path, mode: Mode) -> std::io::Result<Listing> {
    let mut listing = Listing::default();
    let mut entries = fs::read_dir(directory)?;
    for entry in (&mut entries).take(MAX_ENTRIES).flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() || (kind.is_file() && mode.allows_file(&path)) {
            listing.entries.push(Entry {
                name: entry.file_name().to_string_lossy().into_owned(),
                path,
                folder: kind.is_dir(),
            });
        }
    }
    listing.limited = entries.next().is_some();
    listing
        .entries
        .sort_by(|a, b| b.folder.cmp(&a.folder).then_with(|| a.name.cmp(&b.name)));
    Ok(listing)
}

fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

#[derive(Clone)]
enum Request {
    Browse(Box<Browser>),
    Confirm(String, Arc<dyn Any + Send + Sync>),
}

#[derive(Clone)]
struct Dialog {
    opener: egui::Id,
    title: String,
    first: bool,
    request: Request,
}

#[derive(Clone)]
struct Answer {
    opener: egui::Id,
    value: Selection,
    expires: u64,
}

#[derive(Clone)]
enum Selection {
    Paths(Vec<PathBuf>),
    Confirmed(Arc<dyn Any + Send + Sync>),
}

fn dialog_id() -> egui::Id {
    egui::Id::new("wizard-file-picker")
}

pub(super) fn is_open(ctx: &egui::Context) -> bool {
    ctx.data_mut(|data| data.get_temp::<Dialog>(dialog_id()).is_some())
}

pub(super) fn is_active(ctx: &egui::Context) -> bool {
    ctx.data_mut(|data| {
        data.get_temp::<Dialog>(dialog_id()).is_some()
            || data.get_temp::<Answer>(dialog_id()).is_some()
            || data
                .get_temp::<Option<egui::Id>>(dialog_id())
                .flatten()
                .is_some()
    })
}

pub(super) fn choose(
    opener: &egui::Response,
    title: &str,
    mode: Mode,
    start: Option<PathBuf>,
) -> Option<Vec<PathBuf>> {
    let choice = match answer(opener) {
        Some(Selection::Paths(paths)) => Some(paths),
        _ => None,
    };
    if opener.clicked() && !is_open(&opener.ctx) {
        let browser = Browser::new(&opener.ctx, mode, start);
        open(opener, title, Request::Browse(Box::new(browser)));
    }
    choice
}

pub(super) fn confirmed<T: Clone + Send + Sync + 'static>(
    opener: &egui::Response,
    title: &str,
    description: &str,
    value: Option<T>,
) -> Option<T> {
    let confirmed = match answer(opener) {
        Some(Selection::Confirmed(value)) => value.downcast_ref::<T>().cloned(),
        _ => None,
    };
    if opener.clicked() && !is_open(&opener.ctx) {
        if let Some(value) = value {
            open(
                opener,
                title,
                Request::Confirm(description.to_owned(), Arc::new(value)),
            );
        }
    }
    confirmed
}

fn answer(opener: &egui::Response) -> Option<Selection> {
    opener.ctx.data_mut(|data| {
        let answer = data.get_temp::<Answer>(dialog_id())?;
        if answer.opener == opener.id {
            data.remove::<Answer>(dialog_id());
            Some(answer.value)
        } else {
            None
        }
    })
}

fn open(opener: &egui::Response, title: &str, request: Request) {
    opener.ctx.data_mut(|data| {
        data.remove::<Answer>(dialog_id());
        data.insert_temp(
            dialog_id(),
            Dialog {
                opener: opener.id,
                title: title.into(),
                first: true,
                request,
            },
        );
    });
    opener.ctx.request_repaint();
}

pub(super) fn show(ctx: &egui::Context) {
    let pass = ctx.cumulative_pass_nr();
    let dialog = ctx.data_mut(|data| {
        let dialog = data.get_temp::<Dialog>(dialog_id());
        data.remove::<Dialog>(dialog_id());
        if data
            .get_temp::<Answer>(dialog_id())
            .is_some_and(|answer| pass >= answer.expires)
        {
            data.remove::<Answer>(dialog_id());
        }
        dialog
    });
    let Some(mut dialog) = dialog else {
        let still_modal = ctx.memory(|memory| {
            memory.top_modal_layer() == Some(egui::Modal::default_area(dialog_id()).layer())
        });
        if still_modal {
            ctx.request_repaint();
        } else {
            if let Some(opener) =
                ctx.data_mut(|data| data.remove_temp::<Option<egui::Id>>(dialog_id()).flatten())
            {
                ctx.memory_mut(|memory| memory.request_focus(opener));
            }
        }
        return;
    };
    let response = egui::Modal::new(dialog_id()).show(ctx, |ui| {
        ui.set_width((ctx.content_rect().width() - 60.0).clamp(360.0, 660.0));
        dense::compact_body(ui);
        ui.heading(&dialog.title);
        let picked = match &mut dialog.request {
            Request::Browse(browser) => browser.draw(ui, dialog.first).map(Selection::Paths),
            Request::Confirm(description, value) => {
                ui.label(description.as_str());
                dense::primary_action(ui, true, "Confirm")
                    .clicked()
                    .then(|| Selection::Confirmed(Arc::clone(value)))
            }
        };
        let cancel = dense::explained(
            ui.button("Cancel"),
            "Close this panel and keep your previous choices.",
        );
        if dialog.first && matches!(dialog.request, Request::Confirm(..)) {
            cancel.request_focus();
        }
        if cancel.clicked() {
            ui.close();
        }
        dense::help_strip(ui, "Use Tab to move between controls. Press Esc to cancel.");
        picked
    });
    dialog.first = false;
    // Escape or clicking outside cancels without changing the form.
    if response.inner.is_some() || response.should_close() {
        let pass = ctx.cumulative_pass_nr();
        if let Some(value) = response.inner {
            ctx.data_mut(|data| {
                data.insert_temp(
                    dialog_id(),
                    Answer {
                        opener: dialog.opener,
                        value,
                        expires: pass + 2,
                    },
                )
            });
        }
        let opener = dialog.opener;
        ctx.data_mut(|data| data.insert_temp(dialog_id(), Some(opener)));
        ctx.request_repaint();
    } else {
        ctx.data_mut(|data| data.insert_temp(dialog_id(), dialog));
    }
}

pub(super) fn save_new(path: &Path, text: String) -> std::io::Result<()> {
    use std::io::Write as _;
    let parent = path.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(text.as_bytes())?;
    file.as_file().sync_all()?;
    file.persist_noclobber(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_matching_files_and_folders() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let listing = list(root, Mode::File(&["TOML"])).expect("manifest folder");
        assert!(listing
            .entries
            .iter()
            .any(|entry| entry.folder && entry.name == "src"));
        assert!(listing
            .entries
            .iter()
            .any(|entry| !entry.folder && entry.name == "Cargo.toml"));
        assert!(listing
            .entries
            .iter()
            .all(|entry| entry.folder || entry.path.extension().unwrap() == "toml"));
    }

    #[test]
    fn typed_absolute_paths_and_explicit_multiple_files_remain_distinct() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let existing = root.join("Cargo.toml");
        let mut browser = Browser {
            mode: Mode::File(&["toml"]),
            directory: root.to_owned(),
            location: String::new(),
            filename: existing.display().to_string(),
            selected: BTreeSet::new(),
            entries: Arc::new(Listing::default()),
            loading: None,
            page: 0,
            error: None,
        };
        assert_eq!(browser.pick().unwrap(), std::slice::from_ref(&existing));
        browser.mode = Mode::Files(&[]);
        browser.selected.insert(existing);
        browser.selected.insert(root.join("src/main.rs"));
        assert_eq!(browser.pick().unwrap().len(), 2);
        browser.selected.clear();
        browser.mode = Mode::Save("Cargo.toml", &["toml"]);
        assert_eq!(
            browser.pick().unwrap_err(),
            "That file already exists. Choose a new name."
        );
    }

    #[test]
    fn export_rejects_an_existing_destination_without_leaving_a_temporary_file() {
        let folder = tempfile::tempdir().unwrap();
        let output = folder.path().join("report.txt");
        save_new(&output, "original".into()).unwrap();
        assert_eq!(
            save_new(&output, "replacement".into()).unwrap_err().kind(),
            std::io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read_to_string(output).unwrap(), "original");
        assert_eq!(fs::read_dir(folder.path()).unwrap().count(), 1);
    }

    #[test]
    fn modal_fit_and_cancel_restores_focus_without_selection() {
        let ctx = egui::Context::default();
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 480.0));
        let id = egui::Id::new("test-opener");
        let opener = std::cell::RefCell::new(None);
        let listing = Arc::new(Listing {
            entries: (0..10)
                .map(|index| Entry {
                    path: PathBuf::from(format!("{index}.zip")),
                    name: format!("Package{index}.zip"),
                    folder: false,
                })
                .collect(),
            limited: true,
        });
        let browser = Browser {
            mode: Mode::Files(&["zip"]),
            directory: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
            location: env!("CARGO_MANIFEST_DIR").into(),
            filename: String::new(),
            selected: BTreeSet::from([PathBuf::from("0.zip")]),
            entries: listing,
            loading: None,
            page: 0,
            error: Some("Could not open this folder.".into()),
        };
        let draw = |events| {
            let input = egui::RawInput {
                screen_rect: Some(viewport),
                events,
                ..Default::default()
            };
            ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    *opener.borrow_mut() = Some(ui.interact(viewport, id, egui::Sense::click()));
                });
                show(ctx);
            })
        };
        draw(Vec::new());
        open(
            opener.borrow().as_ref().unwrap(),
            "Choose a ZIP or files for one app",
            Request::Browse(Box::new(browser)),
        );
        draw(Vec::new());
        let output = draw(Vec::new());
        for name in ["Folder path", "Choose files", "Cancel"] {
            let shape = output.shapes.iter().find(|shape| {
                matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == name)
            }).expect("visible picker control");
            assert!(
                viewport.contains_rect(shape.shape.visual_bounding_rect()),
                "{name}"
            );
        }
        draw(vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        draw(Vec::new());
        draw(Vec::new());
        assert!(!is_open(&ctx));
        assert!(answer(opener.borrow().as_ref().unwrap()).is_none());
        assert_eq!(ctx.memory(|memory| memory.focused()), Some(id));
    }
}
