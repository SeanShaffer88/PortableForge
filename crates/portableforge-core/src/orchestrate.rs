//! Sandbox orchestration (host side): preflight evaluation, the
//! on-disk session layout, the launch plan, heartbeat reading, and
//! harvesting results out of the exchange folder.
//!
//! Everything here is file-based and platform-neutral so it can be
//! tested anywhere; the few truly Windows-only fact *collections*
//! (edition, feature state) live in the host binary and feed
//! [`PreflightFacts`] into [`evaluate_preflight`].

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::diff::diff_snapshots;
use crate::session::{Phase, Session};
use crate::snapshot::Snapshot;
use crate::wsb::{SandboxConfig, SandboxMode};

// ---------------------------------------------------------------- preflight

/// Facts about the machine, collected by the host binary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreflightFacts {
    /// True when running on Windows at all.
    pub is_windows: bool,
    /// Windows build number (Sandbox needs 18342+); 0 when unknown.
    pub os_build: u32,
    /// Edition supports Windows Sandbox (Pro/Enterprise/Education/IoT).
    pub edition_supported: bool,
    /// `WindowsSandbox.exe` is present in System32.
    pub sandbox_exe_present: bool,
    /// The optional feature is enabled; `None` = could not be determined.
    pub feature_enabled: Option<bool>,
    /// `None` = could not be measured (the check is then skipped, and
    /// the report says so — never a guessed pass or fail).
    pub free_disk_mb: Option<u64>,
    pub free_ram_mb: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightCheck {
    /// Plain-language check name (wizard wording — no jargon).
    pub label: String,
    pub ok: bool,
    /// Plain-language fix when not ok; empty when ok.
    pub fix: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightReport {
    pub checks: Vec<PreflightCheck>,
    /// The best sandbox mode this machine can honestly offer.
    pub best_mode: SandboxMode,
}

impl PreflightReport {
    pub fn sandbox_ready(&self) -> bool {
        self.best_mode == SandboxMode::WindowsSandbox && self.checks.iter().all(|c| c.ok)
    }
}

pub fn evaluate_preflight(f: &PreflightFacts) -> PreflightReport {
    let mut checks = Vec::new();
    let mut check = |label: &str, ok: bool, fix: &str| {
        checks.push(PreflightCheck {
            label: label.to_string(),
            ok,
            fix: if ok { String::new() } else { fix.to_string() },
        });
    };

    check(
        "This is a Windows computer",
        f.is_windows,
        "PortableForge makes portable Windows programs, so it needs to run on Windows.",
    );
    check(
        "Windows is new enough",
        f.os_build >= 18342,
        "Update Windows: the safe test box needs Windows 10 build 18342 or newer, or Windows 11.",
    );
    check(
        "Your Windows edition includes the safe test box",
        f.edition_supported,
        "The test box comes with Windows Pro, Enterprise, or Education. On Windows Home, use Careful mode instead — we will explain it honestly before anything runs.",
    );
    check(
        "The safe test box is installed",
        f.sandbox_exe_present,
        "Turn on 'Windows Sandbox' in Windows Features (we can show you exactly where), then restart the computer.",
    );
    // Unknown facts are never dressed up as passes: an unmeasured check
    // does not block, but its label says it was not checked.
    check(
        if f.feature_enabled.is_none() {
            "The safe test box is switched on (could not be checked)"
        } else {
            "The safe test box is switched on"
        },
        f.feature_enabled != Some(false),
        "Windows Sandbox is installed but switched off. Turn it on in Windows Features, then restart.",
    );
    check(
        if f.free_disk_mb.is_none() {
            "Enough free space (could not be measured)"
        } else {
            "Enough free space"
        },
        f.free_disk_mb.map(|mb| mb >= 2048).unwrap_or(true),
        "Free up at least 2 GB: the test box and the program being converted both need room.",
    );
    check(
        if f.free_ram_mb.is_none() {
            "Enough free memory (could not be measured)"
        } else {
            "Enough free memory"
        },
        f.free_ram_mb.map(|mb| mb >= 2048).unwrap_or(true),
        "Close some other programs first — the test box needs about 2 GB of memory to itself.",
    );

    let sandbox_ok = f.is_windows
        && f.os_build >= 18342
        && f.edition_supported
        && f.sandbox_exe_present
        && f.feature_enabled != Some(false);
    let best_mode = if sandbox_ok {
        SandboxMode::WindowsSandbox
    } else if f.is_windows {
        // A user VM cannot be detected reliably; Mode B is chosen by the
        // user, not auto-detected. Fallback offer is the honest Mode C.
        SandboxMode::HostSnapshot
    } else {
        SandboxMode::HostSnapshot
    };
    PreflightReport { checks, best_mode }
}

// ------------------------------------------------------------ session layout

/// On-disk layout of one packaging session:
/// ```text
/// <base>\<session-id>\
///   session.json     the Session record
///   box.wsb          generated Sandbox configuration (Mode A)
///   input\           the original installer (mapped read-only)
///   tools\           portableforge-agent.exe (mapped read-only)
///   exchange\        snapshots, harvest, state.json heartbeat (read/write)
///   output\          the finished package tree and .paf.exe
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionLayout {
    pub root: PathBuf,
    pub input: PathBuf,
    pub tools: PathBuf,
    pub exchange: PathBuf,
    pub output: PathBuf,
    pub wsb_path: PathBuf,
    pub session_path: PathBuf,
}

pub fn session_layout(base: &Path, session_id: &str) -> SessionLayout {
    let root = base.join(session_id);
    SessionLayout {
        input: root.join("input"),
        tools: root.join("tools"),
        exchange: root.join("exchange"),
        output: root.join("output"),
        wsb_path: root.join("box.wsb"),
        session_path: root.join("session.json"),
        root,
    }
}

pub fn prepare_session(
    base: &Path,
    session: &Session,
    agent_source: Option<&Path>,
    networking: bool,
) -> Result<SessionLayout, String> {
    let layout = session_layout(base, &session.id);
    for dir in [
        &layout.root,
        &layout.input,
        &layout.tools,
        &layout.exchange,
        &layout.output,
    ] {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("We could not make the work folder {}: {e}", dir.display()))?;
    }
    if let Some(agent) = agent_source {
        let dest = layout.tools.join("portableforge-agent.exe");
        std::fs::copy(agent, &dest)
            .map_err(|e| format!("We could not copy the helper program: {e}"))?;
    }
    std::fs::write(&layout.session_path, session.to_json())
        .map_err(|e| format!("We could not save the run record: {e}"))?;
    if session.sandbox_mode == SandboxMode::WindowsSandbox {
        let cfg = SandboxConfig {
            input_dir: layout.input.to_string_lossy().into_owned(),
            tools_dir: layout.tools.to_string_lossy().into_owned(),
            exchange_dir: layout.exchange.to_string_lossy().into_owned(),
            networking,
            ..Default::default()
        };
        cfg.check()?;
        std::fs::write(&layout.wsb_path, cfg.to_wsb())
            .map_err(|e| format!("We could not save the test box settings: {e}"))?;
    }
    Ok(layout)
}

