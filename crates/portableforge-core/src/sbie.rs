//! Sandboxie quick-box support: the pure, testable pieces.
//!
//! Sandboxie is an external black box we orchestrate, never
//! reimplement (same rule as Windows Sandbox and `reg.exe`). It is
//! NOT a file we ship — the person installs Sandboxie-Plus
//! themselves; PortableForge detects it and drives it through its
//! own `Start.exe` command line.
//!
//! How a capture works in this mode:
//! - A fresh named box per session means the box folder starts
//!   empty, so every file in it afterwards is something the
//!   program wrote — the box folder IS the after-state.
//! - Registry notes are `reg.exe` exports run INSIDE the box (the
//!   boxed view merges the real registry with the box's own
//!   changes), written to `C:\PFExports\<point>\` and read back
//!   from the box folder's mirror of that path. The same three
//!   hives the in-sandbox agent exports, so the notes feed the
//!   exact same diff/classify pipeline.
//! - The box folder mirrors real paths: `drive\<letter>\…` for
//!   drives and `user\<name>\…` for that user's profile folder.
//!   The default box root is `C:\Sandbox\<user>\<box>` (Sandboxie's
//!   own default FileRootPath); a custom FileRootPath is a known
//!   limitation, reported as an empty capture with a warning.

use std::path::{Path, PathBuf};

use crate::regfile;
use crate::snapshot::{RegEntry, Snapshot};

/// Where `Start.exe` lives in the two install shapes (Plus and
/// Classic). Detection only — nothing is downloaded or bundled.
pub const START_EXE_CANDIDATES: &[&str] = &[
    r"C:\Program Files\Sandboxie-Plus\Start.exe",
    r"C:\Program Files\Sandboxie\Start.exe",
];

/// Folder (inside the box's view of the C: drive) that the boxed
/// registry exports are written to, one subfolder per capture
/// point. Our own bookkeeping — filtered out of file snapshots.
pub const EXPORT_ROOT: &str = r"C:\PFExports";

/// The same three hives the in-sandbox agent exports.
pub const REG_HIVES: [(&str, &str); 3] = [
    ("HKCU\\Software", "hkcu.reg"),
    ("HKLM\\Software", "hklm.reg"),
    (
        "HKLM\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
        "uninstall.reg",
    ),
];

/// A session id becomes a Sandboxie box name: `PF_` plus the id
/// with anything but letters/digits turned into `_`, capped at
/// Sandboxie's 32-character box-name limit, keeping the TAIL of
/// the id (the timestamp) so names stay unique.
pub fn box_name_for(session_id: &str) -> String {
    let cleaned: String = session_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let budget = 32 - 3;
    let tail = if cleaned.len() > budget {
        &cleaned[cleaned.len() - budget..]
    } else {
        &cleaned[..]
    };
    format!("PF_{tail}")
}

/// Sandboxie's default box folder for a user and box name.
pub fn default_box_dir(user_name: &str, box_name: &str) -> PathBuf {
    PathBuf::from(r"C:\Sandbox").join(user_name).join(box_name)
}

/// The box-mirror path of a real host path (drive paths and the
/// named user's profile folder). `None` for anything else.
pub fn host_to_box_path(box_dir: &Path, user_name: &str, host_path: &str) -> Option<PathBuf> {
    let norm = host_path.replace('/', "\\");
    let profile = format!(r"C:\Users\{user_name}");
    if let Some(rest) = strip_prefix_ci(&norm, &profile) {
        return Some(box_dir.join("user").join(user_name).join(rest));
    }
    let bytes = norm.as_bytes();
    if bytes.len() > 3 && bytes[1] == b':' && bytes[2] == b'\\' {
        let letter = (bytes[0] as char).to_ascii_uppercase().to_string();
        return Some(box_dir.join("drive").join(letter).join(&norm[3..]));
    }
    None
}

fn strip_prefix_ci<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    if text.len() >= prefix.len() && text[..prefix.len()].eq_ignore_ascii_case(prefix) {
        let rest = &text[prefix.len()..];
        Some(rest.strip_prefix('\\').unwrap_or(rest))
    } else {
        None
    }
}

/// A command to run inside the box, as data (program + arguments)
/// so callers never build a shell string.
pub fn boxed_command(
    start_exe: &str,
    box_name: &str,
    program: &str,
    args: &[&str],
) -> (String, Vec<String>) {
    let mut out = vec![format!("/box:{box_name}"), program.to_string()];
    out.extend(args.iter().map(|a| a.to_string()));
    (start_exe.to_string(), out)
}

