//! Classify and stage user-selected application inputs without running them.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use super::{
    error, hold_directory_chain, io_error, is_link, open_regular, safe_name, DirectoryGuard,
    ScanError, MAX_ENTRIES,
};

mod archive;
mod stage;
use stage::Workspace;

const MAX_INPUT_BYTES: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DroppedInput {
    Folder,
    Setup,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DropPlan {
    Folder(PathBuf),
    Setup(PathBuf),
    Zip(PathBuf),
    Files(Vec<PathBuf>),
}

/// An owned, temporary scan root. Keep it alive while using its staged files.
#[derive(Debug)]
pub struct StagedInput {
    root: PathBuf,
    original: PathBuf,
    label: String,
    _workspace: Workspace,
}

impl StagedInput {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn original(&self) -> &Path {
        &self.original
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    /// Release the staged input and report any cleanup failure.
    pub fn cleanup(mut self) -> Result<(), ScanError> {
        self._workspace.cleanup()
    }
}

pub fn classify_drop(path: &Path) -> Result<DroppedInput, ScanError> {
    match inspect(path)? {
        InputKind::Folder => Ok(DroppedInput::Folder),
        InputKind::Exe | InputKind::Msi => Ok(DroppedInput::Setup),
        _ => Err(error(
            "Choose an unpacked app folder or a setup file ending in .exe or .msi.",
        )),
    }
}

/// Classify an explicit selection. A loose file group belongs to one folder
/// and contains at least one application executable.
pub fn plan_drop(paths: Vec<PathBuf>) -> Result<DropPlan, ScanError> {
    plan_drop_with_cancel(paths, || false)
}

/// Classify a selection, allowing the caller to stop a large loose-file scan.
pub fn plan_drop_with_cancel(
    paths: Vec<PathBuf>,
    cancelled: impl Fn() -> bool,
) -> Result<DropPlan, ScanError> {
    check_cancel(&cancelled)?;
    if paths.is_empty() {
        return Err(error(
            "Choose an app folder, setup, ZIP, or a group of app files.",
        ));
    }
    if paths.len() > MAX_ENTRIES {
        return Err(error(
            "The selection contains too many files. Choose an app folder instead.",
        ));
    }
    if paths.len() == 1 {
        let path = paths.into_iter().next().expect("one path");
        let kind = inspect(&path)?;
        check_cancel(&cancelled)?;
        return match kind {
            InputKind::Folder => Ok(DropPlan::Folder(path)),
            InputKind::Exe | InputKind::Msi => Ok(DropPlan::Setup(path)),
            InputKind::Zip => Ok(DropPlan::Zip(path)),
            InputKind::File => Err(error(
                "Choose the app executable and its companion files together.",
            )),
        };
    }
    validate_loose(&paths, &cancelled)?;
    check_cancel(&cancelled)?;
    Ok(DropPlan::Files(paths))
}

/// Materialize a ZIP or explicit loose files in an owned, private directory.
/// Folder and setup inputs already have their own direct workflows.
pub fn stage_input(plan: DropPlan) -> Result<StagedInput, ScanError> {
    stage_input_with_cancel(plan, || false)
}

/// Cooperatively stop validation and staging when the caller cancels.
pub fn stage_input_with_cancel(
    plan: DropPlan,
    cancelled: impl Fn() -> bool,
) -> Result<StagedInput, ScanError> {
    check_cancel(&cancelled)?;
    let (original, label) = match &plan {
        DropPlan::Zip(path) if inspect(path)? == InputKind::Zip => {
            (path.clone(), display_name(path.file_stem()))
        }
        DropPlan::Files(paths) => {
            validate_loose(paths, &cancelled)?;
            let parent = paths[0].parent().expect("validated file parent");
            (parent.to_path_buf(), display_name(parent.file_name()))
        }
        _ => return Err(error("Only a ZIP or a group of app files needs staging.")),
    };
    check_cancel(&cancelled)?;
    let mut workspace = Workspace::new()?;
    let root = workspace.path().join(&label);
    let prepared = (|| {
        fs::create_dir(&root).map_err(io_error)?;
        let root = match plan {
            DropPlan::Zip(path) => {
                let (_guard, mut file) = open_selected(&path)?;
                archive::unpack(&mut file, &root, &cancelled)?
            }
            DropPlan::Files(paths) => {
                copy_selected(&paths, &root, &cancelled)?;
                root
            }
            _ => unreachable!("plan already validated"),
        };
        check_cancel(&cancelled)?;
        Ok(root)
    })();
    match prepared {
        Ok(root) => Ok(StagedInput {
            root,
            original,
            label,
            _workspace: workspace,
        }),
        Err(cause) => match workspace.cleanup() {
            Ok(()) => Err(cause),
            Err(cleanup) => Err(error(format!("{cause}\n{cleanup}"))),
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InputKind {
    Folder,
    Exe,
    Msi,
    Zip,
    File,
}

fn inspect(path: &Path) -> Result<InputKind, ScanError> {
    if !path.is_absolute() {
        return Err(error("Choose a file or folder on a local drive."));
    }
    let Some(parent) = path.parent() else {
        hold_directory_chain(path)?;
        return Ok(InputKind::Folder);
    };
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| safe_name(name))
        .ok_or_else(|| error("Choose a file or folder with a plain name."))?;
    let _parents = hold_directory_chain(parent)?;
    let metadata = fs::symlink_metadata(path).map_err(io_error)?;
    if is_link(&metadata) {
        return Err(error(
            "Choose the original file or folder instead of a link.",
        ));
    }
    if metadata.is_dir() {
        return Ok(InputKind::Folder);
    }
    if !metadata.is_file() {
        return Err(error("Choose an ordinary file or folder."));
    }
    open_regular(path)?;
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default();
    Ok(if extension.eq_ignore_ascii_case("exe") {
        InputKind::Exe
    } else if extension.eq_ignore_ascii_case("msi") {
        InputKind::Msi
    } else if extension.eq_ignore_ascii_case("zip") {
        InputKind::Zip
    } else {
        InputKind::File
    })
}

fn validate_loose(paths: &[PathBuf], cancelled: &dyn Fn() -> bool) -> Result<(), ScanError> {
    if !(2..=MAX_ENTRIES).contains(&paths.len()) {
        return Err(error(
            "Choose at least two app files, or drop the app folder.",
        ));
    }
    let first = paths[0].parent();
    let mut names = BTreeSet::new();
    let mut has_exe = false;
    for path in paths {
        check_cancel(cancelled)?;
        let kind = inspect(path)?;
        if !matches!(kind, InputKind::Exe | InputKind::File) {
            return Err(error("Drop a folder, setup, or ZIP by itself. For an unpacked app, select only its loose files."));
        }
        if path.parent() != first {
            return Err(error("Select app files from the same folder, or drop the app folder to preserve its subfolders."));
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| crate::path_safety::safe_name(name))
            .ok_or_else(|| error("An app file name is unsupported. Choose a plain file name."))?;
        if !names.insert(name.to_uppercase()) {
            return Err(error(
                "The selection contains duplicate or conflicting file names.",
            ));
        }
        if kind == InputKind::Exe {
            if installer_name(name) {
                return Err(error(
                    "Drop the setup executable by itself to capture its installation.",
                ));
            }
            has_exe = true;
        }
    }
    if !has_exe {
        return Err(error(
            "Select the application .exe along with its companion files.",
        ));
    }
    Ok(())
}

fn installer_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let stem = lower.strip_suffix(".exe").unwrap_or(&lower);
    stem.ends_with(".paf")
        || stem
            .split(|character: char| !character.is_ascii_alphanumeric())
            .any(|part| matches!(part, "setup" | "install" | "installer" | "uninstall"))
        || stem.starts_with("setup")
        || stem.ends_with("setup")
}

fn display_name(name: Option<&std::ffi::OsStr>) -> String {
    name.and_then(|name| name.to_str())
        .filter(|name| crate::path_safety::safe_name(name))
        .unwrap_or("Application")
        .to_owned()
}

fn open_selected(path: &Path) -> Result<(Vec<DirectoryGuard>, File), ScanError> {
    let parent = path
        .parent()
        .ok_or_else(|| error("Choose an ordinary local file."))?;
    let guard = hold_directory_chain(parent)?;
    let file = open_regular(path)?;
    Ok((guard, file))
}

fn copy_selected(
    paths: &[PathBuf],
    root: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), ScanError> {
    let mut total = 0;
    for path in paths {
        check_cancel(cancelled)?;
        let (_guard, file) = open_selected(path)?;
        let length = file.metadata().map_err(io_error)?.len();
        total = bounded_total(total, length)?;
        let name = path.file_name().expect("validated name");
        write_bounded(file, &root.join(name), length, cancelled)?;
    }
    Ok(())
}

fn bounded_total(total: u64, length: u64) -> Result<u64, ScanError> {
    total
        .checked_add(length)
        .filter(|total| *total <= MAX_INPUT_BYTES)
        .ok_or_else(|| {
            error("App files exceed the 4 GiB staging limit. Choose a smaller app folder.")
        })
}

fn check_cancel(cancelled: &dyn Fn() -> bool) -> Result<(), ScanError> {
    if cancelled() {
        return Err(error("Input staging stopped."));
    }
    Ok(())
}

fn write_bounded(
    source: impl Read,
    path: &Path,
    length: u64,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), ScanError> {
    check_cancel(cancelled)?;
    let mut source = source.take(length + 1);
    let mut target = File::options()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(io_error)?;
    let mut buffer = [0; 64 * 1024];
    let mut copied = 0;
    loop {
        check_cancel(cancelled)?;
        let read = source.read(&mut buffer).map_err(io_error)?;
        if read == 0 {
            break;
        }
        copied += read as u64;
        if copied > length {
            break;
        }
        target.write_all(&buffer[..read]).map_err(io_error)?;
    }
    if copied != length {
        return Err(error(
            "An input file changed or its actual size differs from the declared size.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn classifies_a_folder_with_an_executable_suffix_and_regular_setup() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("pf-drop-{}-{stamp}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let folder = root.join("App.exe");
        std::fs::create_dir(&folder).unwrap();
        let setup = root.join("Setup.MSI");
        std::fs::write(&setup, b"inert").unwrap();
        let other = root.join("Other.txt");
        std::fs::write(&other, b"inert").unwrap();
        assert_eq!(classify_drop(&folder).unwrap(), DroppedInput::Folder);
        assert_eq!(classify_drop(&setup).unwrap(), DroppedInput::Setup);
        assert!(classify_drop(&other).is_err());
        std::fs::remove_file(other).unwrap();
        std::fs::remove_file(setup).unwrap();
        std::fs::remove_dir(folder).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[cfg(any(unix, all(windows, target_arch = "x86_64")))]
    mod staging {
        use super::*;
        use std::cell::Cell;
        use zip::write::SimpleFileOptions;
        use zip::{CompressionMethod, ZipWriter};

        fn write_zip(path: &Path, files: &[(&str, &[u8], CompressionMethod)]) {
            let mut writer = ZipWriter::new(File::create(path).unwrap());
            for &(name, data, method) in files {
                let options = SimpleFileOptions::default().compression_method(method);
                if name.ends_with('/') {
                    writer.add_directory(name, options).unwrap();
                } else {
                    writer.start_file(name, options).unwrap();
                    writer.write_all(data).unwrap();
                }
            }
            writer.finish().unwrap();
        }

        #[test]
        fn stages_stored_and_deflated_entries_and_keeps_archive_unchanged() {
            let fixture = Workspace::new().unwrap();
            let path = fixture.path().join("Widget.zip");
            write_zip(
                &path,
                &[
                    ("Widget/", b"", CompressionMethod::Stored),
                    (
                        "Widget/bin/Widget.exe",
                        b"not a launched executable",
                        CompressionMethod::Stored,
                    ),
                    (
                        "Widget/bin/data.cfg",
                        b"compression fixture",
                        CompressionMethod::Deflated,
                    ),
                    ("Widget/empty/", b"", CompressionMethod::Stored),
                ],
            );
            let original = fs::read(&path).unwrap();
            let plan = plan_drop(vec![path.clone()]).unwrap();
            assert_eq!(plan, DropPlan::Zip(path.clone()));
            let staged = stage_input(plan).unwrap();
            assert_eq!(staged.original(), path);
            assert_eq!(staged.label(), "Widget");
            let root = staged.root().to_path_buf();
            assert_eq!(root.file_name().unwrap(), "Widget");
            assert_eq!(
                fs::read(root.join("bin/Widget.exe")).unwrap(),
                b"not a launched executable"
            );
            assert_eq!(
                fs::read(root.join("bin/data.cfg")).unwrap(),
                b"compression fixture"
            );
            assert!(root.join("empty").is_dir());
            assert_eq!(fs::read(&path).unwrap(), original);
            let workspace = staged._workspace.path().to_path_buf();
            drop(staged);
            assert!(!workspace.exists());
        }

        #[test]
        fn stages_only_the_selected_loose_files_and_rejects_mixed_inputs() {
            let fixture = Workspace::new().unwrap();
            let root = fixture.path();
            let exe = root.join("Widget.exe");
            let dll = root.join("Widget.dll");
            let helper = root.join("Helper.exe");
            let setup = root.join("Widget-Setup.exe");
            let other = root.join("other");
            fs::create_dir(&other).unwrap();
            let elsewhere = other.join("settings.ini");
            for path in [&exe, &dll, &helper, &setup, &elsewhere] {
                fs::write(path, path.file_name().unwrap().to_str().unwrap()).unwrap();
            }
            assert!(plan_drop(vec![exe.clone(), elsewhere])
                .unwrap_err()
                .message
                .contains("same folder"));
            assert!(plan_drop(vec![exe.clone(), setup])
                .unwrap_err()
                .message
                .contains("by itself"));
            assert!(plan_drop(vec![exe.clone(), exe.clone()])
                .unwrap_err()
                .message
                .contains("duplicate"));
            assert!(plan_drop(vec![exe.clone(), other]).is_err());
            let plan = plan_drop(vec![exe, dll, helper]).unwrap();
            let staged = stage_input(plan).unwrap();
            assert_eq!(staged.original(), root);
            let mut names: Vec<_> = fs::read_dir(staged.root())
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect();
            names.sort();
            assert_eq!(names, ["Helper.exe", "Widget.dll", "Widget.exe"]);
            assert_eq!(
                fs::read(staged.root().join("Widget.dll")).unwrap(),
                b"Widget.dll"
            );
            let work = staged._workspace.path().to_path_buf();
            drop(staged);
            assert!(!work.exists());
        }

        #[test]
        fn rejects_case_conflicts_file_prefixes_and_unsafe_paths() {
            let fixture = Workspace::new().unwrap();
            let path = fixture.path().join("Widget.zip");
            for (left, right) in [
                ("Assets/first", "assets/second"),
                ("assets", "assets/icon"),
                ("Widget.exe", "dir/../bad"),
                ("Widget.exe", "folder/CON.txt"),
                ("Widget.exe", "folder/name:stream"),
            ] {
                write_zip(
                    &path,
                    &[
                        (left, b"x", CompressionMethod::Stored),
                        (right, b"y", CompressionMethod::Stored),
                    ],
                );
                assert!(
                    stage_input(DropPlan::Zip(path.clone())).is_err(),
                    "{left} + {right}"
                );
            }
        }

        #[test]
        fn rejects_duplicate_raw_records_and_links() {
            let fixture = Workspace::new().unwrap();
            let path = fixture.path().join("Widget.zip");
            write_zip(
                &path,
                &[
                    ("one.exe", b"a", CompressionMethod::Stored),
                    ("two.exe", b"b", CompressionMethod::Stored),
                ],
            );
            let mut bytes = fs::read(&path).unwrap();
            let occurrences: Vec<_> = bytes
                .windows(7)
                .enumerate()
                .filter_map(|(offset, window)| (window == b"two.exe").then_some(offset))
                .collect();
            assert_eq!(occurrences.len(), 2);
            for offset in occurrences {
                bytes[offset..offset + 7].copy_from_slice(b"one.exe");
            }
            fs::write(&path, &bytes).unwrap();
            assert!(stage_input(DropPlan::Zip(path.clone()))
                .unwrap_err()
                .message
                .contains("duplicate"));

            let mut writer = ZipWriter::new(File::create(&path).unwrap());
            writer
                .add_symlink("link", "target", SimpleFileOptions::default())
                .unwrap();
            writer.finish().unwrap();
            assert!(stage_input(DropPlan::Zip(path))
                .unwrap_err()
                .message
                .contains("link"));
        }

        #[test]
        fn synthesized_directories_count_toward_the_archive_limit() {
            let fixture = Workspace::new().unwrap();
            let path = fixture.path().join("Widget.zip");
            let mut writer = ZipWriter::new(File::create(&path).unwrap());
            for index in 0..=MAX_ENTRIES / super::super::super::MAX_DEPTH {
                let name = format!(
                    "{index}/{}item",
                    "dir/".repeat(super::super::super::MAX_DEPTH - 2)
                );
                writer
                    .start_file(
                        name,
                        SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
                    )
                    .unwrap();
                writer.write_all(b"x").unwrap();
            }
            writer.finish().unwrap();
            let failure = stage_input(DropPlan::Zip(path)).unwrap_err();
            assert!(
                failure.message.contains("20,000 file and folder limit"),
                "{failure}"
            );
        }

        #[cfg(windows)]
        #[test]
        fn explicit_cleanup_reports_a_locked_staged_file() {
            use std::os::windows::fs::OpenOptionsExt;

            let fixture = Workspace::new().unwrap();
            let path = fixture.path().join("Widget.zip");
            write_zip(
                &path,
                &[("Widget.exe", b"owned", CompressionMethod::Stored)],
            );
            let staged = stage_input(DropPlan::Zip(path)).unwrap();
            let work = staged._workspace.path().to_path_buf();
            let lock = File::options()
                .read(true)
                .share_mode(1)
                .open(staged.root().join("Widget.exe"))
                .unwrap();
            let failure = staged.cleanup().unwrap_err();
            drop(lock);
            let preserved = work.exists();
            fs::remove_dir_all(&work).unwrap();
            assert!(preserved);
            assert!(
                failure.message.contains(&work.display().to_string()),
                "{failure}"
            );
        }

        #[test]
        fn cancellation_stops_staging() {
            let fixture = Workspace::new().unwrap();
            let path = fixture.path().join("Widget.zip");
            write_zip(
                &path,
                &[("Widget.exe", &[42; 180_000], CompressionMethod::Deflated)],
            );
            assert_eq!(
                plan_drop_with_cancel(vec![path.clone()], || true)
                    .unwrap_err()
                    .message,
                "Input staging stopped."
            );
            let checks = Cell::new(0);
            let result = stage_input_with_cancel(DropPlan::Zip(path), || {
                let count = checks.get() + 1;
                checks.set(count);
                count >= 9
            });
            assert_eq!(result.unwrap_err().message, "Input staging stopped.");
            assert_eq!(checks.get(), 9);
        }
    }

    #[cfg(windows)]
    #[test]
    fn rejects_a_network_share_before_reading_it() {
        let error = classify_drop(Path::new(r"\\host\share\setup.exe")).unwrap_err();
        assert!(error.message.contains("Network and device paths"));
    }
}
