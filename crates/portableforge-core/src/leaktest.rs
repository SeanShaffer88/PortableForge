//! Leak-test analysis (the "Testing the finished copy" step).
//!
//! The finished portable package runs in a *fresh* test box. A diff of
//! that box (before vs after the run) is analysed against one rule:
//! **a portable app may only change its own folder.** Everything else
//! it changed — files elsewhere, remembered settings, background
//! helpers — is a leak, reported in plain words with the fix that the
//! classification step would suggest for it.

use serde::{Deserialize, Serialize};

use crate::classify::{classify, Classification};
use crate::diff::{ChangeKind, Diff};
use crate::snapshot::Snapshot;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Leak {
    /// `file`, `settings`, or `helper`.
    pub kind: String,
    /// Where the change happened (path or key).
    pub where_: String,
    /// Plain-language explanation for the report.
    pub plain: String,
    /// The classification that suggests the fix, when one applies.
    pub suggested_fix: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeakReport {
    pub leaks: Vec<Leak>,
}

impl LeakReport {
    pub fn clean(&self) -> bool {
        self.leaks.is_empty()
    }

    /// One-line verdict for the wizard and the session log.
    pub fn verdict(&self) -> String {
        if self.clean() {
            "Perfect — the finished copy left the test computer exactly as it found it.".to_string()
        } else {
            format!(
                "The finished copy left {} thing{} behind. Each one below has a suggested fix.",
                self.leaks.len(),
                if self.leaks.len() == 1 { "" } else { "s" }
            )
        }
    }
}

fn under_portable_root(path: &str, portable_root: &str) -> bool {
    path.to_ascii_lowercase()
        .starts_with(&portable_root.to_ascii_lowercase())
}

/// Analyse the leak-test diff. `portable_root` is where the package was
/// installed inside the test box, e.g.
/// `C:\Users\WDAGUtilityAccount\Desktop\WidgetPortable`.
/// `before`/`after` snapshots of that same run let the classifier
/// phrase each leak's suggested fix.
pub fn analyse_leaks(
    portable_root: &str,
    before: &Snapshot,
    after: &Snapshot,
    run_diff: &Diff,
) -> LeakReport {
    // Suggested fixes, computed once from the whole run diff.
    let fixes = classify(&Diff::default(), run_diff, &[portable_root.to_string()]);
    let fix_for = |needle: &str| -> Option<String> {
        fixes.iter().find_map(|c: &Classification| {
            let text = format!("{:?}", c.finding);
            if text
                .to_ascii_lowercase()
                .contains(&needle.to_ascii_lowercase())
            {
                let (title, _) = crate::guide::plain_card(&c.finding);
                Some(title)
            } else {
                None
            }
        })
    };

    let mut leaks = Vec::new();
    for change in run_diff.real_file_changes() {
        if change.kind == ChangeKind::Removed {
            // Removing a pre-existing file is also a leak (of the
            // destructive kind) — report it like a change.
            if !under_portable_root(&change.path, portable_root) {
                leaks.push(Leak {
                    kind: "file".into(),
                    where_: change.path.clone(),
                    plain: format!("It deleted a file outside its own folder: {}", change.path),
                    suggested_fix: None,
                });
            }
            continue;
        }
        if !under_portable_root(&change.path, portable_root) {
            leaks.push(Leak {
                kind: "file".into(),
                where_: change.path.clone(),
                plain: format!("It left a file outside its own folder: {}", change.path),
                suggested_fix: fix_for(&change.path),
            });
        }
    }
    for change in run_diff.real_reg_changes() {
        // Any remembered-setting change is outside the portable folder
        // by definition — that is what "portable" must prevent.
        leaks.push(Leak {
            kind: "settings".into(),
            where_: format!("{}\\{}", change.key, change.name),
            plain: format!(
                "It wrote a remembered setting outside its own folder: {}",
                change.key
            ),
            suggested_fix: fix_for(&change.key),
        });
    }
    for svc in &run_diff.services_added {
        leaks.push(Leak {
            kind: "helper".into(),
            where_: svc.clone(),
            plain: format!("It started a background helper that keeps running: {svc}"),
            suggested_fix: None,
        });
    }
    let _ = (before, after); // snapshots kept in the API for session records
    LeakReport { leaks }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::FileChange;
    use crate::snapshot::{sha256_hex, FileEntry, RegEntry};

    fn added(path: &str) -> FileChange {
        FileChange {
            kind: ChangeKind::Added,
            path: path.into(),
            before: None,
            after: Some(FileEntry {
                path: path.into(),
                size: 1,
                sha256: sha256_hex(b"x"),
                mtime_secs: 0,
            }),
            noise: false,
        }
    }

    #[test]
    fn writes_inside_the_folder_are_clean_outside_are_leaks() {
        let root = r"C:\Box\WidgetPortable";
        let mut d = Diff::default();
        d.files
            .push(added(r"C:\Box\WidgetPortable\Data\settings.ini"));
        d.files
            .push(added(r"C:\Users\X\AppData\Roaming\Acme\settings.ini"));
        d.registry.push(crate::diff::RegChange {
            kind: ChangeKind::Added,
            key: r"HKEY_CURRENT_USER\Software\Acme".into(),
            name: "Mode".into(),
            before: None,
            after: Some(RegEntry {
                key: r"HKEY_CURRENT_USER\Software\Acme".into(),
                name: "Mode".into(),
                data: "\"1\"".into(),
            }),
            noise: false,
        });
        let report = analyse_leaks(root, &Snapshot::new("a"), &Snapshot::new("b"), &d);
        assert!(!report.clean());
        assert_eq!(report.leaks.len(), 2);
        assert!(report.verdict().contains("2 things"));
    }

    #[test]
    fn a_tidy_run_reports_perfect() {
        let mut d = Diff::default();
        d.files
            .push(added(r"C:\Box\WidgetPortable\Data\settings.ini"));
        let report = analyse_leaks(
            r"C:\Box\WidgetPortable",
            &Snapshot::new("a"),
            &Snapshot::new("b"),
            &d,
        );
        assert!(report.clean());
        assert!(report.verdict().starts_with("Perfect"));
    }
}
