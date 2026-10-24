//! Snapshots: a portable, JSON-serialisable inventory of files and
//! registry values at one point in time. The in-sandbox agent produces
//! snapshots; the host diffs them. Registry data arrives as parsed
//! `.reg` exports (the agent runs `reg.exe export` on Windows), so this
//! module needs no Windows APIs and is fully testable anywhere.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::progress::Progress;

/// One file in a snapshot. `path` is stored exactly as supplied by the
/// caller (the agent supplies sandbox-absolute Windows paths, already
/// normalised to `\` separators by [`normalise_path`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub size: u64,
    /// Lowercase hex SHA-256 of the file contents.
    pub sha256: String,
    /// Modification time, seconds since the Unix epoch (0 if unknown).
    pub mtime_secs: i64,
}

/// One registry value in a snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegEntry {
    /// Full key path, e.g. `HKEY_CURRENT_USER\Software\Acme\Widget`.
    pub key: String,
    /// Value name; `(Default)` for the unnamed value.
    pub name: String,
    /// Data rendered as text (the `.reg` textual form, normalised).
    pub data: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Human label: `pre`, `post-install`, `post-run`, `leak-test`, …
    pub label: String,
    pub files: Vec<FileEntry>,
    pub registry: Vec<RegEntry>,
    /// Installed services observed at snapshot time (names only).
    pub services: Vec<String>,
    /// Files the scan could not read and left out. Never hidden: a
    /// note with skips says so, and the count travels with it.
    #[serde(default)]
    pub skipped_files: usize,
    /// Plain warnings from the capture itself (e.g. a registry hive
    /// that could not be read), carried into the review.
    #[serde(default)]
    pub capture_warnings: Vec<String>,
}

impl Snapshot {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            ..Default::default()
        }
    }

    /// Keyed by lower-cased path (Windows paths compare case-insensitively).
    pub fn file_map(&self) -> BTreeMap<String, &FileEntry> {
        self.files
            .iter()
            .map(|f| (f.path.to_ascii_lowercase(), f))
            .collect()
    }

    /// Registry identity is (key, value name), case-insensitive on Windows.
    pub fn reg_map(&self) -> BTreeMap<(String, String), &RegEntry> {
        self.registry
            .iter()
            .map(|r| ((r.key.to_ascii_lowercase(), r.name.to_ascii_lowercase()), r))
            .collect()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("snapshot serialisation cannot fail")
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|e| format!("invalid snapshot JSON: {e}"))
    }
}

/// Normalise a Windows path for comparison: forward slashes become
/// backslashes, duplicate separators collapse, trailing separator removed,
/// drive letter upper-cased. Comparison of paths is case-insensitive;
/// callers lowercase when using paths as map keys.
pub fn normalise_path(p: &str) -> String {
    let mut out = String::with_capacity(p.len());
    let mut last_sep = false;
    for ch in p.chars() {
        let ch = if ch == '/' { '\\' } else { ch };
        if ch == '\\' {
            if last_sep {
                continue;
            }
            last_sep = true;
        } else {
            last_sep = false;
        }
        out.push(ch);
    }
    while out.len() > 3 && out.ends_with('\\') {
        out.pop();
    }
    // Upper-case a drive letter (`c:\…` -> `C:\…`).
    let mut chars: Vec<char> = out.chars().collect();
    if chars.len() >= 2 && chars[1] == ':' {
        chars[0] = chars[0].to_ascii_uppercase();
    }
    chars.into_iter().collect()
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(64);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// The outcome of one tree scan: the files recorded, and how many
/// were seen but could not be read (locked, permission-denied, gone
/// mid-scan). Skips are counted, never silently dropped — on a real
/// machine some files are always locked, and one locked file must
/// not fail the whole note.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanReport {
    pub files: Vec<FileEntry>,
    pub skipped: usize,
}

