//! Read-only storage clues and observations from isolated captures.

use std::collections::BTreeSet;
use std::path::Component;

use crate::{
    diff::{ChangeKind, Diff},
    folder::{
        validation::{raw_configs, read_raw_config, MAX_RAW_CONFIGS},
        FolderScan,
    },
    noise,
};

const MAX_DISPLAY_BYTES: usize = 256;
const MAX_OBSERVED_PER_PHASE: usize = 24;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum StorageSource {
    /// An explicit storage reference in a settings file.
    Referenced,
    /// A settings file present in the inventoried input.
    Included,
    /// A new or modified file or registry key during setup.
    Setup,
    /// A new or modified file or registry key during the captured first run.
    FirstRun,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageFinding {
    pub location: String,
    pub evidence: String,
    pub source: StorageSource,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StorageReport {
    pub findings: Vec<StorageFinding>,
    /// The folder inventory, settings reader, or report reached a limit.
    pub limited: bool,
}

const USER_LOCATIONS: &[(&str, &str)] = &[
    (r"AppData\Roaming", "Roaming AppData"),
    (r"AppData\Local", "Local AppData"),
    (r"AppData\LocalLow", "LocalLow AppData"),
    ("Documents", "Documents"),
    ("Saved Games", "Saved Games"),
];

/// Inspect a staged folder without executing any input or following a path from a config.
pub fn inspect_folder(scan: &FolderScan) -> StorageReport {
    let mut report = StorageReport {
        limited: !scan.complete,
        ..Default::default()
    };
    let mut referenced = BTreeSet::new();
    let mut included = Vec::new();
    for (index, entry) in raw_configs(scan).enumerate() {
        if index == MAX_RAW_CONFIGS {
            report.limited = true;
            break;
        }
        if !entry
            .relative_path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        {
            report.limited = true;
            continue;
        }
        let relative = entry.relative_path.to_string_lossy();
        let evidence = display_path(&relative);
        included.push(StorageFinding {
            location: evidence.clone(),
            evidence: String::new(),
            source: StorageSource::Included,
        });
        let Ok(Some(text)) = read_raw_config(scan, entry) else {
            report.limited = true;
            continue;
        };
        for location in reference_locations(&text) {
            if referenced.insert(location) {
                report.findings.push(StorageFinding {
                    location: location.into(),
                    evidence: evidence.clone(),
                    source: StorageSource::Referenced,
                });
            }
        }
    }
    report.findings.extend(included);
    report
}

/// Summarize tracked file and registry changes from setup and first-run snapshots.
/// This report only describes what changed between snapshots.
pub fn observe_capture(setup: &Diff, first_run: &Diff) -> StorageReport {
    let mut report = StorageReport::default();
    for (diff, source) in [
        (setup, StorageSource::Setup),
        (first_run, StorageSource::FirstRun),
    ] {
        let start = report.findings.len();
        let mut seen = BTreeSet::new();
        let files = diff
            .real_file_changes()
            .filter(|change| is_write(change.kind))
            .filter_map(|change| captured_file(&change.path));
        let keys = diff
            .real_reg_changes()
            .filter(|change| is_write(change.kind))
            .filter_map(|change| captured_registry(&change.key));
        for (location, evidence) in files.chain(keys) {
            if !seen.insert(location.to_lowercase()) {
                continue;
            }
            if report.findings.len() - start == MAX_OBSERVED_PER_PHASE {
                report.limited = true;
                break;
            }
            report.findings.push(StorageFinding {
                location: display_path(&location),
                evidence: display_path(&evidence),
                source,
            });
        }
    }
    report
}

fn is_write(kind: ChangeKind) -> bool {
    matches!(kind, ChangeKind::Added | ChangeKind::Modified)
}

fn referenced_path_start(line: &str, index: usize) -> bool {
    line[..index].chars().next_back().is_none_or(|previous| {
        previous.is_whitespace() || matches!(previous, '=' | ':' | '"' | '\'' | '(' | '[' | ',')
    })
}

fn reference_locations(text: &str) -> BTreeSet<&'static str> {
    let mut locations = BTreeSet::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with([';', '#']) {
            continue;
        }
        // JSON escapes path separators. Config references are clues only.
        let line = line.replace('/', "\\").replace(r"\\", "\\");
        let folded = line.to_ascii_lowercase();
        for (token, location) in [
            ("%appdata%", "Roaming AppData"),
            ("%localappdata%", "Local AppData"),
            ("%programdata%", "ProgramData"),
        ] {
            if folded.match_indices(token).any(|(index, _)| {
                referenced_path_start(&line, index)
                    && strip_location(&folded[index..], token).is_some()
            }) {
                locations.insert(location);
            }
        }
        for (index, char) in line.char_indices() {
            if !matches!(char, 'A'..='Z' | 'a'..='z' | '%') || !referenced_path_start(&line, index)
            {
                continue;
            }
            let tail = &line[index..];
            let root = if let Some(profile) = strip_case_prefix(tail, r"%USERPROFILE%\") {
                user_location(profile)
            } else {
                windows_location(tail)
            };
            if let Some((_, label, _)) = root {
                locations.insert(label);
            }
        }
    }
    locations
}

