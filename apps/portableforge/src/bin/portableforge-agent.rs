//! PortableForge agent — the Rust program that runs *inside* the safe
//! test box (or a guest VM). 100% Rust; it never uses scripts.
//!
//! Commands:
//!   run --exchange <folder> [--once] [--root <folder>]...
//!       Writes heartbeat state into the exchange folder and waits for
//!       command files there — the channel the host wizard drives:
//!       `cmd-before`, `cmd-after-install` and `cmd-after-runtime` each
//!       ask for one system snapshot (written back as `before.json`,
//!       `after-install.json`, `after-runtime.json`); `quit` ends the
//!       run. After the runtime snapshot the agent also harvests the
//!       installed program's folder into `exchange\harvest\`.
//!   capture --exchange <folder> --name <note.json> [--root <folder>]...
//!       Take one system snapshot right now (used for a leak-test box
//!       and for Mode B, where the user runs the agent by hand).
//!   snapshot --root <folder> --prefix <windows path> --out <file.json>
//!   snapshot-reg --reg-file <export.reg> --out <file.json>
//!   On Windows, `capture-reg --out <file.json>` exports the real
//!   registry itself (HKCU/HKLM Software + Uninstall hives) via the
//!   operating system's own `reg.exe` and parses the result.
//!
//! A system snapshot scans the folders where programs and their
//! settings actually land (Program Files, ProgramData, the user
//! profile) — never the Windows folder itself, which is operating
//! system housekeeping — and, on Windows, merges a registry capture.

use portableforge_core::orchestrate::{harvest_tree, CapturePoint};
use portableforge_core::progress::Progress;
use portableforge_core::session::{Phase, Session};
use portableforge_core::snapshot::{scan_roots, scan_tree, Snapshot};
use portableforge_core::wsb::SandboxMode;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn flag(args: &[String], name: &str) -> Option<String> {
    args.windows(2).find(|w| w[0] == name).map(|w| w[1].clone())
}

fn need(args: &[String], name: &str) -> Result<String, String> {
    flag(args, name).ok_or_else(|| format!("Missing {name}."))
}