/// A remembered set of file fingerprints from an earlier scan. When a
/// file's size and modification time are unchanged, its hash is
/// reused instead of re-reading the file — the standard rsync-style
/// trade-off, and the reason the second and third notes of a capture
/// are much faster than the first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HashCache {
    by_path: BTreeMap<String, FileEntry>,
}

impl HashCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_snapshot(snap: &Snapshot) -> Self {
        let mut cache = Self::new();
        cache.absorb(snap.files.iter());
        cache
    }

    pub fn absorb<'a>(&mut self, entries: impl Iterator<Item = &'a FileEntry>) {
        for e in entries {
            self.by_path.insert(e.path.to_ascii_lowercase(), e.clone());
        }
    }

    /// The cached hash for `display_path` when size and mtime both
    /// match (and the mtime was actually known — 0 means unknown,
    /// which never trusts the cache).
    pub fn lookup(&self, display_path: &str, size: u64, mtime_secs: i64) -> Option<String> {
        if mtime_secs == 0 {
            return None;
        }
        let entry = self.by_path.get(&display_path.to_ascii_lowercase())?;
        if entry.size == size && entry.mtime_secs == mtime_secs {
            Some(entry.sha256.clone())
        } else {
            None
        }
    }

    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("hash cache serialisation cannot fail")
    }
}

/// Walk `root` on the machine running this code (the agent, inside the
/// sandbox) and record every regular file under it. `display_prefix`
/// replaces the walked root in recorded paths, so a snapshot taken over a
/// fixture tree can carry the Windows paths it stands for in tests, and
/// the sandbox agent can record true absolute paths.
///
/// A folder that cannot be listed at all is an error only at the top
/// level; inside the tree, unreadable folders and files are counted
/// as skips and the scan carries on.
pub fn scan_tree_report(
    root: &Path,
    display_prefix: &str,
    progress: &mut Progress,
    cache: Option<&HashCache>,
    on_file: &mut dyn FnMut(u64),
) -> std::io::Result<ScanReport> {
    let mut report = ScanReport::default();
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    let mut top = true;
    while let Some(dir) = stack.pop() {
        let listing = match std::fs::read_dir(&dir) {
            Ok(listing) => listing,
            Err(e) => {
                if top {
                    return Err(e);
                }
                report.skipped += 1;
                continue;
            }
        };
        top = false;
        for item in listing {
            let item = match item {
                Ok(item) => item,
                Err(_) => {
                    report.skipped += 1;
                    continue;
                }
            };
            let path = item.path();
            let kind = match item.file_type() {
                Ok(kind) => kind,
                Err(_) => {
                    report.skipped += 1;
                    continue;
                }
            };
            if kind.is_dir() {
                stack.push(path);
            } else if kind.is_file() {
                let rel = path.strip_prefix(root).unwrap_or(&path);
                let rel_text = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("\\");
                let display = if display_prefix.is_empty() {
                    rel_text.clone()
                } else {
                    format!("{}\\{}", normalise_path(display_prefix), rel_text)
                };
                let meta = match item.metadata() {
                    Ok(meta) => meta,
                    Err(_) => {
                        report.skipped += 1;
                        continue;
                    }
                };
                let size = meta.len();
                let mtime_secs = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                let sha256 = match cache.and_then(|c| c.lookup(&display, size, mtime_secs)) {
                    Some(sha) => sha,
                    None => match std::fs::read(&path) {
                        Ok(bytes) => sha256_hex(&bytes),
                        Err(_) => {
                            report.skipped += 1;
                            continue;
                        }
                    },
                };
                report.files.push(FileEntry {
                    path: display,
                    size,
                    sha256,
                    mtime_secs,
                });
                progress.advance(1);
                on_file(progress.done());
            }
        }
    }
    report.files.sort_by(|a, b| {
        a.path
            .to_ascii_lowercase()
            .cmp(&b.path.to_ascii_lowercase())
    });
    Ok(report)
}

