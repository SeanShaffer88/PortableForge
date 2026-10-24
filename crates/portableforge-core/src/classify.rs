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

/// A file sitting directly in the profile root (`C:\Users\<name>\<file>`)
/// — not in any subfolder. Returns the file name.
fn profile_root_file(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split('\\').collect();
    if parts.len() == 4
        && parts[1].eq_ignore_ascii_case("users")
        && !parts[2].is_empty()
        && !parts[3].is_empty()
    {
        return Some(parts[3].to_string());
    }
    None
}

pub fn classify(install: &Diff, runtime: &Diff, install_dirs: &[String]) -> Vec<Classification> {
    let mut out: Vec<Classification> = Vec::new();
    let mut move_dirs_seen: Vec<String> = Vec::new();

    // Services added by either pass are always red flags (a service
    // both passes saw is still one service — de-duplicated here and
    // again for every finding type at the end).
    let mut services: Vec<&String> = install
        .services_added
        .iter()
        .chain(&runtime.services_added)
        .collect();
    services.sort();
    services.dedup();
    for svc in services {
        out.push(Classification {
            finding: Finding::RedFlag {
                kind: "service".into(),
                detail: format!("Installs Windows service '{svc}'"),
            },
            severity: Severity::RedFlag,
            confidence: 1.0,
            why: format!(
                "Service '{svc}': a portable app cannot cleanly install/remove a service \
                 on every host; this package will need RequiresAdmin handling or should \
                 not be claimed portable."
            ),
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
        // Fonts land in a shared system folder — a portable package
        // cannot own them, so they are always a red flag.
        if path.to_ascii_lowercase().contains(r"\windows\fonts\") {
            let font = path.rsplit('\\').next().unwrap_or(&path).to_string();
            out.push(Classification {
                finding: Finding::RedFlag {
                    kind: "font".into(),
                    detail: format!("Installs the font '{font}'"),
                },
                severity: Severity::RedFlag,
                confidence: 1.0,
                why: format!(
                    "Font '{font}' was installed into the Windows Fonts folder ({path}). \
                     Fonts are shared by the whole machine; the portable copy can bundle \
                     the font file, but installing it is not portable."
                ),
            });
            continue;
        }
        if let Some(dir) = is_program_files(&path) {
            if !is_under_install_dir(&path, install_dirs) {
                out.push(Classification {
                    finding: Finding::AppFile { path: path.clone() },
                    severity: Severity::Info,
                    confidence: 0.9,
                    why: format!(
                        "Program file {path} under detected install dir '{dir}' — belongs in App\\."
                    ),
                });
            } else {
                out.push(Classification {
                    finding: Finding::AppFile { path: path.clone() },
                    severity: Severity::Info,
                    confidence: 0.95,
                    why: format!(
                        "Program file {path} under the install directory — belongs in App\\."
                    ),
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
                        "The app writes under a per-user location ({host_pattern}) — \
                         for example {path}. The launcher will move it into the \
                         portable Data directory."
                    ),
                });
            }
            continue;
        }
        // A single file loose in the profile root: moved as a file,
        // not as a folder.
        if let Some(file_name) = profile_root_file(&path) {
            out.push(Classification {
                finding: Finding::FilesMove {
                    host_pattern: "%USERPROFILE%".to_string(),
                    data_target: file_name,
                },
                severity: Severity::Review,
                confidence: 0.7,
                why: format!(
                    "The app writes the single file {path} straight into the profile \
                     folder; the launcher will carry that one file in Data\\."
                ),
            });
            continue;
        }
        if is_under_install_dir(&path, install_dirs) {
            out.push(Classification {
                finding: Finding::AppFile { path: path.clone() },
                severity: Severity::Info,
                confidence: 0.85,
                why: format!("File {path} under the install directory — belongs in App\\."),
            });
        }
    }

    for change in install.real_reg_changes().chain(runtime.real_reg_changes()) {
        let key = change.key.clone();
        let lower = key.to_ascii_lowercase();
        if noise::is_uninstall_key(&key) {
            out.push(Classification {
                finding: Finding::UninstallMetadata { key: key.clone() },
                severity: Severity::Info,
                confidence: 1.0,
                why: format!(
                    "Uninstall registration at {key} — used only to pre-fill name/version metadata."
                ),
            });
            continue;
        }
        // Scheduled tasks and firewall rules get their own red flags:
        // they are machine-level commitments with names of their own,
        // and lumping them in with generic registry writes hides them.
        let special = if lower.contains(r"\windows nt\currentversion\schedule") {
            Some((
                "scheduled-task",
                "Registers a scheduled task",
                "A scheduled task runs on the machine's own timetable, whether or not \
                 the portable copy is even present; that is not something a portable \
                 package can own.",
            ))
        } else if lower.contains("firewallpolicy") {
            Some((
                "firewall-rule",
                "Adds a firewall rule",
                "A firewall rule changes the machine's network defences for every \
                 program, not just this one; a portable package must not quietly \
                 own that.",
            ))
        } else {
            None
        };
        if let Some((kind, what, why)) = special {
            out.push(Classification {
                finding: Finding::RedFlag {
                    kind: kind.into(),
                    detail: format!("{what}: {key}"),
                },
                severity: Severity::RedFlag,
                confidence: 0.95,
                why: format!("{why} Evidence: {key}."),
            });
            continue;
        }
        // COM / ActiveX class registration is not a file association:
        // it wires a component into the machine's class store.
        if lower.contains(r"\clsid\") || lower.ends_with(r"\clsid") {
            out.push(Classification {
                finding: Finding::RedFlag {
                    kind: "com-registration".into(),
                    detail: format!("Registers a COM/ActiveX component: {key}"),
                },
                severity: Severity::RedFlag,
                confidence: 0.9,
                why: format!(
                    "COM registration at {key}: other programs find this component \
                     through the machine's class registration, which a portable \
                     package cannot cleanly provide or remove."
                ),
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
                why: format!(
                    "Machine-wide registry write at {key}: HKLM state is shared by \
                     all users and needs admin rights to write; a portable package \
                     must justify it explicitly."
                ),
            });
            continue;
        }
        if lower.starts_with("hkey_classes_root") || lower.contains(r"\software\classes\") {
            out.push(Classification {
                finding: Finding::AssociationCandidate { key: key.clone() },
                severity: Severity::Review,
                confidence: 0.8,
                why: format!(
                    "File-association / shell registration at {key}. Off by default — \
                     enable only if the package should claim these file types."
                ),
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
                    finding: Finding::RegistryKey { key: short.clone() },
                    severity: Severity::Info,
                    confidence: 0.9,
                    why: format!(
                        "Per-user settings key {short} — the launcher will carry it in Data\\settings."
                    ),
                });
            }
        }
    }

    // One finding, once — whatever the two passes each saw. The two
    // diffs overlap by nature (an install change still shows in the
    // runtime pass), and the review must never show the same card
    // twice.
    let mut seen: Vec<Finding> = Vec::new();
    out.retain(|c| {
        if seen.contains(&c.finding) {
            false
        } else {
            seen.push(c.finding.clone());
            true
        }
    });
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
            "The file {path} embeds '{matched}' — an absolute path learned on one \
             machine. The launcher must rewrite it to the portable location on \
             every run."
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
    fn duplicates_across_passes_collapse_to_one_finding() {
        let mut install = Diff::default();
        install.services_added.push("AcmeSvc".into());
        install
            .files
            .push(added_file(r"C:\Program Files\Acme\Widget\widget.exe"));
        install.registry.push(added_reg(
            r"HKEY_LOCAL_MACHINE\SOFTWARE\Acme\Widget",
            "Path",
        ));
        let runtime = install.clone();
        let out = classify(&install, &runtime, &[]);
        let count = |pred: &dyn Fn(&Classification) -> bool| out.iter().filter(|c| pred(c)).count();
        assert_eq!(
            count(&|c| matches!(&c.finding, Finding::RedFlag { kind, .. } if kind == "service")),
            1
        );
        assert_eq!(count(&|c| matches!(&c.finding, Finding::AppFile { .. })), 1);
        assert_eq!(
            count(
                &|c| matches!(&c.finding, Finding::RedFlag { kind, .. } if kind == "registry-hklm")
            ),
            1
        );
    }

    #[test]
    fn fonts_profile_root_files_com_and_machine_registrations() {
        let mut install = Diff::default();
        install
            .files
            .push(added_file(r"C:\Windows\Fonts\AcmeSans.ttf"));
        install
            .files
            .push(added_file(r"C:\Users\WDAGUtilityAccount\widget.ini"));
        install.registry.push(added_reg(
            r"HKEY_LOCAL_MACHINE\SOFTWARE\Classes\CLSID\{11111111-2222-3333-4444-555555555555}",
            "InprocServer32",
        ));
        install
            .registry
            .push(added_reg(r"HKEY_CLASSES_ROOT\.wid", "Content Type"));
        install.registry.push(added_reg(
            r"HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Schedule\TaskCache\Tree\Acme",
            "Id",
        ));
        install.registry.push(added_reg(
            r"HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Services\SharedAccess\Parameters\FirewallPolicy\FirewallRules",
            "AcmeRule",
        ));
        let out = classify(&install, &Diff::default(), &[]);
        let kinds: Vec<&str> = out
            .iter()
            .filter_map(|c| match &c.finding {
                Finding::RedFlag { kind, .. } => Some(kind.as_str()),
                _ => None,
            })
            .collect();
        assert!(kinds.contains(&"font"), "{kinds:?}");
        assert!(kinds.contains(&"com-registration"), "{kinds:?}");
        assert!(kinds.contains(&"scheduled-task"), "{kinds:?}");
        assert!(kinds.contains(&"firewall-rule"), "{kinds:?}");
        // The plain .wid association stays an association candidate.
        assert!(out
            .iter()
            .any(|c| matches!(&c.finding, Finding::AssociationCandidate { .. })));
        // The loose profile-root file is a FilesMove for %USERPROFILE%.
        assert!(out.iter().any(|c| matches!(
            &c.finding,
            Finding::FilesMove { host_pattern, data_target }
            if host_pattern == "%USERPROFILE%" && data_target == "widget.ini"
        )));
    }

    #[test]
    fn every_why_names_its_evidence() {
        // The audit behind the evidence rule: build one of every
        // finding type and require its `why` to name the thing it
        // was decided from (path, key, or service name).
        let mut install = Diff::default();
        install.services_added.push("EvidenceSvc".into());
        install
            .files
            .push(added_file(r"C:\Program Files\Acme\Widget\widget.exe"));
        install
            .files
            .push(added_file(r"C:\Windows\Fonts\Evidence.ttf"));
        install.files.push(added_file(r"C:\Users\X\loose.ini"));
        let mut runtime = Diff::default();
        runtime.files.push(added_file(
            r"C:\Users\X\AppData\Roaming\Acme\Widget\settings.ini",
        ));
        runtime
            .registry
            .push(added_reg(r"HKEY_CURRENT_USER\Software\Acme\Widget", "Mode"));
        runtime.registry.push(added_reg(
            r"HKEY_LOCAL_MACHINE\SOFTWARE\Acme\Widget",
            "Path",
        ));
        runtime
            .registry
            .push(added_reg(r"HKEY_CLASSES_ROOT\.evi", "Content Type"));
        runtime.registry.push(added_reg(
            r"HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Acme",
            "DisplayName",
        ));
        let out = classify(&install, &runtime, &[]);
        assert!(out.len() >= 8, "expected a finding of each kind: {out:?}");
        for c in &out {
            let evidence = match &c.finding {
                Finding::AppFile { path } => path.clone(),
                Finding::DirectoriesMove { host_pattern, .. } => host_pattern.clone(),
                Finding::FilesMove { data_target, .. } => data_target.clone(),
                Finding::RegistryKey { key } => key.clone(),
                Finding::FileWriteRewrite { path, .. } => path.clone(),
                Finding::AssociationCandidate { key } => key.clone(),
                Finding::RedFlag { detail, .. } => {
                    // Detail always embeds the evidence; the why must
                    // carry an identifying fragment of it.
                    detail
                        .split(['\'', ':'])
                        .filter(|s| s.len() > 3)
                        .nth(1)
                        .unwrap_or(detail)
                        .trim()
                        .to_string()
                }
                Finding::UninstallMetadata { key } => key.clone(),
            };
            assert!(
                c.why.contains(&evidence),
                "why for {:?} does not name its evidence {evidence:?}: {}",
                c.finding,
                c.why
            );
        }
        let fw = classify_file_content(
            r"App\Widget\settings.ini",
            "home=C:\\Users\\WDAGUtilityAccount\n",
        )
        .unwrap();
        assert!(fw.why.contains(r"App\Widget\settings.ini"));
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
