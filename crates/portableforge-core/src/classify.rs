//! Classification: turn install/runtime diffs into PortableApps.com
//! Launcher handling. Every finding carries its evidence and a confidence
//! so the review table can show *why* — the user accepts, redirects, or
//! ignores each row; nothing here is applied silently.

use serde::{Deserialize, Serialize};

use crate::diff::{ChangeKind, Diff};
use crate::noise;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Severity {
    /// Handled automatically by generated config.
    Info,
    /// Needs the user's eye in review (association candidates, rewrites).
    Review,
    /// Blocks a clean "portable" claim until the user decides
    /// (services, drivers, HKLM state).
    RedFlag,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Finding {
    /// Program files belonging in `App\<AppName>\`.
    AppFile { path: String },
    /// A directory the app writes under a per-user location; the
    /// launcher moves it between the host location and `Data\`.
    DirectoriesMove {
        /// Host-side path as a PAL-style pattern, e.g. `%APPDATA%\Acme\Widget`.
        host_pattern: String,
        /// Single folder name inside `Data\` (PAL: the line's key).
        data_target: String,
        /// A specimen file seen there (evidence), if any.
        evidence_file: String,
    },
    /// A single settings file to move. PAL: the line is
    /// `<file name>=<host directory>` — `data_target` is the file name,
    /// `host_pattern` is the host directory that holds it.
    FilesMove {
        host_pattern: String,
        data_target: String,
    },
    /// A registry key the launcher should carry in `Data\settings`.
    RegistryKey { key: String },
    /// A file whose contents embed an absolute sandbox path — proof of a
    /// hard-coded path the launcher must rewrite (`[FileWriteN]`).
    FileWriteRewrite { path: String, matched_text: String },
    /// File-association / protocol writes — off by default, opt-in only.
    AssociationCandidate { key: String },
    /// Machine-wide state a portable app cannot cleanly own.
    RedFlag { kind: String, detail: String },
    /// Uninstall-registration metadata (feeds the metadata form only).
    UninstallMetadata { key: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Classification {
    pub finding: Finding,
    pub severity: Severity,
    /// 0.0–1.0 — how sure the classifier is. Shown in the review table.
    pub confidence: f32,
    /// Plain-language explanation for the review table.
    pub why: String,
}

/// Per-user locations, longest first, mapped to PAL variables.
/// Returns `(pal_pattern_prefix, data_subdir)` for a matched path.
fn per_user_location(path: &str) -> Option<(&'static str, &'static str)> {
    let p = path.to_ascii_lowercase();
    // Sandbox + real profiles share these suffix shapes; match on the
    // well-known folder names anywhere after a Users\<name>\ prefix.
    const LOCATIONS: &[(&str, &str, &str)] = &[
        (r"\appdata\roaming\", "%APPDATA%", "AppData\\Roaming"),
        (r"\appdata\local\", "%LOCALAPPDATA%", "AppData\\Local"),
        (r"\documents\", "%USERPROFILE%\\Documents", "Documents"),
        (r"\saved games\", "%USERPROFILE%\\Saved Games", "SavedGames"),
    ];
    for (needle, pal, sub) in LOCATIONS {
        if p.contains(needle) {
            return Some((pal, sub));
        }
    }
    let _ = p;
    None
}

/// Extract the part of `path` after a per-user location marker,
/// e.g. `…\AppData\Roaming\Acme\Widget\settings.ini` -> `Acme\Widget\settings.ini`.
fn after_location(path: &str) -> Option<String> {
    let lower = path.to_ascii_lowercase();
    for marker in [
        r"\appdata\roaming\",
        r"\appdata\local\",
        r"\documents\",
        r"\saved games\",
    ] {
        if let Some(idx) = lower.find(marker) {
            return Some(path[idx + marker.len()..].to_string());
        }
    }
    None
}

fn is_under_install_dir(path: &str, install_dirs: &[String]) -> bool {
    let p = path.to_ascii_lowercase();
    install_dirs
        .iter()
        .any(|d| p.starts_with(&d.to_ascii_lowercase()))
}

fn is_program_files(path: &str) -> Option<String> {
    let lower = path.to_ascii_lowercase();
    for marker in [r"\program files (x86)\", r"\program files\"] {
        if let Some(idx) = lower.find(marker) {
            let rest = &path[idx + marker.len()..];
            // Install dir = first component under Program Files.
            let first = rest.split('\\').next().unwrap_or("");
            if !first.is_empty() {
                let start = idx + marker.len();
                return Some(path[..start + first.len()].to_string());
            }
        }
    }
    None
}

/// Canary strings proving a file embeds an absolute local path.
/// `WDAGUtilityAccount` is the Windows Sandbox user — a harvested file
/// naming it could only have learned that path at run time in the sandbox.
pub const CANARIES: &[&str] = &["WDAGUtilityAccount", "C:\\Users\\", "C:/Users/"];

pub fn scan_content_for_canary(content: &str) -> Option<String> {
    for c in CANARIES {
        if content.contains(c) {
            return Some((*c).to_string());
        }
    }
    None
}

pub fn classify(install: &Diff, runtime: &Diff, install_dirs: &[String]) -> Vec<Classification> {
    let mut out: Vec<Classification> = Vec::new();
    let mut move_dirs_seen: Vec<String> = Vec::new();

    // Services added by either pass are always red flags.
    for svc in install.services_added.iter().chain(&runtime.services_added) {
        out.push(Classification {
            finding: Finding::RedFlag {
                kind: "service".into(),
                detail: format!("Installs Windows service '{svc}'"),
            },
            severity: Severity::RedFlag,
            confidence: 1.0,
            why: "A portable app cannot cleanly install/remove a service on every host; \
                  this package will need RequiresAdmin handling or should not be claimed portable."
                .into(),
        });
    }

    for change in install
        .real_file_changes()
        .chain(runtime.real_file_changes())
    {
        if change.kind == ChangeKind::Removed {
            continue;
        }
        let path = change.path.clone();
        if let Some(dir) = is_program_files(&path) {
            if !is_under_install_dir(&path, install_dirs) {
                out.push(Classification {
                    finding: Finding::AppFile { path },
                    severity: Severity::Info,
                    confidence: 0.9,
                    why: format!(
                        "Program file under detected install dir '{dir}' — belongs in App\\."
                    ),
                });
            } else {
                out.push(Classification {
                    finding: Finding::AppFile { path },
                    severity: Severity::Info,
                    confidence: 0.95,
                    why: "Program file under the install directory — belongs in App\\.".into(),
                });
            }
            continue;
        }
        if let (Some((pal, _sub)), Some(rest)) = (per_user_location(&path), after_location(&path)) {
            // Group by the top two components (Vendor\App) when present.
            let parts: Vec<&str> = rest.split('\\').collect();
            let group = if parts.len() >= 2 {
                format!("{}\\{}", parts[0], parts[1])
            } else {
                rest.clone()
            };
            let host_pattern = format!("{pal}\\{group}");
            if !move_dirs_seen.contains(&host_pattern) {
                move_dirs_seen.push(host_pattern.clone());
                // PAL [DirectoriesMove]: the key is a single folder name
                // inside Data — the last component of the host path
                // (Application Template 3.5.2 + PAL manual, verified).
                // The manual adds a hard rule: never use `settings` as
                // the key — Data\settings belongs to registry storage
                // and moved data there would be lost.
                let raw_folder = group
                    .rsplit('\\')
                    .next()
                    .unwrap_or("Settings")
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
                    .collect::<String>();
                let data_folder = if raw_folder.eq_ignore_ascii_case("settings") {
                    format!("{raw_folder}Data")
                } else {
                    raw_folder
                };
                out.push(Classification {
                    finding: Finding::DirectoriesMove {
                        host_pattern: host_pattern.clone(),
                        data_target: if data_folder.is_empty() {
                            "AppData".to_string()
                        } else {
                            data_folder
                        },
                        evidence_file: path.clone(),
                    },
                    severity: Severity::Info,
                    confidence: 0.9,
                    why: format!(
                        "The app writes under a per-user location ({host_pattern}); \
                         the launcher will move it into the portable Data directory."
                    ),
                });
            }
            continue;
        }
        if is_under_install_dir(&path, install_dirs) {
            out.push(Classification {
                finding: Finding::AppFile { path },
                severity: Severity::Info,
                confidence: 0.85,
                why: "File under the install directory — belongs in App\\.".into(),
            });
        }
    }

    for change in install.real_reg_changes().chain(runtime.real_reg_changes()) {
        let key = change.key.clone();
        let lower = key.to_ascii_lowercase();
        if noise::is_uninstall_key(&key) {
            out.push(Classification {
                finding: Finding::UninstallMetadata { key },
                severity: Severity::Info,
                confidence: 1.0,
                why: "Uninstall registration — used only to pre-fill name/version metadata.".into(),
            });
            continue;
        }
        if lower.starts_with("hkey_local_machine") {
            out.push(Classification {
                finding: Finding::RedFlag {
                    kind: "registry-hklm".into(),
                    detail: format!("Writes machine-wide registry: {key}"),
                },
                severity: Severity::RedFlag,
                confidence: 0.95,
                why: "HKLM state is shared by all users and needs admin rights to write; \
                      a portable package must justify it explicitly."
                    .into(),
            });
            continue;
        }
        if lower.starts_with("hkey_classes_root") || lower.contains(r"\software\classes\") {
            out.push(Classification {
                finding: Finding::AssociationCandidate { key },
                severity: Severity::Review,
                confidence: 0.8,
                why: "File-association / shell registration. Off by default — \
                      enable only if the package should claim these file types."
                    .into(),
            });
            continue;
        }
        if lower.starts_with("hkey_current_user\\software\\") {
            // One RegistryKey finding per distinct 4-component key
            // (HKCU\Software\<Vendor>\<App>).
            let parts: Vec<&str> = key.split('\\').collect();
            let short = if parts.len() >= 4 {
                parts[..4].join("\\")
            } else {
                key.clone()
            };
            let already = out
                .iter()
                .any(|c| matches!(&c.finding, Finding::RegistryKey { key: k } if *k == short));
            if !already {
                out.push(Classification {
                    finding: Finding::RegistryKey { key: short },
                    severity: Severity::Info,
                    confidence: 0.9,
                    why: "Per-user settings key — the launcher will carry it in Data\\settings."
                        .into(),
                });
            }
        }
    }

    out
}

/// Convenience: classify file contents harvested from the sandbox for
/// absolute-path canaries (drives `[FileWriteN]` rules).
pub fn classify_file_content(path: &str, content: &str) -> Option<Classification> {
    let matched = scan_content_for_canary(content)?;
    Some(Classification {
        finding: Finding::FileWriteRewrite {
            path: path.to_string(),
            matched_text: matched.clone(),
        },
        severity: Severity::Review,
        confidence: 0.95,
        why: format!(
            "This file embeds '{matched}' — an absolute path learned on one machine. \
             The launcher must rewrite it to the portable location on every run."
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::FileChange;
    use crate::snapshot::sha256_hex;

    fn added_file(path: &str) -> FileChange {
        FileChange {
            kind: ChangeKind::Added,
            path: path.into(),
            before: None,
            after: Some(crate::snapshot::FileEntry {
                path: path.into(),
                size: 1,
                sha256: sha256_hex(b"x"),
                mtime_secs: 0,
            }),
            noise: false,
        }
    }

    fn added_reg(key: &str, name: &str) -> crate::diff::RegChange {
        crate::diff::RegChange {
            kind: ChangeKind::Added,
            key: key.into(),
            name: name.into(),
            before: None,
            after: Some(crate::snapshot::RegEntry {
                key: key.into(),
                name: name.into(),
                data: "\"1\"".into(),
            }),
            noise: false,
        }
    }

    #[test]
    fn appdata_becomes_directories_move_hkcu_becomes_registry_key() {
        let mut install = Diff::default();
        install
            .files
            .push(added_file(r"C:\Program Files\Acme\Widget\widget.exe"));
        let mut runtime = Diff::default();
        runtime.files.push(added_file(
            r"C:\Users\WDAGUtilityAccount\AppData\Roaming\Acme\Widget\settings.ini",
        ));
        runtime
            .registry
            .push(added_reg(r"HKEY_CURRENT_USER\Software\Acme\Widget", "Mode"));
        let out = classify(&install, &runtime, &[]);
        assert!(out
            .iter()
            .any(|c| matches!(&c.finding, Finding::AppFile { .. })));
        assert!(out.iter().any(|c| matches!(
            &c.finding,
            Finding::DirectoriesMove { host_pattern, .. } if host_pattern == r"%APPDATA%\Acme\Widget"
        )));
        assert!(out.iter().any(|c| matches!(
            &c.finding,
            Finding::RegistryKey { key } if key == r"HKEY_CURRENT_USER\Software\Acme\Widget"
        )));
    }

    #[test]
    fn directories_move_key_is_never_settings() {
        // PAL manual rule: Data\settings is registry storage; a moved
        // folder keyed `settings` would lose data.
        let mut runtime = Diff::default();
        runtime.files.push(added_file(
            r"C:\Users\X\AppData\Roaming\Acme\Settings\opts.ini",
        ));
        let out = classify(&Diff::default(), &runtime, &[]);
        let mv = out.iter().find_map(|c| match &c.finding {
            Finding::DirectoriesMove { data_target, .. } => Some(data_target.clone()),
            _ => None,
        });
        assert_eq!(mv.as_deref(), Some("SettingsData"));
    }

    #[test]
    fn services_and_hklm_are_red_flags() {
        let mut install = Diff::default();
        install.services_added.push("AcmeSvc".into());
        install.registry.push(added_reg(
            r"HKEY_LOCAL_MACHINE\SOFTWARE\Acme\Widget",
            "Path",
        ));
        let out = classify(&install, &Diff::default(), &[]);
        assert_eq!(
            out.iter()
                .filter(|c| c.severity == Severity::RedFlag)
                .count(),
            2
        );
    }

    #[test]
    fn canary_scan_catches_sandbox_username() {
        let c = classify_file_content(
            "settings.ini",
            "lastdir=C:\\Users\\WDAGUtilityAccount\\Documents\n",
        )
        .unwrap();
        assert_eq!(c.severity, Severity::Review);
        assert!(classify_file_content("a.ini", "theme=dark\n").is_none());
    }
}