/// The command that starts the test box on Windows. Returned as data
/// (program + argument) so callers never build a shell string.
pub fn sandbox_launch_command(layout: &SessionLayout) -> (String, String) {
    (
        "WindowsSandbox.exe".to_string(),
        layout.wsb_path.to_string_lossy().into_owned(),
    )
}

/// Read the agent's current state from the exchange folder, if it has
/// written one yet.
pub fn read_agent_state(layout: &SessionLayout) -> Option<Session> {
    let text = std::fs::read_to_string(layout.exchange.join("state.json")).ok()?;
    Session::from_json(&text).ok()
}

// ------------------------------------------------------------------ harvest

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HarvestReport {
    pub files_copied: usize,
    pub bytes_copied: u64,
    pub skipped: Vec<String>,
}

/// Copy a harvested tree (from the exchange folder) into its package
/// destination, preserving relative structure. Refuses path escapes:
/// any harvested entry that would land outside the destination is
/// skipped and reported, never written.
pub fn harvest_tree(source: &Path, dest: &Path) -> Result<HarvestReport, String> {
    let mut report = HarvestReport::default();
    let mut stack = vec![source.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) => {
                report.skipped.push(format!("{}: {e}", dir.display()));
                continue;
            }
        };
        for item in entries.flatten() {
            let path = item.path();
            let Ok(kind) = item.file_type() else { continue };
            if kind.is_dir() {
                stack.push(path);
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            let rel = path.strip_prefix(source).unwrap_or(&path).to_path_buf();
            // Safety: no absolute or parent-escaping relative paths.
            if rel.components().any(|c| {
                matches!(
                    c,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            }) {
                report
                    .skipped
                    .push(format!("unsafe path skipped: {}", rel.display()));
                continue;
            }
            let target = dest.join(&rel);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("We could not make {}: {e}", parent.display()))?;
            }
            let bytes = std::fs::copy(&path, &target)
                .map_err(|e| format!("We could not copy {}: {e}", path.display()))?;
            report.files_copied += 1;
            report.bytes_copied += bytes;
        }
    }
    Ok(report)
}

