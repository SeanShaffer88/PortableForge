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
//!   watch --exchange <folder> [--root <folder>]... [--quiet <secs>]
//!         [--name <note.json>] [--timeout <secs>]
//!       Wait until the scanned folders have stopped changing for
//!       `--quiet` seconds, then take the snapshot — the hands-free
//!       way to get the after-install note: start it, run the setup
//!       program, and the note takes itself when things settle.
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
use portableforge_core::session::{write_file_atomic, Phase, Session};
#[cfg(windows)]
use portableforge_core::snapshot::parse_service_names;
use portableforge_core::snapshot::{
    scan_roots_full, scan_tree, tree_fingerprint, HashCache, Snapshot,
};
use portableforge_core::wsb::SandboxMode;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
        Some("watch") => cmd_watch(&args[1..]),
        Some("snapshot") => cmd_snapshot(&args[1..]),
        Some("snapshot-reg") => cmd_snapshot_reg(&args[1..]),
        Some("capture-reg") => cmd_capture_reg(&args[1..]),
        _ => Err("Commands: run | capture | watch | snapshot | snapshot-reg | capture-reg".into()),
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
/// Windows, the registry capture and the service list. Missing roots
/// are skipped; unreadable files are counted in the snapshot, never
/// fatal. When `cache_dir` is given, a hash cache lives there so
/// later notes skip re-reading unchanged files. `on_progress` hears
/// `(folder being scanned, files recorded so far)` as the scan runs.
fn system_snapshot(
    label: &str,
    extra_roots: &[String],
    cache_dir: Option<&Path>,
    on_progress: &mut dyn FnMut(&str, u64),
) -> Result<Snapshot, String> {
    let cache = cache_dir.map(|dir| HashCache::load(&dir.join("hash-cache.json")));
    let (mut snap, fresh) = scan_roots_full(
        &snapshot_roots(extra_roots),
        label,
        cache.as_ref(),
        on_progress,
    )
    .map_err(|e| format!("Scan failed: {e}"))?;
    if let Some(dir) = cache_dir {
        let _ = write_file_atomic(&dir.join("hash-cache.json"), &fresh.to_json());
    }
    if snap.skipped_files > 0 {
        snap.capture_warnings.push(format!(
            "{} files could not be read and were left out of this note.",
            snap.skipped_files
        ));
    }
    #[cfg(windows)]
    {
        let (entries, warnings) = capture_registry_entries();
        snap.registry = entries;
        snap.capture_warnings.extend(warnings);
        snap.services = capture_service_names();
    }
    Ok(snap)
}

fn cmd_capture(args: &[String]) -> Result<(), String> {
    let exchange = PathBuf::from(need(args, "--exchange")?);
    let name = need(args, "--name")?;
    std::fs::create_dir_all(&exchange)
        .map_err(|e| format!("Cannot open the exchange folder: {e}"))?;
    let snap = system_snapshot(
        "capture",
        &flag_all(args, "--root"),
        Some(&exchange),
        &mut |_, _| {},
    )?;
    for w in &snap.capture_warnings {
        eprintln!("{w}");
    }
    let out = exchange.join(&name);
    write_file_atomic(&out, &snap.to_json())
        .map_err(|e| format!("Cannot write {}: {e}", out.display()))?;
    println!(
        "{} files, {} settings -> {}",
        snap.files.len(),
        snap.registry.len(),
        out.display()
    );
    Ok(())
}

/// Fingerprint every snapshot root (cheap: names, sizes, times — no
/// file contents), one combined list.
fn combined_fingerprint(extra_roots: &[String]) -> Vec<(String, u64, i64)> {
    let mut out = Vec::new();
    for (root, _) in snapshot_roots(extra_roots) {
        if root.is_dir() {
            for (rel, size, mtime) in tree_fingerprint(&root) {
                out.push((format!("{}\u{1}{rel}", root.display()), size, mtime));
            }
        }
    }
    out.sort();
    out
}

