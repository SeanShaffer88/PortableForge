//! Mode C ("Careful mode") rollback planning.
//!
//! Mode C runs on the real computer, watched and put back — it is NOT
//! sealed off, and PortableForge says so wherever it appears. Putting
//! things back is driven by this plan, generated from the run's diff:
//! an ordered, inspectable list of undo actions. Safety rules:
//! - only changes the run itself made are undone;
//! - deletions are limited to places an install plausibly writes
//!   (the install folder, per-user app folders) — anything else is
//!   listed as "check by hand", never auto-deleted;
//! - services are never auto-removed; they are listed for the user
//!   with the reason.

use serde::{Deserialize, Serialize};

use crate::diff::{ChangeKind, Diff};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RollbackAction {
    /// Delete a file the run added.
    DeleteFile { path: String },
    /// Restore a changed file's contents are NOT recoverable from a
    /// diff alone — flagged honestly instead of pretending.
    FileWasChanged { path: String },
    /// Delete a remembered setting the run added.
    DeleteSetting { key: String, name: String },
    /// A changed setting whose old value was recorded — restore it.
    RestoreSetting {
        key: String,
        name: String,
        old_data: String,
    },
    /// Needs a person: never automated.
    CheckByHand { what: String, why: String },
}

impl RollbackAction {
    pub fn plain(&self) -> String {
        match self {
            RollbackAction::DeleteFile { path } => format!("Delete the file it added: {path}"),
            RollbackAction::FileWasChanged { path } => {
                format!("A file it changed (its old contents were not saved): {path}")
            }
            RollbackAction::DeleteSetting { key, name } => {
                format!("Remove the setting it added: {key} — {name}")
            }
            RollbackAction::RestoreSetting { key, name, .. } => {
                format!("Put back the setting it changed: {key} — {name}")
            }
            RollbackAction::CheckByHand { what, why } => format!("Check by hand: {what} ({why})"),
        }
    }
}

fn safe_to_delete(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.contains(r"\program files\")
        || p.contains(r"\program files (x86)\")
        || p.contains(r"\appdata\")
        || p.contains(r"\programdata\")
        || p.contains(r"\users\public\")
}

pub fn rollback_plan(run_diff: &Diff) -> Vec<RollbackAction> {
    let mut actions = Vec::new();
    // Files first (deepest paths first, so folders empty out bottom-up).
    let mut file_changes: Vec<_> = run_diff.real_file_changes().collect();
    file_changes.sort_by_key(|c| std::cmp::Reverse(c.path.len()));
    for change in file_changes {
        match change.kind {
            ChangeKind::Added => {
                if safe_to_delete(&change.path) {
                    actions.push(RollbackAction::DeleteFile {
                        path: change.path.clone(),
                    });
                } else {
                    actions.push(RollbackAction::CheckByHand {
                        what: format!("file it added at {}", change.path),
                        why: "it is in a place we do not tidy automatically".into(),
                    });
                }
            }
            ChangeKind::Modified => actions.push(RollbackAction::FileWasChanged {
                path: change.path.clone(),
            }),
            ChangeKind::Removed => actions.push(RollbackAction::CheckByHand {
                what: format!("file it removed: {}", change.path),
                why: "a removed file cannot be brought back from the change list alone".into(),
            }),
        }
    }
    for change in run_diff.real_reg_changes() {
        match change.kind {
            ChangeKind::Added => actions.push(RollbackAction::DeleteSetting {
                key: change.key.clone(),
                name: change.name.clone(),
            }),
            ChangeKind::Modified => {
                if let Some(before) = &change.before {
                    actions.push(RollbackAction::RestoreSetting {
                        key: change.key.clone(),
                        name: change.name.clone(),
                        old_data: before.data.clone(),
                    });
                }
            }
            ChangeKind::Removed => actions.push(RollbackAction::CheckByHand {
                what: format!("setting it removed: {}\\{}", change.key, change.name),
                why: "its old value was recorded in the pre-run note and can be restored by hand"
                    .into(),
            }),
        }
    }
    for svc in &run_diff.services_added {
        actions.push(RollbackAction::CheckByHand {
            what: format!("background helper it started: {svc}"),
            why: "helpers are never removed automatically".into(),
        });
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{FileChange, RegChange};
    use crate::snapshot::{sha256_hex, FileEntry, RegEntry};

    fn fe(path: &str) -> FileEntry {
        FileEntry {
            path: path.into(),
            size: 1,
            sha256: sha256_hex(b"x"),
            mtime_secs: 0,
        }
    }

    #[test]
    fn plan_deletes_safely_restores_settings_and_flags_the_rest() {
        let mut d = Diff::default();
        d.files.push(FileChange {
            kind: ChangeKind::Added,
            path: r"C:\Program Files\Acme\widget.exe".into(),
            before: None,
            after: Some(fe(r"C:\Program Files\Acme\widget.exe")),
            noise: false,
        });
        d.files.push(FileChange {
            kind: ChangeKind::Added,
            path: r"C:\Windows\System32\drivers\acme.sys".into(),
            before: None,
            after: Some(fe(r"C:\Windows\System32\drivers\acme.sys")),
            noise: false,
        });
        d.registry.push(RegChange {
            kind: ChangeKind::Modified,
            key: r"HKEY_CURRENT_USER\Software\Acme".into(),
            name: "Mode".into(),
            before: Some(RegEntry {
                key: r"HKEY_CURRENT_USER\Software\Acme".into(),
                name: "Mode".into(),
                data: "\"old\"".into(),
            }),
            after: Some(RegEntry {
                key: r"HKEY_CURRENT_USER\Software\Acme".into(),
                name: "Mode".into(),
                data: "\"new\"".into(),
            }),
            noise: false,
        });
        let plan = rollback_plan(&d);
        assert!(
            matches!(plan[0], RollbackAction::DeleteFile { .. })
                || matches!(plan[1], RollbackAction::DeleteFile { .. })
        );
        assert!(plan.iter().any(|a| matches!(
            a,
            RollbackAction::CheckByHand { what, .. } if what.contains("acme.sys")
        )));
        assert!(plan.iter().any(|a| matches!(
            a,
            RollbackAction::RestoreSetting { old_data, .. } if old_data == "\"old\""
        )));
        assert!(plan.iter().all(|a| !a.plain().is_empty()));
    }
}