// ------------------------------------------------------- specimen harvesting

/// One first-run specimen: a file the app created in a per-user
/// location while being tried out. Its contents become the package's
/// `App\DefaultData` seed (so a fresh portable copy starts with sane
/// settings), and the *moved* original lives in `Data\` at run time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Specimen {
    /// Absolute path inside the sandbox where the file was seen.
    pub sandbox_path: String,
    /// Where it belongs relative to the package root, e.g.
    /// `Data\Widget\settings.ini` for the live copy.
    pub data_relative: String,
    /// The DefaultData seed location relative to the package root.
    pub default_data_relative: String,
}

/// Plan specimens from the runtime diff: every added/modified file
/// under a recognised per-user location, grouped by the same folder
/// the classifier chose for its `[DirectoriesMove]` entry.
pub fn specimen_plan(runtime: &crate::diff::Diff) -> Vec<Specimen> {
    let mut out = Vec::new();
    for change in runtime.real_file_changes() {
        if change.kind == crate::diff::ChangeKind::Removed {
            continue;
        }
        let path = &change.path;
        let lower = path.to_ascii_lowercase();
        let mut hit: Option<(String, String)> = None; // (data folder, inner rel)
        for marker in [r"\appdata\roaming\", r"\appdata\local\", r"\documents\"] {
            if let Some(idx) = lower.find(marker) {
                let rest = &path[idx + marker.len()..];
                let mut parts = rest.splitn(3, '\\');
                let vendor = parts.next().unwrap_or("");
                let app = parts.next().unwrap_or("");
                let inner = parts.next().unwrap_or("");
                if !app.is_empty() {
                    let raw_folder = app
                        .chars()
                        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
                        .collect::<String>();
                    // Same PAL rule as the classifier: `settings` is
                    // reserved in Data (registry storage).
                    let folder = if raw_folder.eq_ignore_ascii_case("settings") {
                        format!("{raw_folder}Data")
                    } else {
                        raw_folder
                    };
                    let rel = if inner.is_empty() {
                        format!("{folder}\\{}", path.rsplit('\\').next().unwrap_or("file"))
                    } else {
                        format!("{folder}\\{inner}")
                    };
                    let _ = vendor;
                    hit = Some((folder, rel));
                    break;
                }
            }
        }
        if let Some((_folder, rel)) = hit {
            out.push(Specimen {
                sandbox_path: path.clone(),
                data_relative: rel.clone(),
                default_data_relative: rel,
            });
        }
    }
    out.sort_by(|a, b| a.data_relative.cmp(&b.data_relative));
    out.dedup_by(|a, b| a.data_relative == b.data_relative);
    out
}

// ------------------------------------------------------------ post-capture

/// What the host should do next, judged from the agent's state file.
/// The agent (already shipping as the `portableforge-agent` binary)
/// writes `state.json` into the exchange folder as it works; this is
/// the host half of the live wiring — the wizard polls this instead of
/// guessing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostAction {
    /// Nothing to do yet — the agent has not finished (or not started).
    Wait,
    /// The capture finished; run `post_capture`.
    Harvest,
    /// The agent reported a failure, in its own plain words.
    Failed(String),
}