fn cmd_watch(args: &[String]) -> Result<(), String> {
    let exchange = PathBuf::from(need(args, "--exchange")?);
    std::fs::create_dir_all(&exchange)
        .map_err(|e| format!("Cannot open the exchange folder: {e}"))?;
    let extra_roots = flag_all(args, "--root");
    let quiet_secs: u64 = flag(args, "--quiet")
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);
    let timeout_secs: u64 = flag(args, "--timeout")
        .and_then(|v| v.parse().ok())
        .unwrap_or(3600);
    let poll_secs: u64 = flag(args, "--poll")
        .and_then(|v| v.parse().ok())
        .unwrap_or(2)
        .max(1);
    let name = flag(args, "--name").unwrap_or_else(|| "after-install.json".into());
    let started = Instant::now();
    let mut last = combined_fingerprint(&extra_roots);
    let mut last_change = Instant::now();
    println!("Watching for things to settle (quiet for {quiet_secs}s)…");
    loop {
        std::thread::sleep(Duration::from_secs(poll_secs));
        let now_fp = combined_fingerprint(&extra_roots);
        if now_fp != last {
            last = now_fp;
            last_change = Instant::now();
        }
        if last_change.elapsed() >= Duration::from_secs(quiet_secs) {
            let snap = system_snapshot("watch", &extra_roots, Some(&exchange), &mut |_, _| {})?;
            for w in &snap.capture_warnings {
                eprintln!("{w}");
            }
            let out = exchange.join(&name);
            write_file_atomic(&out, &snap.to_json())
                .map_err(|e| format!("Cannot write {}: {e}", out.display()))?;
            println!(
                "Settled. {} files, {} settings -> {}",
                snap.files.len(),
                snap.registry.len(),
                out.display()
            );
            return Ok(());
        }
        if started.elapsed() >= Duration::from_secs(timeout_secs) {
            return Err(format!(
                "Things never settled within {timeout_secs} seconds, so no note was taken."
            ));
        }
    }
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
    // While the scan runs, the heartbeat file keeps moving and says
    // what is being looked through, so a long scan never looks like
    // a hang on the host side. The hook throttles itself to one
    // state write every couple of seconds.
    let state_path = exchange.join("state.json");
    let snap = {
        let mut last_write: Option<Instant> = None;
        let mut hook = |root: &str, count: u64| {
            if let Some(t) = last_write {
                if t.elapsed() < Duration::from_secs(2) {
                    return;
                }
            }
            last_write = Some(Instant::now());
            let mut s = session.clone();
            s.agent_heartbeat_secs = Some(now_secs());
            s.agent_action = Some(format!(
                "Taking the {} note — looking through {root} ({count} files checked so far)",
                point.label()
            ));
            let _ = write_file_atomic(&state_path, &s.to_json());
        };
        system_snapshot(point.label(), extra_roots, Some(exchange), &mut hook)?
    };
    session.agent_action = None;
    for w in &snap.capture_warnings {
        session.say(w.clone());
    }
    if point == CapturePoint::AfterRuntime {
        let before = std::fs::read_to_string(exchange.join(CapturePoint::Before.note_name()))
            .ok()
            .and_then(|t| Snapshot::from_json(&t).ok())
            .unwrap_or_default();
        let line = harvest_install_tree(exchange, &before, &snap);
        session.say(line);
    }
    let out = exchange.join(point.note_name());
    write_file_atomic(&out, &snap.to_json())
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
        write_file_atomic(&state_path, &session.to_json())
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
    write_file_atomic(Path::new(&out), &snap.to_json())
        .map_err(|e| format!("Cannot write {out}: {e}"))?;
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
    write_file_atomic(Path::new(&out), &snap.to_json())
        .map_err(|e| format!("Cannot write {out}: {e}"))?;
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

/// One hive export attempt: run `reg.exe`, read the export, parse it.
#[cfg(windows)]
fn export_hive(
    hive: &str,
    file: &Path,
) -> Result<Vec<portableforge_core::snapshot::RegEntry>, String> {
    let status = std::process::Command::new("reg.exe")
        .args(["export", hive, &file.to_string_lossy(), "/y"])
        .status()
        .map_err(|e| format!("Could not run reg.exe: {e}"))?;
    if !status.success() {
        return Err(format!("reg.exe export of {hive} failed"));
    }
    let text = read_reg_export(file)?;
    Ok(portableforge_core::regfile::parse_reg(&text))
}

/// Export the real registry hives with `reg.exe` and parse them.
/// A hive that cannot be exported is retried once, then skipped —
/// never fatal — and the returned warnings name each hive that was
/// left out, so the gap is visible instead of silent.
#[cfg(windows)]
fn capture_registry_entries() -> (Vec<portableforge_core::snapshot::RegEntry>, Vec<String>) {
    let tmp = std::env::temp_dir().join("portableforge-reg");
    if std::fs::create_dir_all(&tmp).is_err() {
        return (
            Vec::new(),
            vec!["The remembered settings could not be read at all.".into()],
        );
    }
    let hives = [
        ("HKCU\\Software", "hkcu.reg"),
        ("HKLM\\Software", "hklm.reg"),
        (
            "HKLM\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
            "uninstall.reg",
        ),
    ];
    let mut all = Vec::new();
    let mut warnings = Vec::new();
    for (hive, name) in hives {
        let file = tmp.join(name);
        match export_hive(hive, &file).or_else(|_| export_hive(hive, &file)) {
            Ok(entries) => all.extend(entries),
            Err(_) => warnings.push(format!(
                "The {hive} part of the remembered settings could not be read, so it was left out of this note."
            )),
        }
    }
    (all, warnings)
}

/// The machine's program services, from the operating system's own
/// `sc.exe`. Any failure simply means an empty list — services are
/// one signal among several, never a reason to fail a capture.
#[cfg(windows)]
fn capture_service_names() -> Vec<String> {
    let output = std::process::Command::new("sc.exe")
        .args(["query", "type=", "service", "state=", "all"])
        .output();
    match output {
        Ok(out) if out.status.success() => {
            parse_service_names(&String::from_utf8_lossy(&out.stdout))
        }
        _ => Vec::new(),
    }
}

#[cfg(windows)]
fn cmd_capture_reg(args: &[String]) -> Result<(), String> {
    let out = need(args, "--out")?;
    let mut snap = Snapshot::new("snapshot");
    let (entries, warnings) = capture_registry_entries();
    snap.registry = entries;
    snap.capture_warnings = warnings;
    for w in &snap.capture_warnings {
        eprintln!("{w}");
    }
    std::fs::write(&out, snap.to_json()).map_err(|e| format!("Cannot write {out}: {e}"))?;
    println!("{} entries -> {out}", snap.registry.len());
    Ok(())
}

#[cfg(not(windows))]
fn cmd_capture_reg(_args: &[String]) -> Result<(), String> {
    Err("capture-reg runs on Windows (inside the test box). On this system, use snapshot-reg with an exported file.".into())
}
