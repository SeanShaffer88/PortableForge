//! Diffing two snapshots: what was added, removed, or changed.
//! Paths and registry identities compare case-insensitively (Windows
//! semantics); the recorded display form comes from the newer snapshot.

use serde::{Deserialize, Serialize};

use crate::noise;
use crate::snapshot::{FileEntry, RegEntry, Snapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeKind {
    Added,
    Removed,
    Modified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    pub kind: ChangeKind,
    pub path: String,
    pub before: Option<FileEntry>,
    pub after: Option<FileEntry>,
    /// True when this change matches a known-noise pattern. Noise stays
    /// in the diff, flagged — never silently dropped.
    pub noise: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegChange {
    pub kind: ChangeKind,
    pub key: String,
    pub name: String,
    pub before: Option<RegEntry>,
    pub after: Option<RegEntry>,
    pub noise: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diff {
    pub files: Vec<FileChange>,
    pub registry: Vec<RegChange>,
    /// Services present after but not before (and vice versa is not
    /// tracked — a removed service is not the new app's doing).
    pub services_added: Vec<String>,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.real_file_changes().count() == 0
            && self.real_reg_changes().count() == 0
            && self.services_added.is_empty()
    }

    pub fn real_file_changes(&self) -> impl Iterator<Item = &FileChange> {
        self.files.iter().filter(|c| !c.noise)
    }

    pub fn real_reg_changes(&self) -> impl Iterator<Item = &RegChange> {
        self.registry.iter().filter(|c| !c.noise)
    }
}

pub fn diff_snapshots(before: &Snapshot, after: &Snapshot) -> Diff {
    let b_files = before.file_map();
    let a_files = after.file_map();
    let mut files: Vec<FileChange> = Vec::new();

    let mut keys: Vec<String> = b_files.keys().chain(a_files.keys()).cloned().collect();
    keys.sort();
    keys.dedup();

    for key in keys {
        let b = b_files.get(&key).copied();
        let a = a_files.get(&key).copied();
        let change = match (b, a) {
            (None, Some(a)) => Some(FileChange {
                kind: ChangeKind::Added,
                path: a.path.clone(),
                before: None,
                after: Some(a.clone()),
                noise: noise::is_noise_path(&a.path),
            }),
            (Some(b), None) => Some(FileChange {
                kind: ChangeKind::Removed,
                path: b.path.clone(),
                before: Some(b.clone()),
                after: None,
                noise: noise::is_noise_path(&b.path),
            }),
            (Some(b), Some(a)) if b.sha256 != a.sha256 || b.size != a.size => Some(FileChange {
                kind: ChangeKind::Modified,
                path: a.path.clone(),
                before: Some(b.clone()),
                after: Some(a.clone()),
                noise: noise::is_noise_path(&a.path),
            }),
            _ => None,
        };
        if let Some(c) = change {
            files.push(c);
        }
    }

    let b_reg = before.reg_map();
    let a_reg = after.reg_map();
    let mut reg_keys: Vec<(String, String)> = b_reg.keys().chain(a_reg.keys()).cloned().collect();
    reg_keys.sort();
    reg_keys.dedup();
    let mut registry = Vec::new();
    for id in reg_keys {
        let b = b_reg.get(&id).copied();
        let a = a_reg.get(&id).copied();
        let change = match (b, a) {
            (None, Some(a)) => Some(RegChange {
                kind: ChangeKind::Added,
                key: a.key.clone(),
                name: a.name.clone(),
                before: None,
                after: Some(a.clone()),
                noise: noise::is_noise_reg(&a.key, &a.name),
            }),
            (Some(b), None) => Some(RegChange {
                kind: ChangeKind::Removed,
                key: b.key.clone(),
                name: b.name.clone(),
                before: Some(b.clone()),
                after: None,
                noise: noise::is_noise_reg(&b.key, &b.name),
            }),
            (Some(b), Some(a)) if b.data != a.data => Some(RegChange {
                kind: ChangeKind::Modified,
                key: a.key.clone(),
                name: a.name.clone(),
                before: Some(b.clone()),
                after: Some(a.clone()),
                noise: noise::is_noise_reg(&a.key, &a.name),
            }),
            _ => None,
        };
        if let Some(c) = change {
            registry.push(c);
        }
    }

    let services_added = after
        .services
        .iter()
        .filter(|s| !before.services.iter().any(|b| b.eq_ignore_ascii_case(s)))
        .cloned()
        .collect();

    Diff {
        files,
        registry,
        services_added,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::sha256_hex;

    fn fe(path: &str, content: &[u8]) -> FileEntry {
        FileEntry {
            path: path.into(),
            size: content.len() as u64,
            sha256: sha256_hex(content),
            mtime_secs: 0,
        }
    }

    fn re(key: &str, name: &str, data: &str) -> RegEntry {
        RegEntry {
            key: key.into(),
            name: name.into(),
            data: data.into(),
        }
    }

    #[test]
    fn diff_finds_added_modified_removed_and_noise() {
        let mut before = Snapshot::new("pre");
        before.files.push(fe(r"C:\App\keep.exe", b"v1"));
        before.files.push(fe(r"C:\App\gone.dll", b"x"));
        before
            .registry
            .push(re(r"HKEY_CURRENT_USER\Software\Acme", "Mode", "\"a\""));
        let mut after = Snapshot::new("post");
        after.files.push(fe(r"C:\App\keep.exe", b"v2"));
        after.files.push(fe(r"C:\App\new.dat", b"n"));
        after
            .files
            .push(fe(r"C:\Windows\Prefetch\A.EXE-1.pf", b"p"));
        after
            .registry
            .push(re(r"HKEY_CURRENT_USER\Software\Acme", "Mode", "\"b\""));
        after.services.push("AcmeSvc".into());

        let d = diff_snapshots(&before, &after);
        assert_eq!(d.files.len(), 4); // modified, removed, added, noise-added
        assert_eq!(d.real_file_changes().count(), 3);
        assert_eq!(d.registry.len(), 1);
        assert_eq!(d.registry[0].kind, ChangeKind::Modified);
        assert_eq!(d.services_added, vec!["AcmeSvc".to_string()]);
        assert!(!d.is_empty());
    }

    #[test]
    fn identical_snapshots_diff_empty() {
        let mut s = Snapshot::new("x");
        s.files.push(fe(r"C:\A\b.exe", b"same"));
        assert!(diff_snapshots(&s, &s).is_empty());
    }

    #[test]
    fn file_paths_compare_case_insensitively() {
        let mut before = Snapshot::new("pre");
        before.files.push(fe(r"c:\app\Main.EXE", b"same"));
        let mut after = Snapshot::new("post");
        after.files.push(fe(r"C:\App\main.exe", b"same"));
        assert!(diff_snapshots(&before, &after).files.is_empty());
    }
}