pub fn next_host_action(state: Option<&Session>) -> HostAction {
    let Some(s) = state else {
        return HostAction::Wait;
    };
    if let Some(line) = s.log.iter().find(|l| l.starts_with("FAILED")) {
        return HostAction::Failed(line.clone());
    }
    match s.phase {
        Phase::Done => HostAction::Harvest,
        _ => HostAction::Wait,
    }
}

/// One point in the capture sequence. The host asks for a snapshot by
/// writing the point's command file into the exchange folder; the agent
/// inside the test box watches for it, takes the snapshot, and writes
/// the point's note file (the same names [`post_capture`] reads).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapturePoint {
    Before,
    AfterInstall,
    AfterRuntime,
}

impl CapturePoint {
    /// The note file the agent writes for this point.
    pub fn note_name(&self) -> &'static str {
        match self {
            CapturePoint::Before => "before.json",
            CapturePoint::AfterInstall => "after-install.json",
            CapturePoint::AfterRuntime => "after-runtime.json",
        }
    }

    /// The command file the host writes to request this point.
    pub fn command_name(&self) -> &'static str {
        match self {
            CapturePoint::Before => "cmd-before",
            CapturePoint::AfterInstall => "cmd-after-install",
            CapturePoint::AfterRuntime => "cmd-after-runtime",
        }
    }

    /// Snapshot label recorded inside the note.
    pub fn label(&self) -> &'static str {
        match self {
            CapturePoint::Before => "pre",
            CapturePoint::AfterInstall => "post-install",
            CapturePoint::AfterRuntime => "post-run",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "before" => Some(CapturePoint::Before),
            "after-install" => Some(CapturePoint::AfterInstall),
            "after-runtime" => Some(CapturePoint::AfterRuntime),
            _ => None,
        }
    }

    pub const ALL: &[CapturePoint] = &[
        CapturePoint::Before,
        CapturePoint::AfterInstall,
        CapturePoint::AfterRuntime,
    ];
}

/// Ask the agent in the test box to take one capture snapshot. The
/// request is a small file in the shared exchange folder — the only
/// channel the host and the agent share, and one that works the same
/// for a user-run VM (Mode B): put the command file in the folder the
/// guest can see.
pub fn request_snapshot(layout: &SessionLayout, point: CapturePoint) -> Result<(), String> {
    std::fs::create_dir_all(&layout.exchange)
        .map_err(|e| format!("We could not open the session folder: {e}"))?;
    std::fs::write(layout.exchange.join(point.command_name()), b"")
        .map_err(|e| format!("We could not ask the test box for its note: {e}"))?;
    Ok(())
}

/// Program details the install itself recorded (its uninstall
/// registration): the honest source for pre-filling the naming step,
/// instead of guessing from file names.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetadataHints {
    pub display_name: Option<String>,
    pub publisher: Option<String>,
    pub display_version: Option<String>,
}

