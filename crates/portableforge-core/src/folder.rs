//! Read-only intake of an existing application folder.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, File, Metadata};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};

use crate::snapshot::{pe_arch, PeArch};

#[cfg(windows)]
mod windows;

mod validation;
pub use validation::validate;

const MAX_ENTRIES: usize = 20_000;
const MAX_DEPTH: usize = 16;
const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024 * 1024;
const MAX_EXECUTABLES: usize = 512;
const MAX_PE_OFFSET: u64 = 16 * 1024 * 1024;
const MAX_SMALL_READ: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EntryKind {
    File,
    Directory,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FolderEntry {
    pub relative_path: PathBuf,
    pub kind: EntryKind,
    pub size_bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IssueSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FolderIssue {
    pub code: &'static str,
    pub severity: IssueSeverity,
    pub relative_path: Option<PathBuf>,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExeCandidate {
    pub relative_path: PathBuf,
    pub architecture: PeArch,
    pub reasons: Vec<String>,
    pub score: i32,
}

#[derive(Clone, Debug)]
pub struct AppProperties {
    pub name: String,
    pub name_inferred: bool,
    pub publisher: Option<String>,
    pub version: Option<String>,
    #[cfg(windows)]
    pub icon: Option<crate::artwork::Raster>,
}

#[derive(Clone, Debug)]
pub struct FolderScan {
    pub root: PathBuf,
    pub entries: Vec<FolderEntry>,
    pub issues: Vec<FolderIssue>,
    pub complete: bool,
    pub candidates: Vec<ExeCandidate>,
    pub recommendation: Option<PathBuf>,
    pub properties: Option<AppProperties>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanError {
    pub message: String,
}

impl fmt::Display for ScanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ScanError {}

fn error(message: impl Into<String>) -> ScanError {
    ScanError {
        message: message.into(),
    }
}

impl FolderScan {
    /// Read a previously inventoried file through a fresh safety check.
    pub fn read_small(&self, relative: &Path, max_bytes: usize) -> Result<Vec<u8>, ScanError> {
        let max_bytes = max_bytes.min(MAX_SMALL_READ);
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.relative_path == relative && entry.kind == EntryKind::File)
            .ok_or_else(|| error("The file was not found in the scanned folder."))?;
        if entry.size_bytes > max_bytes as u64 {
            return Err(error("The selected file exceeds the read limit."));
        }
        let (_guard, file) = open_inventory_file(&self.root, relative)?;
        if file.metadata().map_err(io_error)?.len() != entry.size_bytes {
            return Err(error("The selected file changed after the scan."));
        }
        let mut bytes = Vec::new();
        file.take(max_bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(io_error)?;
        if bytes.len() as u64 != entry.size_bytes || bytes.len() > max_bytes {
            return Err(error(
                "The selected file changed or exceeds the read limit.",
            ));
        }
        Ok(bytes)
    }

    /// Inspect an explicitly selected candidate. Only PE resources are read.
    pub fn properties_for(&self, relative: &Path) -> Result<AppProperties, ScanError> {
        let candidate = self
            .candidates
            .iter()
            .find(|candidate| candidate.relative_path == relative)
            .ok_or_else(|| error("Choose an executable identified by the folder scan."))?;
        let (_guard, mut file) = open_inventory_file(&self.root, relative)?;
        let size = file.metadata().map_err(io_error)?.len();
        if executable_architecture(&mut file, size).map_err(io_error)?
            != Some(candidate.architecture)
        {
            return Err(error("The executable changed after the scan."));
        }
        let properties = AppProperties {
            name: fallback_name(&self.root, relative),
            name_inferred: true,
            publisher: None,
            version: None,
            #[cfg(windows)]
            icon: None,
        };
        #[cfg(windows)]
        let properties = {
            let mut properties = properties;
            windows::read_properties(&self.root.join(relative), &mut properties);
            properties
        };
        Ok(properties)
    }
}

/// Inspect a local folder without launching files or writing to it.
/// Paths that cannot be inspected safely are excluded and block a recommendation.
pub fn scan(root: &Path) -> Result<FolderScan, ScanError> {
    scan_with_limits(root, Limits::default())
}

#[derive(Clone, Copy)]
struct Limits {
    entries: usize,
    depth: usize,
    bytes: u64,
    executables: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            entries: MAX_ENTRIES,
            depth: MAX_DEPTH,
            bytes: MAX_TOTAL_BYTES,
            executables: MAX_EXECUTABLES,
        }
    }
}

struct Walker {
    scan: FolderScan,
    limits: Limits,
    seen: usize,
    bytes: u64,
    executables: usize,
    stopped: bool,
}

fn scan_with_limits(root: &Path, limits: Limits) -> Result<FolderScan, ScanError> {
    let root = std::path::absolute(root).map_err(io_error)?;
    let _root_guard = hold_directory_chain(&root)?;
    let mut walker = Walker {
        scan: FolderScan {
            root: root.clone(),
            entries: Vec::new(),
            issues: Vec::new(),
            complete: true,
            candidates: Vec::new(),
            recommendation: None,
            properties: None,
        },
        limits,
        seen: 0,
        bytes: 0,
        executables: 0,
        stopped: false,
    };
    walker.walk(&root, Path::new(""), 0);
    walker.finish();
    Ok(walker.scan)
}

impl Walker {
    fn issue(
        &mut self,
        code: &'static str,
        severity: IssueSeverity,
        path: &Path,
        message: impl Into<String>,
    ) {
        if severity == IssueSeverity::Error {
            self.scan.complete = false;
        }
        self.scan.issues.push(FolderIssue {
            code,
            severity,
            relative_path: (!path.as_os_str().is_empty()).then(|| path.to_path_buf()),
            message: message.into(),
        });
    }

    fn limit(&mut self, code: &'static str, path: &Path, message: &'static str) {
        self.issue(code, IssueSeverity::Error, path, message);
        self.stopped = true;
    }

    fn walk(&mut self, directory: &Path, relative: &Path, depth: usize) {
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(cause) => {
                self.issue(
                    "unreadable_folder",
                    IssueSeverity::Error,
                    relative,
                    format!("Could not read this folder: {cause}"),
                );
                return;
            }
        };
        let remaining = self.limits.entries.saturating_sub(self.seen);
        let entries = match entries.take(remaining + 1).collect::<io::Result<Vec<_>>>() {
            Ok(entries) if entries.len() <= remaining => entries,
            Ok(_) => {
                self.limit(
                    "entry_limit",
                    relative,
                    "This folder exceeds the scan's file and folder limit.",
                );
                return;
            }
            Err(cause) => {
                self.issue(
                    "unreadable_entry",
                    IssueSeverity::Error,
                    relative,
                    format!("Could not list this folder: {cause}"),
                );
                return;
            }
        };
        self.seen += entries.len();
        if depth >= self.limits.depth && !entries.is_empty() {
            self.issue(
                "depth_limit",
                IssueSeverity::Error,
                relative,
                "This folder exceeds the scan's nesting limit.",
            );
            return;
        }

        let mut groups: BTreeMap<String, Vec<(PathBuf, PathBuf)>> = BTreeMap::new();
        let mut unsafe_names = Vec::new();
        for entry in entries {
            let name = entry.file_name();
            let path = relative.join(&name);
            if let Some(name) = name.to_str().filter(|name| safe_name(name)) {
                groups
                    .entry(name.to_uppercase())
                    .or_default()
                    .push((entry.path(), path));
            } else {
                unsafe_names.push(path);
            }
        }
        unsafe_names.sort();
        for path in unsafe_names {
            self.issue(
                "unsafe_name",
                IssueSeverity::Error,
                &path,
                "This name cannot be used safely in a Windows package.",
            );
        }
        for mut rows in groups.into_values() {
            if self.stopped {
                break;
            }
            if rows.len() != 1 {
                rows.sort_by(|left, right| left.1.cmp(&right.1));
                for (_, path) in rows {
                    self.issue(
                        "case_collision",
                        IssueSeverity::Error,
                        &path,
                        "Another entry in this folder has the same Windows name.",
                    );
                }
                continue;
            }
            let (path, relative) = rows.pop().expect("one directory entry");
            self.visit(&path, &relative, depth);
        }
    }

    fn visit(&mut self, path: &Path, relative: &Path, depth: usize) {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) if !is_link(&metadata) => metadata,
            Ok(_) => {
                self.issue(
                    "reparse_point",
                    IssueSeverity::Error,
                    relative,
                    "Links and redirected paths are excluded.",
                );
                return;
            }
            Err(cause) => {
                self.issue(
                    "unreadable_entry",
                    IssueSeverity::Error,
                    relative,
                    format!("Could not inspect this entry: {cause}"),
                );
                return;
            }
        };
        if metadata.is_dir() {
            let _guard = match hold_directory(path) {
                Ok(guard) => guard,
                Err(cause) => {
                    self.issue(
                        "unreadable_folder",
                        IssueSeverity::Error,
                        relative,
                        cause.message,
                    );
                    return;
                }
            };
            self.scan.entries.push(FolderEntry {
                relative_path: relative.into(),
                kind: EntryKind::Directory,
                size_bytes: 0,
            });
            self.walk(path, relative, depth + 1);
        } else if metadata.is_file() {
            let size_bytes = metadata.len();
            if size_bytes > self.limits.bytes.saturating_sub(self.bytes) {
                self.limit(
                    "size_limit",
                    relative,
                    "This folder exceeds the scan's total file size limit.",
                );
                return;
            }
            self.bytes += size_bytes;
            self.scan.entries.push(FolderEntry {
                relative_path: relative.into(),
                kind: EntryKind::File,
                size_bytes,
            });
            if relative
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
            {
                self.examine(path, relative, size_bytes);
            }
        } else {
            self.issue(
                "non_regular_file",
                IssueSeverity::Error,
                relative,
                "This entry is not an ordinary file or folder.",
            );
        }
    }

    fn examine(&mut self, path: &Path, relative: &Path, size: u64) {
        if self.executables >= self.limits.executables {
            self.limit(
                "executable_limit",
                relative,
                "This folder exceeds the scan's executable inspection limit.",
            );
            return;
        }
        self.executables += 1;
        let inspected = open_regular(path).and_then(|mut file| {
            if file.metadata().map_err(io_error)?.len() != size {
                return Err(error("This file changed while the folder was scanned."));
            }
            executable_architecture(&mut file, size).map_err(io_error)
        });
        match inspected {
            Ok(Some(architecture)) => {
                self.scan
                    .candidates
                    .push(rank(&self.scan.root, relative, architecture));
            }
            Ok(None) => {
                self.issue(
                    "invalid_executable",
                    IssueSeverity::Warning,
                    relative,
                    "This .exe file has no valid Windows executable header or is a renamed DLL.",
                );
            }
            Err(cause) => {
                self.issue(
                    "unreadable_executable",
                    IssueSeverity::Error,
                    relative,
                    cause.message,
                );
            }
        }
    }

    fn finish(&mut self) {
        self.scan
            .entries
            .sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        self.scan.candidates.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then(left.relative_path.cmp(&right.relative_path))
        });
        let Some(first) = self.scan.candidates.first() else {
            self.issue(
                "no_executable",
                IssueSeverity::Warning,
                Path::new(""),
                "No valid Windows application executable was found.",
            );
            return;
        };
        let path = first.relative_path.clone();
        let clear = first.score >= 0
            && self
                .scan
                .candidates
                .get(1)
                .is_none_or(|next| first.score - next.score >= 3);
        if self.scan.complete && clear {
            self.scan.recommendation = Some(path.clone());
            match self.scan.properties_for(&path) {
                Ok(properties) => {
                    if properties.name_inferred {
                        self.issue(
                            "name_inferred",
                            IssueSeverity::Info,
                            &path,
                            "The suggested app name comes from the folder or executable name.",
                        );
                    }
                    if properties.publisher.is_none() {
                        self.issue(
                            "publisher_unknown",
                            IssueSeverity::Info,
                            &path,
                            "The application did not provide a publisher.",
                        );
                    }
                    if properties.version.is_none() {
                        self.issue(
                            "version_unknown",
                            IssueSeverity::Info,
                            &path,
                            "The application did not provide a version.",
                        );
                    }
                    #[cfg(windows)]
                    if properties.icon.is_none() {
                        self.issue(
                            "icon_unavailable",
                            IssueSeverity::Info,
                            &path,
                            "No readable application icon was found.",
                        );
                    }
                    self.scan.properties = Some(properties);
                }
                Err(cause) => {
                    self.scan.recommendation = None;
                    self.issue(
                        "properties_changed",
                        IssueSeverity::Error,
                        &path,
                        cause.message,
                    );
                }
            }
        } else if self.scan.complete {
            self.issue(
                "choose_executable",
                IssueSeverity::Info,
                Path::new(""),
                "Choose the application's main executable.",
            );
        }
    }
}