/// The three boxed `reg.exe` exports for one capture point. The
/// caller creates the mirror folder first (creating it on the
/// host inside the box folder is how the box sees it).
pub fn export_commands(start_exe: &str, box_name: &str, point: &str) -> Vec<(String, Vec<String>)> {
    REG_HIVES
        .iter()
        .map(|(hive, file)| {
            let target = format!(r"{EXPORT_ROOT}\{point}\{file}");
            boxed_command(
                start_exe,
                box_name,
                r"C:\Windows\System32\reg.exe",
                &["export", hive, &target, "/y"],
            )
        })
        .collect()
}

/// The host-side mirror folder the exports for a point land in.
pub fn export_mirror_dir(box_dir: &Path, point: &str) -> PathBuf {
    box_dir
        .join("drive")
        .join("C")
        .join("PFExports")
        .join(point)
}

/// True when all three exports for a point are in the mirror.
pub fn exports_ready(box_dir: &Path, point: &str) -> bool {
    let dir = export_mirror_dir(box_dir, point);
    REG_HIVES.iter().all(|(_, file)| dir.join(file).is_file())
}

/// Read one capture point's exports back into registry entries.
/// A hive that is missing or unreadable becomes a warning, never
/// a silent gap (same rule as the agent's capture).
pub fn registry_from_exports(box_dir: &Path, point: &str) -> (Vec<RegEntry>, Vec<String>) {
    let dir = export_mirror_dir(box_dir, point);
    let mut all = Vec::new();
    let mut warnings = Vec::new();
    for (hive, file) in REG_HIVES {
        let path = dir.join(file);
        let parsed = std::fs::read(&path)
            .map_err(|e| format!("{e}"))
            .and_then(|bytes| regfile::decode_reg_export(&bytes))
            .map(|text| regfile::parse_reg(&text));
        match parsed {
            Ok(entries) => all.extend(entries),
            Err(_) => warnings.push(format!(
                "The {hive} part of the remembered settings could not be read, so it was left out of this note."
            )),
        }
    }
    (all, warnings)
}

/// Build the file side of a capture point from the box folder:
/// every file the program wrote, recorded under its real host
/// path. Our own export folder is filtered out; Sandboxie's
/// bookkeeping files live at the box root, outside the scanned
/// `drive\` / `user\` trees, so they never appear.
pub fn snapshot_from_box(box_dir: &Path, user_name: &str, label: impl Into<String>) -> Snapshot {
    let mut roots: Vec<(PathBuf, String)> = Vec::new();
    let drive_root = box_dir.join("drive");
    if let Ok(entries) = std::fs::read_dir(&drive_root) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.len() == 1 && entry.path().is_dir() {
                roots.push((entry.path(), format!("{name}:")));
            }
        }
    }
    let user_root = box_dir.join("user").join(user_name);
    if user_root.is_dir() {
        roots.push((user_root, format!(r"C:\Users\{user_name}")));
    }
    roots.sort();
    let label_text = label.into();
    match crate::snapshot::scan_roots(&roots, label_text.clone()) {
        Ok(mut snap) => {
            snap.files.retain(|f| {
                !f.path
                    .to_ascii_lowercase()
                    .starts_with(&EXPORT_ROOT.to_ascii_lowercase())
            });
            if roots.is_empty() {
                snap.capture_warnings.push(format!(
                    "The quick box folder was not found at {} — if Sandboxie was set up with a different folder, this capture is empty.",
                    box_dir.display()
                ));
            }
            snap
        }
        Err(e) => {
            let mut snap = Snapshot::new(label_text);
            snap.capture_warnings.push(format!(
                "The quick box folder at {} could not be read: {e}",
                box_dir.display()
            ));
            snap
        }
    }
}

/// The install folder pick, with the agent's own heuristic: the
/// folder under Program Files that gained the most files between
/// the clean note and the runtime note. Returns the HOST path;
/// the caller maps it into the box to copy the files out.
pub fn pick_install_dir(before: &Snapshot, after: &Snapshot) -> Option<String> {
    let before_paths: std::collections::BTreeSet<String> = before
        .files
        .iter()
        .map(|f| f.path.to_ascii_lowercase())
        .collect();
    let mut counts: Vec<(String, usize)> = Vec::new();
    for f in &after.files {
        if before_paths.contains(&f.path.to_ascii_lowercase()) {
            continue;
        }
        let lower = f.path.to_ascii_lowercase();
        for marker in [r"\program files (x86)\", r"\program files\"] {
            if let Some(idx) = lower.find(marker) {
                let start = idx + marker.len();
                let first = f.path[start..].split('\\').next().unwrap_or("");
                if !first.is_empty() {
                    let dir = f.path[..start + first.len()].to_string();
                    match counts.iter_mut().find(|(d, _)| *d == dir) {
                        Some((_, n)) => *n += 1,
                        None => counts.push((dir, 1)),
                    }
                }
                break;
            }
        }
    }
    counts.sort_by_key(|b| std::cmp::Reverse(b.1));
    counts.first().map(|(dir, _)| dir.clone())
}

