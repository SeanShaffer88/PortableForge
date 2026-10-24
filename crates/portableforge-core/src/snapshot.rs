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

/// Walk `root` on the machine running this code (the agent, inside the
/// sandbox) and record every regular file under it. `display_prefix`
/// replaces the walked root in recorded paths, so a snapshot taken over a
/// fixture tree can carry the Windows paths it stands for in tests, and
/// the sandbox agent can record true absolute paths.
pub fn scan_tree(
    root: &Path,
    display_prefix: &str,
    progress: &mut Progress,
) -> std::io::Result<Vec<FileEntry>> {
    let mut entries = Vec::new();
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for item in std::fs::read_dir(&dir)? {
            let item = item?;
            let path = item.path();
            let kind = item.file_type()?;
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
                let bytes = std::fs::read(&path)?;
                let meta = item.metadata()?;
                let mtime_secs = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                entries.push(FileEntry {
                    path: display,
                    size: bytes.len() as u64,
                    sha256: sha256_hex(&bytes),
                    mtime_secs,
                });
                progress.advance(1);
            }
        }
    }
    entries.sort_by(|a, b| {
        a.path
            .to_ascii_lowercase()
            .cmp(&b.path.to_ascii_lowercase())
    });
    Ok(entries)
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
    let mut snap = Snapshot::new(label);
    let mut by_path: std::collections::BTreeMap<String, FileEntry> =
        std::collections::BTreeMap::new();
    for (root, prefix) in roots {
        if !root.is_dir() {
            continue;
        }
        let mut progress = Progress::new("scan", None);
        for entry in scan_tree(root, prefix, &mut progress)? {
            by_path.insert(entry.path.to_ascii_lowercase(), entry);
        }
    }
    snap.files = by_path.into_values().collect();
    Ok(snap)
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