fn rank(root: &Path, relative: &Path, architecture: PeArch) -> ExeCandidate {
    let stem = relative
        .file_stem()
        .and_then(|part| part.to_str())
        .unwrap_or_default();
    let folder = root
        .file_name()
        .and_then(|part| part.to_str())
        .unwrap_or_default();
    let normalized = |value: &str| {
        value
            .chars()
            .filter(|ch| ch.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    let stem_key = normalized(stem);
    let folder_key = normalized(folder);
    let name_match = !stem_key.is_empty()
        && (stem_key == folder_key
            || folder_key.strip_prefix(&stem_key).is_some_and(|tail| {
                tail.starts_with(|ch: char| ch.is_ascii_digit()) || tail == "portable"
            }));
    let mut score = 0;
    let mut reasons = Vec::new();
    if relative.components().count() == 1 {
        score += 4;
        reasons.push("In the selected folder".into());
    }
    if name_match {
        score += 8;
        reasons.push("Name matches the selected folder".into());
    }
    if [
        "setup",
        "install",
        "uninstall",
        "update",
        "updater",
        "helper",
        "crash",
        "reporter",
        "service",
        "redist",
    ]
    .iter()
    .any(|word| stem_key.starts_with(word) || stem_key.ends_with(word))
    {
        score -= 20;
        reasons.push("Name resembles a helper, installer, or updater".into());
    }
    if relative.parent().is_some_and(|parent| {
        parent.components().any(|part| {
            matches!(
                normalized(&part.as_os_str().to_string_lossy()).as_str(),
                "tools" | "redist" | "support" | "helpers"
            )
        })
    }) {
        score -= 12;
        reasons.push("Located in a support folder".into());
    }
    if reasons.is_empty() {
        reasons.push("Valid Windows executable".into());
    }
    ExeCandidate {
        relative_path: relative.into(),
        architecture,
        reasons,
        score,
    }
}

fn fallback_name(root: &Path, executable: &Path) -> String {
    let label = |path: &Path| path.to_string_lossy().into_owned();
    root.file_name()
        .map(Path::new)
        .map(label)
        .or_else(|| executable.file_stem().map(Path::new).map(label))
        .unwrap_or_else(|| "Application".into())
}

fn safe_name(name: &str) -> bool {
    if name.is_empty()
        || name.ends_with(['.', ' '])
        || name.chars().any(|ch| {
            ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
        })
    {
        return false;
    }
    let first = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_ascii_uppercase();
    !matches!(
        first.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) && !["COM", "LPT"].iter().any(|prefix| {
        first.strip_prefix(prefix).is_some_and(|suffix| {
            matches!(
                suffix,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    })
}

fn executable_architecture(
    reader: &mut (impl Read + Seek),
    file_len: u64,
) -> io::Result<Option<PeArch>> {
    let mut dos = [0; 64];
    if file_len < dos.len() as u64 {
        return Ok(None);
    }
    reader.seek(SeekFrom::Start(0))?;
    reader.read_exact(&mut dos)?;
    if !dos.starts_with(b"MZ") {
        return Ok(None);
    }
    let offset = u64::from(u32::from_le_bytes(
        dos[60..64].try_into().expect("DOS offset"),
    ));
    if !(64..=MAX_PE_OFFSET).contains(&offset) || offset + 24 > file_len {
        return Ok(None);
    }
    let mut coff = [0; 24];
    reader.seek(SeekFrom::Start(offset))?;
    reader.read_exact(&mut coff)?;
    if !coff.starts_with(b"PE\0\0") {
        return Ok(None);
    }
    let machine = u16::from_le_bytes([coff[4], coff[5]]);
    let sections = u16::from_le_bytes([coff[6], coff[7]]);
    let optional_bytes = u16::from_le_bytes([coff[20], coff[21]]);
    let flags = u16::from_le_bytes([coff[22], coff[23]]);
    if machine == 0
        || flags & 0x2002 != 0x0002
        || !(1..=96).contains(&sections)
        || !(96..=4096).contains(&optional_bytes)
        || offset + 24 + u64::from(optional_bytes) + 40 * u64::from(sections) > file_len
    {
        return Ok(None);
    }
    let mut optional = [0; 70];
    reader.read_exact(&mut optional)?;
    let magic = u16::from_le_bytes([optional[0], optional[1]]);
    let subsystem = u16::from_le_bytes([optional[68], optional[69]]);
    if !matches!(magic, 0x10b | 0x20b)
        || !matches!(subsystem, 2 | 3)
        || (magic == 0x20b && optional_bytes < 112)
        || (machine == 0x014c && magic != 0x10b)
        || (matches!(machine, 0x8664 | 0xaa64) && magic != 0x20b)
    {
        return Ok(None);
    }
    // Reuse the shared machine mapping with a compact validated PE header.
    dos[60..64].copy_from_slice(&64u32.to_le_bytes());
    let mut header = [0; 88];
    header[..64].copy_from_slice(&dos);
    header[64..].copy_from_slice(&coff);
    Ok(pe_arch(&header))
}

fn io_error(cause: io::Error) -> ScanError {
    error(format!(
        "Could not inspect the selected folder or file: {cause}"
    ))
}

fn open_inventory_file(
    root: &Path,
    relative: &Path,
) -> Result<(Vec<DirectoryGuard>, File), ScanError> {
    if relative.as_os_str().is_empty()
        || relative.components().any(
            |part| !matches!(part, Component::Normal(name) if name.to_str().is_some_and(safe_name)),
        )
    {
        return Err(error("Choose a safe file path inside the scanned folder."));
    }
    let path = root.join(relative);
    let parent = path
        .parent()
        .ok_or_else(|| error("The selected file has no folder."))?;
    let guard = hold_directory_chain(parent)?;
    let file = open_regular(&path)?;
    Ok((guard, file))
}

fn hold_directory_chain(path: &Path) -> Result<Vec<DirectoryGuard>, ScanError> {
    if !path.is_absolute() {
        return Err(error("Choose a local folder with a full path."));
    }
    #[cfg(windows)]
    if !windows::local_drive(path) {
        return Err(error(
            "Choose a local drive folder. Network and device paths are excluded.",
        ));
    }
    let mut chain = Vec::new();
    let mut current = PathBuf::new();
    for part in path.components() {
        match part {
            Component::Prefix(_) => current.push(part.as_os_str()),
            Component::RootDir => {
                current.push(part.as_os_str());
                chain.push(hold_directory(&current)?);
            }
            Component::Normal(name) if name.to_str().is_some_and(safe_name) => {
                current.push(name);
                chain.push(hold_directory(&current)?);
            }
            _ => return Err(error("The selected folder path contains an unsafe name.")),
        }
    }
    Ok(chain)
}

fn is_link(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0 || metadata.file_type().is_symlink()
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

#[cfg(windows)]
type DirectoryGuard = File;
#[cfg(not(windows))]
type DirectoryGuard = ();

fn hold_directory(path: &Path) -> Result<DirectoryGuard, ScanError> {
    let metadata = fs::symlink_metadata(path).map_err(io_error)?;
    if !metadata.is_dir() || is_link(&metadata) {
        return Err(error(
            "Choose an ordinary local folder. Links and redirected paths are excluded.",
        ));
    }
    #[cfg(windows)]
    {
        windows::open_directory(path)
    }
    #[cfg(not(windows))]
    {
        Ok(())
    }
}

fn open_regular(path: &Path) -> Result<File, ScanError> {
    let metadata = fs::symlink_metadata(path).map_err(io_error)?;
    if !metadata.is_file() || is_link(&metadata) {
        return Err(error("The selected entry is not an ordinary local file."));
    }
    #[cfg(windows)]
    let file = windows::open_file(path)?;
    #[cfg(not(windows))]
    let file = File::open(path).map_err(io_error)?;
    let metadata = file.metadata().map_err(io_error)?;
    if !metadata.is_file() || is_link(&metadata) {
        return Err(error("The selected entry is not an ordinary local file."));
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static FIXTURE_ID: AtomicUsize = AtomicUsize::new(0);

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let mut fixture = std::env::temp_dir();
            fixture.push(format!(
                "pf-folder-scan-{}-{}",
                std::process::id(),
                FIXTURE_ID.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&fixture).unwrap();
            Self(fixture)
        }

        fn app(&self) -> PathBuf {
            let app = self.0.join("Widget");
            fs::create_dir(&app).unwrap();
            app
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn pe(machine: u16, flags: u16, subsystem: u16, offset: usize) -> Vec<u8> {
        let optional_bytes = if machine == 0x014c { 224u16 } else { 240u16 };
        let optional = offset + 24;
        let mut bytes = vec![0; optional + usize::from(optional_bytes) + 40];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[60..64].copy_from_slice(&(offset as u32).to_le_bytes());
        bytes[offset..offset + 4].copy_from_slice(b"PE\0\0");
        bytes[offset + 4..offset + 6].copy_from_slice(&machine.to_le_bytes());
        bytes[offset + 6..offset + 8].copy_from_slice(&1u16.to_le_bytes());
        bytes[offset + 20..offset + 22].copy_from_slice(&optional_bytes.to_le_bytes());
        bytes[offset + 22..offset + 24].copy_from_slice(&flags.to_le_bytes());
        let magic = if machine == 0x014c {
            0x10bu16
        } else {
            0x20bu16
        };
        bytes[optional..optional + 2].copy_from_slice(&magic.to_le_bytes());
        bytes[optional + 68..optional + 70].copy_from_slice(&subsystem.to_le_bytes());
        bytes
    }

    fn arch(bytes: &[u8]) -> Option<PeArch> {
        executable_architecture(&mut Cursor::new(bytes), bytes.len() as u64).unwrap()
    }

    #[test]
    fn pe_inspection_checks_offset_dll_flag_and_desktop_subsystem() {
        let valid = pe(0x8664, 0x0002, 2, 131_072);
        assert_eq!(arch(&valid), Some(PeArch::X64));
        assert_eq!(arch(&pe(0x014c, 0x0002, 3, 64)), Some(PeArch::X86));
        assert_eq!(arch(&pe(0xaa64, 0x0002, 2, 64)), Some(PeArch::Arm64));
        assert_eq!(arch(&pe(0x8664, 0x2002, 2, 64)), None);
        assert_eq!(arch(&pe(0x8664, 0x0002, 1, 64)), None);
        assert_eq!(arch(&pe(0x8664, 0x0002, 10, 64)), None);
        assert_eq!(arch(&valid[..100]), None);
        let mut bad = valid;
        bad[60..64].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(arch(&bad), None);
    }

    #[test]
    fn obvious_main_executable_beats_helpers_and_preserves_relative_paths() {
        let fixture = Fixture::new();
        let root = fixture.app();
        let support = root.join("Support");
        fs::create_dir(&support).unwrap();
        fs::write(root.join("Widget.exe"), pe(0x8664, 2, 2, 64)).unwrap();
        fs::write(support.join("Updater.exe"), pe(0x014c, 2, 3, 64)).unwrap();
        fs::write(root.join("Fake.exe"), b"not a program").unwrap();
        fs::write(root.join("settings.ini"), b"abc").unwrap();
        let result = scan(&root).unwrap();
        assert!(result.complete);
        assert_eq!(
            result.recommendation.as_deref(),
            Some(Path::new("Widget.exe"))
        );
        assert_eq!(result.candidates.len(), 2);
        assert_eq!(
            result.candidates[1].relative_path,
            Path::new("Support").join("Updater.exe")
        );
        assert!(result
            .issues
            .iter()
            .any(|issue| issue.code == "invalid_executable"));
        assert_eq!(
            result.read_small(Path::new("settings.ini"), 3).unwrap(),
            b"abc"
        );
        assert!(result.read_small(Path::new("settings.ini"), 2).is_err());
        assert!(result.read_small(Path::new("../settings.ini"), 3).is_err());
        assert_eq!(result.properties.unwrap().name, "Widget");
    }

    #[test]
    fn similar_candidates_require_a_choice() {
        let fixture = Fixture::new();
        let root = fixture.app();
        for name in ["Alpha.exe", "Beta.exe"] {
            fs::write(root.join(name), pe(0x8664, 2, 2, 64)).unwrap();
        }
        let result = scan(&root).unwrap();
        assert_eq!(result.candidates.len(), 2);
        assert!(result.recommendation.is_none());
        assert!(result.properties.is_none());
    }

    #[test]
    fn entries_and_size_limits_stop_without_a_recommendation() {
        let fixture = Fixture::new();
        let root = fixture.app();
        fs::write(root.join("Widget.exe"), pe(0x8664, 2, 2, 64)).unwrap();
        fs::write(root.join("Version.txt"), b"data").unwrap();
        let entries = scan_with_limits(
            &root,
            Limits {
                entries: 1,
                ..Limits::default()
            },
        )
        .unwrap();
        assert!(!entries.complete);
        assert!(entries.entries.is_empty());
        assert!(entries
            .issues
            .iter()
            .any(|issue| issue.code == "entry_limit"));
        let size = scan_with_limits(
            &root,
            Limits {
                bytes: 3,
                ..Limits::default()
            },
        )
        .unwrap();
        assert!(!size.complete);
        assert!(size.recommendation.is_none());
        assert!(size.issues.iter().any(|issue| issue.code == "size_limit"));
    }

    #[cfg(any(windows, unix))]
    #[test]
    fn directory_links_are_never_walked() {
        let fixture = Fixture::new();
        let root = fixture.app();
        let target = fixture.0.join("Elsewhere");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("Outside.exe"), pe(0x8664, 2, 2, 64)).unwrap();
        let link = root.join("Linked");
        #[cfg(windows)]
        match std::os::windows::fs::symlink_dir(&target, &link) {
            Ok(()) => {}
            Err(cause) if matches!(cause.raw_os_error(), Some(5 | 1314)) => return,
            Err(cause) => panic!("could not create test link: {cause}"),
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(scan(&link).is_err());
        let scan = scan(&root).unwrap();
        assert!(scan.entries.is_empty());
        assert!(!scan.complete);
        assert!(scan
            .issues
            .iter()
            .any(|issue| issue.code == "reparse_point"));
        #[cfg(windows)]
        fs::remove_dir(&link).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn directory_guard_prevents_a_rename_until_released() {
        let fixture = Fixture::new();
        let original = fixture.app();
        let moved = fixture.0.join("Renamed");
        let guard = hold_directory(&original).unwrap();
        assert!(fs::rename(&original, &moved).is_err());
        assert!(original.is_dir());
        drop(guard);
        fs::rename(&original, &moved).unwrap();
        assert!(moved.is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn collisions_and_unsafe_windows_names_are_excluded() {
        let fixture = Fixture::new();
        let root = fixture.app();
        for name in ["Alias.exe", "alias.EXE", "COM¹.txt", "bad:name"] {
            fs::write(root.join(name), b"unused").unwrap();
        }
        let result = scan(&root).unwrap();
        assert!(result.entries.is_empty());
        assert!(!result.complete);
        assert_eq!(
            result
                .issues
                .iter()
                .filter(|issue| issue.code == "case_collision")
                .count(),
            2
        );
        assert_eq!(
            result
                .issues
                .iter()
                .filter(|issue| issue.code == "unsafe_name")
                .count(),
            2
        );
    }
}