/// End every program running in the box (session clean-up).
pub fn terminate_command(start_exe: &str, box_name: &str) -> (String, Vec<String>) {
    (
        start_exe.to_string(),
        vec![format!("/box:{box_name}"), "/terminate".to_string()],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_names_are_legal_and_unique() {
        let a = box_name_for("PaintFE-20261025-215046");
        assert_eq!(a, "PF_PaintFE_20261025_215046");
        assert!(a.len() <= 32);
        let long = box_name_for("Some Very Long Program Name-20261025-215046");
        assert!(long.len() <= 32);
        assert!(long.ends_with("20261025_215046"), "{long}");
        assert_ne!(a, box_name_for("PaintFE-20261025-215047"));
    }

    #[test]
    fn path_mirror_round_trips() {
        // Separator-agnostic: joins use the host's separator in
        // tests (Linux) and `\` where it matters (Windows).
        fn norm(p: &Path) -> String {
            p.to_string_lossy().replace('/', "\\")
        }
        let box_dir = Path::new(r"C:\Sandbox\Sean\PF_X");
        let p = host_to_box_path(box_dir, "Sean", r"C:\Program Files\PaintFE\paintfe.exe")
            .expect("drive path");
        assert_eq!(
            norm(&p),
            r"C:\Sandbox\Sean\PF_X\drive\C\Program Files\PaintFE\paintfe.exe"
        );
        let p = host_to_box_path(box_dir, "Sean", r"C:\Users\Sean\AppData\Roaming\PaintFE")
            .expect("profile path");
        assert_eq!(
            norm(&p),
            r"C:\Sandbox\Sean\PF_X\user\Sean\AppData\Roaming\PaintFE"
        );
        assert!(host_to_box_path(box_dir, "Sean", "relative\\path").is_none());
    }

    #[test]
    fn boxed_commands_are_data() {
        let (program, args) = boxed_command(
            START_EXE_CANDIDATES[0],
            "PF_X",
            r"C:\Windows\System32\reg.exe",
            &[
                "export",
                "HKCU\\Software",
                r"C:\PFExports\before\hkcu.reg",
                "/y",
            ],
        );
        assert!(program.ends_with("Start.exe"));
        assert_eq!(args[0], "/box:PF_X");
        assert_eq!(args[1], r"C:\Windows\System32\reg.exe");
        let exports = export_commands(START_EXE_CANDIDATES[0], "PF_X", "before");
        assert_eq!(exports.len(), 3);
        assert!(exports[0].1.iter().any(|a| a.contains("hkcu.reg")));
    }

    #[test]
    fn box_snapshot_translates_and_filters() {
        let tmp = std::env::temp_dir().join(format!("pf-sbie-test-{}", std::process::id()));
        let box_dir = tmp.join("Sandbox").join("Sean").join("PF_T");
        let app = box_dir
            .join("drive")
            .join("C")
            .join("Program Files")
            .join("Widget");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("widget.exe"), b"MZ").unwrap();
        let exports = export_mirror_dir(&box_dir, "before");
        std::fs::create_dir_all(&exports).unwrap();
        std::fs::write(exports.join("hkcu.reg"), b"junk").unwrap();
        let snap = snapshot_from_box(&box_dir, "Sean", "post-install");
        assert_eq!(snap.files.len(), 1, "{:?}", snap.files);
        assert_eq!(snap.files[0].path, r"C:\Program Files\Widget\widget.exe");
        // A missing box folder is an empty capture with a warning,
        // never a silent success.
        let snap = snapshot_from_box(&tmp.join("nope"), "Sean", "x");
        assert!(snap.files.is_empty());
        assert_eq!(snap.capture_warnings.len(), 1);
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn install_dir_pick_matches_the_agent_heuristic() {
        let mut before = Snapshot::new("pre");
        before.files.push(crate::snapshot::FileEntry {
            path: r"C:\Program Files\Other\old.exe".into(),
            size: 1,
            sha256: String::new(),
            mtime_secs: 0,
        });
        let mut after = before.clone();
        for n in ["a.exe", "b.dll"] {
            after.files.push(crate::snapshot::FileEntry {
                path: format!(r"C:\Program Files\Widget\{n}"),
                size: 1,
                sha256: String::new(),
                mtime_secs: 0,
            });
        }
        assert_eq!(
            pick_install_dir(&before, &after).as_deref(),
            Some(r"C:\Program Files\Widget")
        );
        assert!(pick_install_dir(&before, &before).is_none());
    }
}