/// Every value of a repeated flag, in order.
fn flag_all(args: &[String], name: &str) -> Vec<String> {
    args.windows(2)
        .filter(|w| w[0] == name)
        .map(|w| w[1].clone())
        .collect()
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(|s| s.as_str()) {
        Some("run") => cmd_run(&args[1..]),
        Some("capture") => cmd_capture(&args[1..]),
        Some("snapshot") => cmd_snapshot(&args[1..]),
        Some("snapshot-reg") => cmd_snapshot_reg(&args[1..]),
        Some("capture-reg") => cmd_capture_reg(&args[1..]),
        _ => Err("Commands: run | capture | snapshot | snapshot-reg | capture-reg".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

/// The folders a system snapshot walks, as `(folder, recorded prefix)`.
/// Extra `--root` folders are added (their own path is the prefix) —
/// that is how tests and Mode B point the agent at other locations.
fn snapshot_roots(extra: &[String]) -> Vec<(PathBuf, String)> {
    let mut roots: Vec<(PathBuf, String)> = Vec::new();
    #[cfg(windows)]
    {
        for dir in [
            r"C:\Program Files",
            r"C:\Program Files (x86)",
            r"C:\ProgramData",
            r"C:\Users",
        ] {
            roots.push((PathBuf::from(dir), dir.to_string()));
        }
    }
    for dir in extra {
        roots.push((PathBuf::from(dir), dir.clone()));
    }
    roots
}

/// One system snapshot: files under every snapshot root plus, on
/// Windows, the registry capture. Missing roots are skipped.
fn system_snapshot(label: &str, extra_roots: &[String]) -> Result<Snapshot, String> {
    // `mut` is used on Windows, where the registry capture is merged in.
    #[allow(unused_mut)]
    let mut snap =
        scan_roots(&snapshot_roots(extra_roots), label).map_err(|e| format!("Scan failed: {e}"))?;
    #[cfg(windows)]
    {
        snap.registry = capture_registry_entries()?;
    }
    Ok(snap)
}

fn cmd_capture(args: &[String]) -> Result<(), String> {
    let exchange = PathBuf::from(need(args, "--exchange")?);
    let name = need(args, "--name")?;
    std::fs::create_dir_all(&exchange)
        .map_err(|e| format!("Cannot open the exchange folder: {e}"))?;
    let snap = system_snapshot("capture", &flag_all(args, "--root"))?;
    let out = exchange.join(&name);
    std::fs::write(&out, snap.to_json())
        .map_err(|e| format!("Cannot write {}: {e}", out.display()))?;
    println!(
        "{} files, {} settings -> {}",
        snap.files.len(),
        snap.registry.len(),
        out.display()
    );
    Ok(())
}

/// Copy the installed program's own folder into `exchange\harvest\`,
/// so the host build step has the real program files to package. The
/// install folder is the one under Program Files that gained the most
/// files between the clean note and the runtime note; when there is no
/// such folder (the program installed elsewhere) nothing is harvested
/// and the state log says so plainly.
fn harvest_install_tree(exchange: &Path, before: &Snapshot, after: &Snapshot) -> String {
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
    counts.sort_by_key(|a| std::cmp::Reverse(a.1));
    let Some((dir, _)) = counts.first() else {
        return "No program folder was found to copy out — the build step will ask for the files."
            .into();
    };
    let source = PathBuf::from(dir);
    if !source.is_dir() {
        return format!("The program folder {dir} could not be read, so nothing was copied out.");
    }
    match harvest_tree(&source, &exchange.join("harvest")) {
        Ok(r) => format!(
            "Copied {} program files out of the test box ({} bytes).",
            r.files_copied, r.bytes_copied
        ),
        Err(e) => format!("Copying the program files out failed: {e}"),
    }
}

/// Take the snapshot one capture point asks for and write its note.
/// After the runtime note, also harvest the program files and mark
/// the session Done — that is the host's signal to read the notes.
fn take_capture(
    exchange: &Path,
    point: CapturePoint,
    extra_roots: &[String],
    session: &mut Session,
) -> Result<(), String> {
    let snap = system_snapshot(point.label(), extra_roots)?;
    if point == CapturePoint::AfterRuntime {
        let before = std::fs::read_to_string(exchange.join(CapturePoint::Before.note_name()))
            .ok()
            .and_then(|t| Snapshot::from_json(&t).ok())
            .unwrap_or_default();
        let line = harvest_install_tree(exchange, &before, &snap);
        session.say(line);
    }
    let out = exchange.join(point.note_name());
    std::fs::write(&out, snap.to_json())
        .map_err(|e| format!("Cannot write {}: {e}", out.display()))?;
    session.say(format!(
        "Took the {} note ({} files, {} settings).",
        point.label(),
        snap.files.len(),
        snap.registry.len()
    ));
    session.phase = match point {
        CapturePoint::Before => Phase::PreScan,
        CapturePoint::AfterInstall => Phase::Install,
        CapturePoint::AfterRuntime => Phase::Done,
    };
    Ok(())
}

fn cmd_run(args: &[String]) -> Result<(), String> {
    let exchange = PathBuf::from(need(args, "--exchange")?);
    std::fs::create_dir_all(&exchange)
        .map_err(|e| format!("Cannot open the exchange folder: {e}"))?;
    let extra_roots = flag_all(args, "--root");
    let once = args.iter().any(|a| a == "--once");
    let state_path = exchange.join("state.json");
    let mut session = Session::new("in-box", SandboxMode::WindowsSandbox);
    loop {
        // Serve any capture the host has asked for, in sequence order.
        for point in CapturePoint::ALL {
            let cmd = exchange.join(point.command_name());
            if cmd.exists() {
                let _ = std::fs::remove_file(&cmd);
                if let Err(e) = take_capture(&exchange, *point, &extra_roots, &mut session) {
                    session.say(format!("FAILED: {e}"));
                }
            }
        }
        session.agent_heartbeat_secs = Some(now_secs());
        std::fs::write(&state_path, session.to_json())
            .map_err(|e| format!("Cannot write heartbeat: {e}"))?;
        if once || exchange.join("quit").exists() {
            break;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    Ok(())
}

fn cmd_snapshot(args: &[String]) -> Result<(), String> {
    let root = need(args, "--root")?;
    let prefix = need(args, "--prefix")?;
    let out = need(args, "--out")?;
    let label = flag(args, "--label").unwrap_or_else(|| "snapshot".into());
    let mut progress = Progress::new("scan", None);
    let files = scan_tree(Path::new(&root), &prefix, &mut progress)
        .map_err(|e| format!("Scan failed: {e}"))?;
    let mut snap = Snapshot::new(label);
    snap.files = files;
    std::fs::write(&out, snap.to_json()).map_err(|e| format!("Cannot write {out}: {e}"))?;
    println!("{} files -> {out}", snap.files.len());
    Ok(())
}

fn cmd_snapshot_reg(args: &[String]) -> Result<(), String> {
    let file = need(args, "--reg-file")?;
    let out = need(args, "--out")?;
    let text = read_reg_export(Path::new(&file))?;
    let (entries, skipped) = portableforge_core::regfile::parse_reg_with_report(&text);
    let mut snap = Snapshot::new("snapshot");
    snap.registry = entries;
    std::fs::write(&out, snap.to_json()).map_err(|e| format!("Cannot write {out}: {e}"))?;
    println!(
        "{} entries ({skipped} skipped) -> {out}",
        snap.registry.len()
    );
    Ok(())
}

/// `.reg` exports are UTF-16LE with a BOM when produced by `reg.exe`;
/// accept that and plain UTF-8.
fn read_reg_export(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16(&units).map_err(|e| format!("Bad UTF-16 export: {e}"));
    }
    String::from_utf8(bytes).map_err(|e| format!("Bad export text: {e}"))
}

/// Export the real registry hives with `reg.exe` and parse them.
/// A hive that cannot be exported is skipped, never fatal — the
/// snapshot still carries the ones that worked.
#[cfg(windows)]
fn capture_registry_entries() -> Result<Vec<portableforge_core::snapshot::RegEntry>, String> {
    let tmp = std::env::temp_dir().join("portableforge-reg");
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let hives = [
        ("HKCU\\Software", "hkcu.reg"),
        ("HKLM\\Software", "hklm.reg"),
        (
            "HKLM\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
            "uninstall.reg",
        ),
    ];
    let mut all = Vec::new();
    for (hive, name) in hives {
        let file = tmp.join(name);
        let status = std::process::Command::new("reg.exe")
            .args(["export", hive, &file.to_string_lossy(), "/y"])
            .status()
            .map_err(|e| format!("Could not run reg.exe: {e}"))?;
        if status.success() {
            let text = read_reg_export(&file)?;
            all.extend(portableforge_core::regfile::parse_reg(&text));
        }
    }
    Ok(all)
}

#[cfg(windows)]
fn cmd_capture_reg(args: &[String]) -> Result<(), String> {
    let out = need(args, "--out")?;
    let mut snap = Snapshot::new("snapshot");
    snap.registry = capture_registry_entries()?;
    std::fs::write(&out, snap.to_json()).map_err(|e| format!("Cannot write {out}: {e}"))?;
    println!("{} entries -> {out}", snap.registry.len());
    Ok(())
}

#[cfg(not(windows))]
fn cmd_capture_reg(_args: &[String]) -> Result<(), String> {
    Err("capture-reg runs on Windows (inside the test box). On this system, use snapshot-reg with an exported file.".into())
}
