//! PortableForge agent — the Rust program that runs *inside* the safe
//! test box (or a guest VM). 100% Rust; it never uses scripts.
//!
//! Commands:
//!   run --exchange <folder> [--once]
//!       Writes heartbeat state into the exchange folder and waits for
//!       a `quit` file there — the channel the host wizard drives.
//!   snapshot --root <folder> --prefix <windows path> --out <file.json>
//!   snapshot-reg --reg-file <export.reg> --out <file.json>
//!   On Windows, `capture-reg --out <file.json>` exports the real
//!   registry itself (HKCU/HKLM Software + Uninstall hives) via the
//!   operating system's own `reg.exe` and parses the result.

use portableforge_core::progress::Progress;
use portableforge_core::session::Session;
use portableforge_core::snapshot::{scan_tree, Snapshot};
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
        Some("snapshot") => cmd_snapshot(&args[1..]),
        Some("snapshot-reg") => cmd_snapshot_reg(&args[1..]),
        Some("capture-reg") => cmd_capture_reg(&args[1..]),
        _ => Err("Commands: run | snapshot | snapshot-reg | capture-reg".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_run(args: &[String]) -> Result<(), String> {
    let exchange = PathBuf::from(need(args, "--exchange")?);
    std::fs::create_dir_all(&exchange)
        .map_err(|e| format!("Cannot open the exchange folder: {e}"))?;
    let once = args.iter().any(|a| a == "--once");
    let state_path = exchange.join("state.json");
    let mut session = Session::new("in-box", SandboxMode::WindowsSandbox);
    loop {
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

#[cfg(windows)]
fn cmd_capture_reg(args: &[String]) -> Result<(), String> {
    let out = need(args, "--out")?;
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
    let mut snap = Snapshot::new("snapshot");
    snap.registry = all;
    std::fs::write(&out, snap.to_json()).map_err(|e| format!("Cannot write {out}: {e}"))?;
    println!("{} entries -> {out}", snap.registry.len());
    Ok(())
}

#[cfg(not(windows))]
fn cmd_capture_reg(_args: &[String]) -> Result<(), String> {
    Err("capture-reg runs on Windows (inside the test box). On this system, use snapshot-reg with an exported file.".into())
}