fn strip_case_prefix<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    path.get(..prefix.len())
        .filter(|begin| begin.eq_ignore_ascii_case(prefix))?;
    path.get(prefix.len()..)
}

fn strip_location<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    let suffix = strip_case_prefix(path, prefix)?;
    match suffix.as_bytes().first() {
        Some(b'\\') => suffix.get(1..),
        Some(b'"' | b'\'' | b',' | b';' | b')' | b']' | b'}') | None => Some(""),
        _ => None,
    }
}

fn user_location(path: &str) -> Option<(&'static str, &'static str, &str)> {
    USER_LOCATIONS.iter().find_map(|&(root, label)| {
        strip_location(path, root).map(|relative| (root, label, relative))
    })
}

fn windows_location(path: &str) -> Option<(&'static str, &'static str, &str)> {
    let [drive, b':', b'\\', ..] = path.as_bytes() else {
        return None;
    };
    if !drive.is_ascii_alphabetic() {
        return None;
    }
    let rest = &path[3..];
    if let Some(relative) = strip_location(rest, "ProgramData") {
        return Some(("ProgramData", "ProgramData", relative));
    }
    let (profile, relative) = strip_case_prefix(rest, r"Users\")?.split_once('\\')?;
    if profile.is_empty() {
        return None;
    }
    user_location(relative)
}

fn captured_file(path: &str) -> Option<(String, String)> {
    let path = path.replace('/', "\\");
    let (root, _, relative) = windows_location(&path)?;
    if relative
        .split('\\')
        .any(|part| matches!(part, "" | "." | ".."))
    {
        return None;
    }
    let (parent, file) = relative.rsplit_once('\\').unwrap_or(("", relative));
    let location = if parent.is_empty() {
        root.to_owned()
    } else {
        format!("{root}\\{parent}")
    };
    Some((location, file.to_owned()))
}

fn captured_registry(key: &str) -> Option<(String, String)> {
    let key = key.replace('/', "\\");
    if noise::is_uninstall_key(&key)
        || key
            .to_ascii_lowercase()
            .ends_with(r"\microsoft\windows\currentversion\uninstall")
    {
        return None;
    }
    let (hive, software_path) = key.split_once('\\')?;
    let hive = if ["HKEY_CURRENT_USER", "HKCU"]
        .iter()
        .any(|name| hive.eq_ignore_ascii_case(name))
    {
        "HKCU"
    } else if ["HKEY_LOCAL_MACHINE", "HKLM"]
        .iter()
        .any(|name| hive.eq_ignore_ascii_case(name))
    {
        "HKLM"
    } else {
        return None;
    };
    strip_case_prefix(software_path, r"Software\")?;
    Some((
        format!("{hive}\\{software_path}"),
        "Windows registry".into(),
    ))
}

/// Shorten paths and replace formatting controls for display only.
pub fn display_path(path: &str) -> String {
    let mut display = String::with_capacity(path.len().min(MAX_DISPLAY_BYTES));
    for char in path.chars() {
        let char = if char.is_control()
            || matches!(char, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        {
            '�'
        } else {
            char
        };
        if display.len() + char.len_utf8() > MAX_DISPLAY_BYTES - '…'.len_utf8() {
            display.push('…');
            break;
        }
        display.push(char);
    }
    display
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{FileChange, RegChange};
    use std::{fs, path::PathBuf, time::SystemTime};

    struct Folder(PathBuf);

    impl Folder {
        fn new() -> Self {
            let stamp = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap();
            let path = std::env::temp_dir().join(format!(
                "portableforge-storage-{}-{}",
                std::process::id(),
                stamp.as_nanos()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn file(path: impl Into<String>, kind: ChangeKind, noise: bool) -> FileChange {
        FileChange {
            path: path.into(),
            kind,
            before: None,
            after: None,
            noise,
        }
    }

    fn key(key: impl Into<String>, kind: ChangeKind, noise: bool) -> RegChange {
        RegChange {
            key: key.into(),
            name: "private value name".into(),
            kind,
            before: None,
            after: None,
            noise,
        }
    }

    #[test]
    fn archive_settings_show_clues_without_revealing_file_contents() {
        let folder = Folder::new();
        let content = r#"{
 "profile": "%APPDATA%\\private value",
 "cache": "D:\\Users\\PrivateAccount\\AppData\\Local\\Example",
 "save": "%USERPROFILE%\\Saved Games",
 "ignored": "D:\\Stuff\\AppData\\Roaming\\Unrelated"
}"#;
        fs::write(folder.0.join("settings.JSON"), content).unwrap();
        fs::write(folder.0.join("broken.ini"), [0xff]).unwrap();
        fs::write(folder.0.join("readme.txt"), "Data=%PROGRAMDATA%").unwrap();
        let report = inspect_folder(&crate::folder::scan(&folder.0).unwrap());
        assert!(report.limited);
        assert_eq!(
            report
                .findings
                .iter()
                .filter(|finding| finding.source == StorageSource::Referenced)
                .map(|finding| (finding.location.as_str(), finding.evidence.as_str()))
                .collect::<Vec<_>>(),
            [
                ("Local AppData", "settings.JSON"),
                ("Roaming AppData", "settings.JSON"),
                ("Saved Games", "settings.JSON"),
            ]
        );
        assert!(report.findings.iter().any(|finding| {
            finding.source == StorageSource::Included && finding.location == "settings.JSON"
        }));
        let evidence = format!("{report:?}");
        for hidden in ["PrivateAccount", "private value", "Unrelated", "readme.txt"] {
            assert!(!evidence.contains(hidden));
        }
    }

    #[test]
    fn standalone_storage_references_require_token_and_folder_boundaries() {
        let text = r#"
# %LOCALAPPDATA%
; %PROGRAMDATA%
root=not%APPDATA%\Widget
root=%APPDATA%Suffix
drive=C:\ProgramDataElsewhere\Widget
folder=%USERPROFILE%\DocumentsOther
path=D:/Users/known/AppData/LocalLow/Tool
documents="%USERPROFILE%\Documents"
shared=E:\ProgramData\Widget
"#;
        assert_eq!(
            reference_locations(text),
            BTreeSet::from(["Documents", "LocalLow AppData", "ProgramData"])
        );
    }

    #[test]
    fn capture_groups_added_or_modified_writes_by_phase_without_host_identity() {
        let setup = Diff {
            files: vec![
                file(
                    r"C:\Users\PrivateAccount\AppData\Roaming\Vendor\Tool\a.ini",
                    ChangeKind::Added,
                    false,
                ),
                file(
                    r"C:\Users\PrivateAccount\appdata\roaming\vendor\TOOL\b.ini",
                    ChangeKind::Modified,
                    false,
                ),
                file(
                    r"C:\Users\PrivateAccount\AppData\Roaming\Deleted\a.ini",
                    ChangeKind::Removed,
                    false,
                ),
                file(
                    r"C:\Users\PrivateAccount\AppData\Local\Noisy\a.ini",
                    ChangeKind::Added,
                    true,
                ),
                file(
                    r"C:\Programs\Documents\Install\a.ini",
                    ChangeKind::Added,
                    false,
                ),
            ],
            registry: vec![
                key(
                    r"HKEY_LOCAL_MACHINE\Software\Vendor\Tool",
                    ChangeKind::Modified,
                    false,
                ),
                key(r"HKLM\Software\Vendor\Tool", ChangeKind::Added, false),
                key(
                    r"HKLM\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\App",
                    ChangeKind::Added,
                    false,
                ),
                key(r"HKCU\Software\Noisy", ChangeKind::Modified, true),
            ],
            ..Default::default()
        };
        let first_run = Diff {
            files: vec![
                file(
                    r"C:\Users\PrivateAccount\AppData\Roaming\Vendor\Tool\prefs.ini",
                    ChangeKind::Added,
                    false,
                ),
                file(
                    r"C:\Users\PrivateAccount\Documents\data.bin",
                    ChangeKind::Added,
                    false,
                ),
            ],
            registry: vec![key(r"HKCU\Software\Vendor\Tool", ChangeKind::Added, false)],
            ..Default::default()
        };
        let report = observe_capture(&setup, &first_run);
        assert!(!report.limited);
        assert_eq!(
            report
                .findings
                .iter()
                .map(|finding| (finding.source, finding.location.as_str()))
                .collect::<Vec<_>>(),
            [
                (StorageSource::Setup, r"AppData\Roaming\Vendor\Tool"),
                (StorageSource::Setup, r"HKLM\Software\Vendor\Tool"),
                (StorageSource::FirstRun, r"AppData\Roaming\Vendor\Tool"),
                (StorageSource::FirstRun, "Documents"),
                (StorageSource::FirstRun, r"HKCU\Software\Vendor\Tool"),
            ]
        );
        let evidence = format!("{report:?}");
        for hidden in [
            "PrivateAccount",
            "private value name",
            "Noisy",
            "Deleted",
            "Uninstall",
        ] {
            assert!(!evidence.contains(hidden));
        }
    }

    #[test]
    fn each_capture_phase_is_bounded_and_hostile_labels_stay_single_line() {
        let setup = Diff {
            files: (0..=MAX_OBSERVED_PER_PHASE)
                .map(|i| {
                    file(
                        format!(r"C:\ProgramData\Tool{i}\value"),
                        ChangeKind::Added,
                        false,
                    )
                })
                .collect(),
            ..Default::default()
        };
        let first_run = Diff {
            registry: vec![key(
                format!("HKCU\\Software\\{}\u{202e}\n", "長".repeat(256)),
                ChangeKind::Added,
                false,
            )],
            ..Default::default()
        };
        let report = observe_capture(&setup, &first_run);
        assert!(report.limited);
        assert_eq!(report.findings.len(), MAX_OBSERVED_PER_PHASE + 1);
        let label = &report.findings.last().unwrap().location;
        assert!(label.len() <= MAX_DISPLAY_BYTES);
        assert!(label.ends_with('…'));
        assert_eq!(display_path("co\u{202e}de\n"), "co�de�");
    }
}