/// [`scan_tree_report`] without a cache or progress hook, returning
/// just the files.
pub fn scan_tree(
    root: &Path,
    display_prefix: &str,
    progress: &mut Progress,
) -> std::io::Result<Vec<FileEntry>> {
    Ok(scan_tree_report(root, display_prefix, progress, None, &mut |_| {})?.files)
}

/// Scan several roots into one snapshot (the in-sandbox agent's system
/// capture: Program Files, ProgramData, the user profile, …). Each root
/// is `(folder to walk, display prefix to record)`. A root that does
/// not exist is skipped, not an error — a capture runs on machines
/// where some of these folders are absent. When two roots record the
/// same path, the later root wins.
pub fn scan_roots(
    roots: &[(PathBuf, String)],
    label: impl Into<String>,
) -> std::io::Result<Snapshot> {
    Ok(scan_roots_full(roots, label, None, &mut |_, _| {})?.0)
}

/// [`scan_roots`] with a hash cache and a live progress hook. The hook
/// is called with `(root being scanned, files recorded so far)` —
/// often enough for a heartbeat, and the returned cache holds every
/// fingerprint this scan produced, ready to persist for the next note.
/// Unreadable files across all roots are summed into the snapshot's
/// `skipped_files`.
pub fn scan_roots_full(
    roots: &[(PathBuf, String)],
    label: impl Into<String>,
    cache: Option<&HashCache>,
    on_progress: &mut dyn FnMut(&str, u64),
) -> std::io::Result<(Snapshot, HashCache)> {
    let mut snap = Snapshot::new(label);
    let mut by_path: std::collections::BTreeMap<String, FileEntry> =
        std::collections::BTreeMap::new();
    let mut recorded: u64 = 0;
    for (root, prefix) in roots {
        if !root.is_dir() {
            continue;
        }
        let mut progress = Progress::new("scan", None);
        let base = recorded;
        let report = scan_tree_report(root, prefix, &mut progress, cache, &mut |done| {
            on_progress(prefix, base + done);
        })?;
        recorded += report.files.len() as u64;
        snap.skipped_files += report.skipped;
        for entry in report.files {
            by_path.insert(entry.path.to_ascii_lowercase(), entry);
        }
        on_progress(prefix, recorded);
    }
    snap.files = by_path.into_values().collect();
    let fresh = HashCache::from_snapshot(&snap);
    Ok((snap, fresh))
}

/// Parse the service names out of `sc.exe query type= service state= all`
/// output. Driver services are left out — a packaging tool cares about
/// program services, and driver installs are already a separate
/// red-flag category.
/// The processor family a Windows program file was built for, read
/// from its PE header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeArch {
    X86,
    X64,
    Arm64,
    /// A PE file for some other machine type.
    Other(u16),
}

impl PeArch {
    pub fn label(&self) -> String {
        match self {
            PeArch::X86 => "x86".to_string(),
            PeArch::X64 => "x64".to_string(),
            PeArch::Arm64 => "ARM64".to_string(),
            PeArch::Other(m) => format!("other (machine {m:#06x})"),
        }
    }
}

/// Read the machine type from a PE file's header bytes. `None` when
/// the bytes are not a PE file at all (no `MZ`, no `PE\0\0` signature
/// where the header says, or too short to tell).
pub fn pe_arch(bytes: &[u8]) -> Option<PeArch> {
    if bytes.len() < 0x40 || &bytes[0..2] != b"MZ" {
        return None;
    }
    let pe_off = u32::from_le_bytes(bytes[0x3C..0x40].try_into().ok()?) as usize;
    if bytes.len() < pe_off + 6 || &bytes[pe_off..pe_off + 4] != b"PE\0\0" {
        return None;
    }
    let machine = u16::from_le_bytes(bytes[pe_off + 4..pe_off + 6].try_into().ok()?);
    Some(match machine {
        0x8664 => PeArch::X64,
        0x014C => PeArch::X86,
        0xAA64 => PeArch::Arm64,
        other => PeArch::Other(other),
    })
}