impl MetadataHints {
    fn from_snapshot(snap: &Snapshot) -> Self {
        let mut hints = Self::default();
        for e in &snap.registry {
            if !e
                .key
                .to_ascii_lowercase()
                .contains(r"\microsoft\windows\currentversion\uninstall\")
            {
                continue;
            }
            // .reg data is the quoted textual form: `"Widget"`.
            let value = e.data.trim().trim_matches('"').to_string();
            if value.is_empty() {
                continue;
            }
            match e.name.as_str() {
                "DisplayName" => hints.display_name = Some(value),
                "Publisher" => hints.publisher = Some(value),
                "DisplayVersion" => hints.display_version = Some(value),
                _ => {}
            }
        }
        hints
    }
}

/// The result of reading a finished capture back from the exchange
/// folder: both change lists, the ready-made review for the "What it
/// left behind" step, and the first-run specimen plan.
#[derive(Debug, Clone, PartialEq)]
pub struct CaptureOutcome {
    pub install_diff: crate::diff::Diff,
    pub runtime_diff: crate::diff::Diff,
    pub review: crate::review::Review,
    pub specimens: Vec<Specimen>,
    pub hints: MetadataHints,
}

/// Read the three notes the agent leaves in the exchange folder
/// (`before.json`, `after-install.json`, `after-runtime.json` — each a
/// snapshot the agent's own commands produce) and turn them into the
/// review the wizard shows. A missing note is a plain error naming
/// exactly which one and which step makes it.
pub fn post_capture(layout: &SessionLayout) -> Result<CaptureOutcome, String> {
    fn load(layout: &SessionLayout, name: &str, step: &str) -> Result<Snapshot, String> {
        let path = layout.exchange.join(name);
        let text = std::fs::read_to_string(&path).map_err(|_| {
            format!(
                "The note {name} is missing from the session folder. It is made by the {step} step — that step may not have finished."
            )
        })?;
        Snapshot::from_json(&text)
    }
    let before = load(layout, "before.json", "clean test box")?;
    let after_install = load(layout, "after-install.json", "install")?;
    let after_runtime = load(layout, "after-runtime.json", "try it out")?;

    let install_diff = diff_snapshots(&before, &after_install);
    let runtime_diff = diff_snapshots(&after_install, &after_runtime);

    // Directories the install itself created (parents of added files),
    // which the classifier uses to tell program files from settings.
    let mut install_dirs: Vec<String> = Vec::new();
    for change in install_diff.real_file_changes() {
        if let Some(idx) = change.path.rfind('\\') {
            let dir = change.path[..idx].to_string();
            if !install_dirs.iter().any(|d| d.eq_ignore_ascii_case(&dir)) {
                install_dirs.push(dir);
            }
        }
    }

    let mut classifications =
        crate::classify::classify(&install_diff, &runtime_diff, &install_dirs);

    // Canary scan: text files the agent harvested may embed absolute
    // paths from inside the test box (its user name is the giveaway).
    // Those become FileWriteRewrite findings in the review.
    let harvest_dir = layout.exchange.join("harvest");
    if harvest_dir.is_dir() {
        let mut stack = vec![harvest_dir.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for item in entries.flatten() {
                let path = item.path();
                let Ok(kind) = item.file_type() else { continue };
                if kind.is_dir() {
                    stack.push(path);
                    continue;
                }
                if !kind.is_file() {
                    continue;
                }
                let ext = path
                    .extension()
                    .map(|e| e.to_string_lossy().to_ascii_lowercase())
                    .unwrap_or_default();
                if !matches!(
                    ext.as_str(),
                    "ini" | "xml" | "json" | "txt" | "cfg" | "conf"
                ) {
                    continue;
                }
                let Ok(bytes) = std::fs::read(&path) else {
                    continue;
                };
                if bytes.len() > 262_144 {
                    continue;
                }
                let Ok(text) = String::from_utf8(bytes) else {
                    continue;
                };
                let rel = path
                    .strip_prefix(&harvest_dir)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned();
                if let Some(c) = crate::classify::classify_file_content(&rel, &text) {
                    let already = classifications.iter().any(|x| x.finding == c.finding);
                    if !already {
                        classifications.push(c);
                    }
                }
            }
        }
    }

    let review = crate::review::Review::from_classifications(classifications);
    let specimens = specimen_plan(&runtime_diff);
    let hints = MetadataHints::from_snapshot(&after_install);

    // Save the artefacts next to the session record, so a run can be
    // inspected, resumed, or sent back without re-reading the notes.
    let _ = std::fs::write(layout.root.join("review.json"), review.to_json());
    if let Ok(json) = serde_json::to_string_pretty(&install_diff) {
        let _ = std::fs::write(layout.root.join("install-diff.json"), json);
    }
    if let Ok(json) = serde_json::to_string_pretty(&runtime_diff) {
        let _ = std::fs::write(layout.root.join("runtime-diff.json"), json);
    }
    if let Ok(json) = serde_json::to_string_pretty(&specimens) {
        let _ = std::fs::write(layout.root.join("specimens.json"), json);
    }

    Ok(CaptureOutcome {
        install_diff,
        runtime_diff,
        review,
        specimens,
        hints,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wsb::SandboxMode as Mode;

    fn facts_good() -> PreflightFacts {
        PreflightFacts {
            is_windows: true,
            os_build: 26100,
            edition_supported: true,
            sandbox_exe_present: true,
            feature_enabled: Some(true),
            free_disk_mb: Some(50_000),
            free_ram_mb: Some(8_000),
        }
    }

    #[test]
    fn good_machine_gets_sandbox_home_gets_honest_fallback() {
        let r = evaluate_preflight(&facts_good());
        assert!(r.sandbox_ready());
        assert_eq!(r.best_mode, Mode::WindowsSandbox);

        let mut home = facts_good();
        home.edition_supported = false;
        let r = evaluate_preflight(&home);
        assert!(!r.sandbox_ready());
        assert_eq!(r.best_mode, Mode::HostSnapshot);
        let failed: Vec<_> = r.checks.iter().filter(|c| !c.ok).collect();
        assert_eq!(failed.len(), 1);
        assert!(failed[0].fix.contains("Windows Home"));
    }

    #[test]
    fn session_layout_and_wsb_are_written() {
        let base = std::env::temp_dir().join(format!("pf-orch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let s = Session::new("widget-run", Mode::WindowsSandbox);
        let layout = prepare_session(&base, &s, None, false).unwrap();
        assert!(layout.input.is_dir() && layout.exchange.is_dir());
        let wsb = std::fs::read_to_string(&layout.wsb_path).unwrap();
        assert!(wsb.contains("portableforge-agent.exe run"));
        assert!(read_agent_state(&layout).is_none());
        let (prog, _arg) = sandbox_launch_command(&layout);
        assert_eq!(prog, "WindowsSandbox.exe");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn post_capture_builds_review_from_exchange_notes() {
        use crate::snapshot::{sha256_hex, FileEntry};
        let base = std::env::temp_dir().join(format!("pf-postcap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let layout = session_layout(&base, "s1");
        std::fs::create_dir_all(&layout.exchange).unwrap();
        let entry = |p: &str| FileEntry {
            path: p.into(),
            size: 3,
            sha256: sha256_hex(b"abc"),
            mtime_secs: 0,
        };
        let mut before = Snapshot::new("pre");
        before.files.push(entry(r"C:\Windows\notepad.exe"));
        let mut after_install = Snapshot::new("post-install");
        after_install.files.push(entry(r"C:\Windows\notepad.exe"));
        after_install
            .files
            .push(entry(r"C:\Program Files\Widget\widget.exe"));
        let mut after_runtime = after_install.clone();
        after_runtime.label = "post-run".into();
        after_runtime.files.push(entry(
            r"C:\Users\WDAGUtilityAccount\AppData\Roaming\Acme\Widget\settings.ini",
        ));
        for (name, snap) in [
            ("before.json", &before),
            ("after-install.json", &after_install),
            ("after-runtime.json", &after_runtime),
        ] {
            std::fs::write(layout.exchange.join(name), snap.to_json()).unwrap();
        }
        // Uninstall registration in the install note -> naming hints.
        after_install.registry.push(crate::snapshot::RegEntry {
            key: r"HKEY_LOCAL_MACHINE\Software\Microsoft\Windows\CurrentVersion\Uninstall\Widget"
                .into(),
            name: "DisplayName".into(),
            data: "\"Widget\"".into(),
        });
        after_install.registry.push(crate::snapshot::RegEntry {
            key: r"HKEY_LOCAL_MACHINE\Software\Microsoft\Windows\CurrentVersion\Uninstall\Widget"
                .into(),
            name: "DisplayVersion".into(),
            data: "\"1.2\"".into(),
        });
        std::fs::write(
            layout.exchange.join("after-install.json"),
            after_install.to_json(),
        )
        .unwrap();
        // A harvested settings file embedding the sandbox user name ->
        // a FileWriteRewrite finding from the canary scan.
        let harvest = layout.exchange.join("harvest");
        std::fs::create_dir_all(&harvest).unwrap();
        std::fs::write(
            harvest.join("settings.ini"),
            "home=C:\\Users\\WDAGUtilityAccount\\Widget\n",
        )
        .unwrap();
        let outcome = post_capture(&layout).unwrap();
        assert!(!outcome.review.items.is_empty(), "review should have cards");
        assert_eq!(outcome.specimens.len(), 1);
        assert_eq!(outcome.specimens[0].data_relative, r"Widget\settings.ini");
        assert_eq!(outcome.hints.display_name.as_deref(), Some("Widget"));
        assert_eq!(outcome.hints.display_version.as_deref(), Some("1.2"));
        assert!(outcome.review.items.iter().any(|i| matches!(
            i.classification.finding,
            crate::classify::Finding::FileWriteRewrite { .. }
        )));
        // Artefacts are saved into the session folder.
        assert!(layout.root.join("review.json").is_file());
        assert!(layout.root.join("install-diff.json").is_file());
        assert!(layout.root.join("specimens.json").is_file());
        let missing = post_capture(&session_layout(&base, "nope"));
        assert!(missing.unwrap_err().contains("before.json"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn snapshot_request_writes_the_agents_command_file() {
        let base = std::env::temp_dir().join(format!("pf-req-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let layout = session_layout(&base, "s1");
        request_snapshot(&layout, CapturePoint::AfterInstall).unwrap();
        assert!(layout.exchange.join("cmd-after-install").is_file());
        assert_eq!(
            CapturePoint::from_name("after-install"),
            Some(CapturePoint::AfterInstall)
        );
        assert_eq!(CapturePoint::AfterRuntime.note_name(), "after-runtime.json");
        assert!(CapturePoint::from_name("bogus").is_none());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn host_action_follows_agent_state() {
        assert_eq!(next_host_action(None), HostAction::Wait);
        let mut s = Session::new("x", Mode::WindowsSandbox);
        assert_eq!(next_host_action(Some(&s)), HostAction::Wait);
        s.phase = Phase::Done;
        assert_eq!(next_host_action(Some(&s)), HostAction::Harvest);
        s.log.push("FAILED: the setup file would not run".into());
        assert!(matches!(next_host_action(Some(&s)), HostAction::Failed(_)));
    }

    #[test]
    fn specimen_plan_maps_appdata_files_into_data() {
        use crate::diff::{ChangeKind, FileChange};
        use crate::snapshot::{sha256_hex, FileEntry};
        let mut d = crate::diff::Diff::default();
        d.files.push(FileChange {
            kind: ChangeKind::Added,
            path: r"C:\Users\WDAGUtilityAccount\AppData\Roaming\Acme\Widget\settings.ini".into(),
            before: None,
            after: Some(FileEntry {
                path: r"C:\Users\WDAGUtilityAccount\AppData\Roaming\Acme\Widget\settings.ini"
                    .into(),
                size: 1,
                sha256: sha256_hex(b"x"),
                mtime_secs: 0,
            }),
            noise: false,
        });
        d.files.push(FileChange {
            kind: ChangeKind::Added,
            path: r"C:\Windows\System32\acme.dll".into(),
            before: None,
            after: Some(FileEntry {
                path: r"C:\Windows\System32\acme.dll".into(),
                size: 1,
                sha256: sha256_hex(b"x"),
                mtime_secs: 0,
            }),
            noise: false,
        });
        let plan = specimen_plan(&d);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].data_relative, r"Widget\settings.ini");
        assert_eq!(plan[0].default_data_relative, r"Widget\settings.ini");
    }

    #[test]
    fn harvest_copies_tree_and_counts() {
        let base = std::env::temp_dir().join(format!("pf-harvest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let src = base.join("src");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("a.exe"), b"MZ1234").unwrap();
        std::fs::write(src.join("sub").join("b.dat"), b"xy").unwrap();
        let dest = base.join("dest");
        let r = harvest_tree(&src, &dest).unwrap();
        assert_eq!(r.files_copied, 2);
        assert_eq!(r.bytes_copied, 8);
        assert_eq!(
            std::fs::read(dest.join("sub").join("b.dat")).unwrap(),
            b"xy"
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