pub fn parse_service_names(sc_output: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut name: Option<String> = None;
    let mut is_driver = false;
    let flush = |name: &mut Option<String>, is_driver: bool, out: &mut Vec<String>| {
        if let Some(n) = name.take() {
            if !is_driver && !n.is_empty() {
                out.push(n);
            }
        }
    };
    for line in sc_output.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("SERVICE_NAME:") {
            flush(&mut name, is_driver, &mut out);
            name = Some(rest.trim().to_string());
            is_driver = false;
        } else if t.starts_with("TYPE") && t.contains("DRIVER") {
            is_driver = true;
        }
    }
    flush(&mut name, is_driver, &mut out);
    out.sort();
    out.dedup();
    out
}

/// A cheap fingerprint of a tree — `(relative path, size, mtime)` for
/// every file, without reading any contents. Two equal fingerprints
/// mean nothing changed, which is what the agent's `watch` waits for.
pub fn tree_fingerprint(root: &Path) -> Vec<(String, u64, i64)> {
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(listing) = std::fs::read_dir(&dir) else {
            continue;
        };
        for item in listing.flatten() {
            let path = item.path();
            let Ok(kind) = item.file_type() else { continue };
            if kind.is_dir() {
                stack.push(path);
            } else if kind.is_file() {
                let Ok(meta) = item.metadata() else { continue };
                let mtime_secs = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                let rel = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("\\");
                out.push((rel, meta.len(), mtime_secs));
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_roots_merges_existing_and_skips_missing() {
        let dir = std::env::temp_dir().join(format!("pf-roots-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a")).unwrap();
        std::fs::create_dir_all(dir.join("b")).unwrap();
        std::fs::write(dir.join("a").join("one.txt"), b"1").unwrap();
        std::fs::write(dir.join("b").join("two.txt"), b"2").unwrap();
        let snap = scan_roots(
            &[
                (dir.join("a"), r"C:\Program Files".into()),
                (dir.join("missing"), r"C:\Nope".into()),
                (dir.join("b"), r"C:\Users".into()),
            ],
            "pre",
        )
        .unwrap();
        assert_eq!(snap.files.len(), 2);
        assert_eq!(snap.label, "pre");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn normalise_path_unifies_separators_and_drive_case() {
        assert_eq!(
            normalise_path("c:/Program Files//Acme/"),
            "C:\\Program Files\\Acme"
        );
        assert_eq!(normalise_path("C:\\"), "C:\\");
    }

    #[test]
    fn sha256_matches_known_vector() {
        // SHA-256 of "abc" (FIPS 180-4 example).
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn scan_tree_records_hashes_and_display_paths() {
        let dir = std::env::temp_dir().join(format!("pf-snap-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), b"alpha").unwrap();
        std::fs::write(dir.join("sub").join("b.txt"), b"beta").unwrap();
        let mut p = Progress::new("scan", None);
        let files = scan_tree(&dir, r"C:\Program Files\Acme", &mut p).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, r"C:\Program Files\Acme\a.txt");
        assert_eq!(files[0].sha256, sha256_hex(b"alpha"));
        assert_eq!(files[1].path, r"C:\Program Files\Acme\sub\b.txt");
        assert_eq!(p.done(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pe_arch_reads_the_machine_type() {
        fn fake_pe(machine: u16) -> Vec<u8> {
            let mut bytes = vec![0u8; 0x100];
            bytes[0..2].copy_from_slice(b"MZ");
            bytes[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
            bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
            bytes[0x84..0x86].copy_from_slice(&machine.to_le_bytes());
            bytes
        }
        assert_eq!(pe_arch(&fake_pe(0x8664)), Some(PeArch::X64));
        assert_eq!(pe_arch(&fake_pe(0x014C)), Some(PeArch::X86));
        assert_eq!(pe_arch(&fake_pe(0xAA64)), Some(PeArch::Arm64));
        assert_eq!(pe_arch(b"MZfake"), None);
        assert_eq!(pe_arch(b"not a program"), None);
        assert_eq!(pe_arch(&[]), None);
    }

    #[test]
    fn service_names_parse_and_leave_drivers_out() {
        let text = "SERVICE_NAME: WidgetSvc\r\nDISPLAY_NAME: Widget Service\r\n        TYPE               : 10  WIN32_OWN_PROCESS\r\n        STATE              : 4  RUNNING\r\n\r\nSERVICE_NAME: SomeDriver\r\n        TYPE               : 1  KERNEL_DRIVER\r\n        STATE              : 4  RUNNING\r\n\r\nSERVICE_NAME: AardvarkSvc\r\n        TYPE               : 20  WIN32_SHARE_PROCESS\r\n";
        assert_eq!(
            parse_service_names(text),
            vec!["AardvarkSvc".to_string(), "WidgetSvc".to_string()]
        );
        assert!(parse_service_names("").is_empty());
    }

    #[test]
    fn hash_cache_reuses_unchanged_and_misses_changed() {
        let dir = std::env::temp_dir().join(format!("pf-cache-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), b"alpha").unwrap();
        let (snap, cache) = scan_roots_full(
            &[(dir.clone(), r"C:\X".into())],
            "one",
            None,
            &mut |_, _| {},
        )
        .unwrap();
        let sha_alpha = snap.files[0].sha256.clone();
        assert_eq!(sha_alpha, sha256_hex(b"alpha"));
        // Same size, same content: cache supplies the hash.
        let (snap2, _) = scan_roots_full(
            &[(dir.clone(), r"C:\X".into())],
            "two",
            Some(&cache),
            &mut |_, _| {},
        )
        .unwrap();
        assert_eq!(snap2.files[0].sha256, sha_alpha);
        // Different size: the cache must miss and the new hash appear.
        std::fs::write(dir.join("a.txt"), b"alpha-longer").unwrap();
        let (snap3, _) = scan_roots_full(
            &[(dir.clone(), r"C:\X".into())],
            "three",
            Some(&cache),
            &mut |_, _| {},
        )
        .unwrap();
        assert_eq!(snap3.files[0].sha256, sha256_hex(b"alpha-longer"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tree_fingerprint_notices_changes() {
        let dir = std::env::temp_dir().join(format!("pf-fp-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), b"1").unwrap();
        let before = tree_fingerprint(&dir);
        assert_eq!(before.len(), 1);
        assert_eq!(tree_fingerprint(&dir), before);
        std::fs::write(dir.join("b.txt"), b"22").unwrap();
        assert_ne!(tree_fingerprint(&dir), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_files_are_skipped_and_counted_not_fatal() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("pf-skip-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("fine.txt"), b"ok").unwrap();
        let locked = dir.join("locked.txt");
        std::fs::write(&locked, b"no").unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&locked).is_ok() {
            // Running as root: permissions do not block reads here, so
            // there is nothing to skip in this environment.
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let mut p = Progress::new("scan", None);
        let report = scan_tree_report(&dir, r"C:\X", &mut p, None, &mut |_| {}).unwrap();
        assert_eq!(report.files.len(), 1);
        assert_eq!(report.skipped, 1);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn snapshot_json_round_trips() {
        let mut s = Snapshot::new("pre");
        s.files.push(FileEntry {
            path: "C:\\A\\b.exe".into(),
            size: 3,
            sha256: sha256_hex(b"abc"),
            mtime_secs: 42,
        });
        s.registry.push(RegEntry {
            key: "HKEY_CURRENT_USER\\Software\\Acme".into(),
            name: "Mode".into(),
            data: "\"fast\"".into(),
        });
        let back = Snapshot::from_json(&s.to_json()).unwrap();
        assert_eq!(s, back);
    }
}
