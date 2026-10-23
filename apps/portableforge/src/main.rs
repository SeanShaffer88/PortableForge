//! PortableForge host program.
//!
//! No arguments: opens the guided wizard (dark egui).
//! With arguments: a small CLI over the same core library, so every
//! wizard action is scriptable and testable. All logic lives in
//! `portableforge-core`; this binary only parses, calls, and prints.

mod wizard;

use portableforge_core::classify::classify;
use portableforge_core::diff::diff_snapshots;
use portableforge_core::paf::PafMetadata;
use portableforge_core::progress::Progress;
use portableforge_core::snapshot::{scan_tree, Snapshot};
use portableforge_core::validate::{generate_tree, has_errors, validate_metadata, validate_tree};
use portableforge_core::wsb::{SandboxConfig, SandboxMode};
use portableforge_core::Session;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        return match wizard::run_gui() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("The window could not be opened: {e}");
                ExitCode::FAILURE
            }
        };
    }
    match run_cli(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

/// Collect preflight facts about this machine. On Windows this reads
/// the real edition/build from the registry (via the operating system's
/// own `reg.exe`, invoked from Rust) and checks for the Sandbox program;
/// elsewhere it honestly reports "not Windows".
pub fn collect_preflight_facts() -> portableforge_core::orchestrate::PreflightFacts {
    #[cfg(windows)]
    {
        fn reg_value(value: &str) -> Option<String> {
            let out = std::process::Command::new("reg.exe")
                .args([
                    "query",
                    r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion",
                    "/v",
                    value,
                ])
                .output()
                .ok()?;
            let text = String::from_utf8_lossy(&out.stdout).to_string();
            // Lines look like: `    CurrentBuildNumber    REG_SZ    26100`
            for line in text.lines() {
                if line.contains(value) {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if let Some(last) = parts.last() {
                        return Some(last.to_string());
                    }
                }
            }
            None
        }
        let build = reg_value("CurrentBuildNumber")
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
        let edition = reg_value("EditionID").unwrap_or_default();
        let edition_supported = matches!(
            edition.as_str(),
            "Professional" | "Enterprise" | "Education" | "ProfessionalWorkstation" | "ServerRdsh"
        ) || edition.contains("Enterprise")
            || edition.contains("Education");
        let sandbox_exe_present =
            std::path::Path::new(r"C:\Windows\System32\WindowsSandbox.exe").exists();
        portableforge_core::orchestrate::PreflightFacts {
            is_windows: true,
            os_build: build,
            edition_supported,
            sandbox_exe_present,
            feature_enabled: None,
            free_disk_mb: None,
            free_ram_mb: None,
        }
    }
    #[cfg(not(windows))]
    {
        portableforge_core::orchestrate::PreflightFacts {
            is_windows: false,
            os_build: 0,
            edition_supported: false,
            sandbox_exe_present: false,
            feature_enabled: None,
            free_disk_mb: None,
            free_ram_mb: None,
        }
    }
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.windows(2).find(|w| w[0] == name).map(|w| w[1].clone())
}

fn need_flag(args: &[String], name: &str) -> Result<String, String> {
    flag(args, name).ok_or_else(|| format!("Missing {name} — see `portableforge help`."))
}

const HELP: &str = "\
PortableForge — make a portable copy of a program, guided step by step.

Just run `portableforge` with no commands to open the guided wizard.

Commands (the same steps, for scripts and testing):
  guide                                   Print the guided steps in plain words
  snapshot <folder> --prefix <path> --out <file.json> [--label <name>]
  parse-reg <file.reg> --out <file.json> [--label <name>]
  diff <before.json> <after.json> --out <file.json>
  classify <install-diff.json> <runtime-diff.json>
  wsb --input <folder> --tools <folder> --exchange <folder> --out <file.wsb> [--online]
  generate <details.json> --out <folder>
  validate <details.json> [--tree <folder>]
  session-new --id <name> --out <file.json>
  preflight                               Check whether this computer can run the safe test box
  tools [--cache <folder>]              List the pinned PortableApps.com builder tools and check any found in the cache folder
  specimens <runtime-diff.json>         List first-run files that would seed the package's starting settings
  leaks <package-folder-in-box> <before.json> <after.json>   Judge a finished-copy test run
  rollback-plan <diff.json>             List how a Careful-mode run would be put back

Everywhere: paths are ordinary file paths; nothing is uploaded anywhere.";

fn run_cli(args: &[String]) -> Result<(), String> {
    match args[0].as_str() {
        "help" | "--help" | "-h" => {
            println!("{HELP}");
            Ok(())
        }
        "guide" => {
            for (i, step) in portableforge_core::guide::STEPS.iter().enumerate() {
                println!("{:>2}. {}", i + 1, step.title);
                println!("    {}", step.what_happens);
                println!("    You: {}", step.what_you_do);
            }
            Ok(())
        }
        "snapshot" => {
            let root = args.get(1).ok_or("Give the folder to look through.")?;
            let prefix = need_flag(args, "--prefix")?;
            let out = need_flag(args, "--out")?;
            let label = flag(args, "--label").unwrap_or_else(|| "snapshot".into());
            let mut progress = Progress::new("Looking through the folder", None);
            let files = scan_tree(Path::new(root), &prefix, &mut progress)
                .map_err(|e| format!("We could not look through that folder: {e}"))?;
            let mut snap = Snapshot::new(label);
            snap.files = files;
            std::fs::write(&out, snap.to_json())
                .map_err(|e| format!("We could not save {out}: {e}"))?;
            println!("Saved {} files to {out}.", snap.files.len());
            Ok(())
        }
        "parse-reg" => {
            let file = args.get(1).ok_or("Give the .reg file to read.")?;
            let out = need_flag(args, "--out")?;
            let label = flag(args, "--label").unwrap_or_else(|| "snapshot".into());
            let text = std::fs::read_to_string(file)
                .map_err(|e| format!("We could not read {file}: {e}"))?;
            // .reg exports are UTF-16 when produced by reg.exe; the agent
            // converts before calling this. Here we accept UTF-8 text.
            let (entries, skipped) = portableforge_core::regfile::parse_reg_with_report(&text);
            let mut snap = Snapshot::new(label);
            snap.registry = entries;
            std::fs::write(&out, snap.to_json())
                .map_err(|e| format!("We could not save {out}: {e}"))?;
            println!(
                "Saved {} entries to {out} ({} lines skipped).",
                snap.registry.len(),
                skipped
            );
            Ok(())
        }
        "diff" => {
            let before = read_snapshot(args.get(1))?;
            let after = read_snapshot(args.get(2))?;
            let out = need_flag(args, "--out")?;
            let d = diff_snapshots(&before, &after);
            let json = serde_json::to_string_pretty(&d).map_err(|e| e.to_string())?;
            std::fs::write(&out, json).map_err(|e| format!("We could not save {out}: {e}"))?;
            println!(
                "Files changed: {}, settings changed: {}, new background helpers: {}.",
                d.real_file_changes().count(),
                d.real_reg_changes().count(),
                d.services_added.len()
            );
            Ok(())
        }
        "classify" => {
            let install = read_diff(args.get(1))?;
            let runtime = read_diff(args.get(2))?;
            let findings = classify(&install, &runtime, &[]);
            for c in &findings {
                let (title, body) = portableforge_core::guide::plain_card(&c.finding);
                println!("- {title}: {body}");
            }
            println!("{} things found.", findings.len());
            Ok(())
        }
        "wsb" => {
            let mut cfg = SandboxConfig {
                input_dir: need_flag(args, "--input")?,
                tools_dir: need_flag(args, "--tools")?,
                exchange_dir: need_flag(args, "--exchange")?,
                ..Default::default()
            };
            cfg.networking = args.iter().any(|a| a == "--online");
            cfg.check()?;
            let out = need_flag(args, "--out")?;
            std::fs::write(&out, cfg.to_wsb())
                .map_err(|e| format!("We could not save {out}: {e}"))?;
            println!("Saved the test box settings to {out}.");
            Ok(())
        }
        "generate" => {
            let meta = read_metadata(args.get(1))?;
            let out = need_flag(args, "--out")?;
            let written = generate_tree(Path::new(&out), &meta, &[])?;
            println!("Made {} files in {out}.", written.len());
            Ok(())
        }
        "validate" => {
            let meta = read_metadata(args.get(1))?;
            let mut issues = validate_metadata(&meta);
            if let Some(tree) = flag(args, "--tree") {
                issues.extend(validate_tree(Path::new(&tree), &meta));
            }
            for i in &issues {
                println!(
                    "{} {}: {}",
                    if i.error { "Problem" } else { "Note" },
                    i.field,
                    i.message
                );
            }
            if has_errors(&issues) {
                Err("There are problems to fix first — see above.".into())
            } else {
                println!("Everything checks out.");
                Ok(())
            }
        }
        "preflight" => {
            let report =
                portableforge_core::orchestrate::evaluate_preflight(&collect_preflight_facts());
            for c in &report.checks {
                println!("{} {}", if c.ok { "OK  " } else { "MISS" }, c.label);
                if !c.fix.is_empty() {
                    println!("     Fix: {}", c.fix);
                }
            }
            let (name, honesty) = report.best_mode.describe();
            println!("Best available way: {name} — {honesty}");
            Ok(())
        }
        "tools" => {
            let cache = flag(args, "--cache").map(std::path::PathBuf::from);
            for pin in portableforge_core::tools::TOOL_PINS {
                println!(
                    "{} {} — {} ({} bytes)",
                    pin.plain_name, pin.version, pin.filename, pin.size_bytes
                );
                if let Some(dir) = &cache {
                    match portableforge_core::tools::find_verified_tool(dir, pin) {
                        Some(p) => println!("     Ready and checked: {}", p.display()),
                        None => println!(
                            "     Not in the cache folder yet. It can be downloaded from {}",
                            pin.download_url
                        ),
                    }
                }
            }
            Ok(())
        }
        "specimens" => {
            let d = read_diff(args.get(1))?;
            let plan = portableforge_core::orchestrate::specimen_plan(&d);
            for s in &plan {
                println!("{}  ->  Data\\{}", s.sandbox_path, s.data_relative);
            }
            println!("{} first-run files.", plan.len());
            Ok(())
        }
        "leaks" => {
            let root = args
                .get(1)
                .ok_or("Give the package folder used in the test box.")?;
            let before = read_snapshot(args.get(2))?;
            let after = read_snapshot(args.get(3))?;
            let d = diff_snapshots(&before, &after);
            let report = portableforge_core::leaktest::analyse_leaks(root, &before, &after, &d);
            println!("{}", report.verdict());
            for leak in &report.leaks {
                println!("- {}", leak.plain);
            }
            Ok(())
        }
        "rollback-plan" => {
            let d = read_diff(args.get(1))?;
            let plan = portableforge_core::rollback::rollback_plan(&d);
            for a in &plan {
                println!("- {}", a.plain());
            }
            println!("{} steps to put things back.", plan.len());
            Ok(())
        }
        "post-capture" => {
            let dir = args
                .get(1)
                .ok_or("Give the session folder (the one containing exchange\\).")?;
            let root = std::path::PathBuf::from(dir);
            let layout = portableforge_core::orchestrate::SessionLayout {
                wsb_path: root.join("box.wsb"),
                session_path: root.join("session.json"),
                input: root.join("input"),
                tools: root.join("tools"),
                exchange: root.join("exchange"),
                output: root.join("output"),
                root,
            };
            let outcome = portableforge_core::orchestrate::post_capture(&layout)?;
            println!("{}", outcome.review.summary());
            println!(
                "Changes: {} from installing, {} from trying it out.",
                outcome.install_diff.real_file_changes().count()
                    + outcome.install_diff.real_reg_changes().count(),
                outcome.runtime_diff.real_file_changes().count()
                    + outcome.runtime_diff.real_reg_changes().count()
            );
            for s in &outcome.specimens {
                println!("Starting-settings seed: Data\\{}", s.data_relative);
            }
            Ok(())
        }
        "session-new" => {
            let id = need_flag(args, "--id")?;
            let out = need_flag(args, "--out")?;
            let s = Session::new(id, SandboxMode::WindowsSandbox);
            std::fs::write(&out, s.to_json())
                .map_err(|e| format!("We could not save {out}: {e}"))?;
            println!("Started a fresh run in {out}.");
            Ok(())
        }
        other => Err(format!("We do not know the command '{other}'.\n\n{HELP}")),
    }
}

fn read_snapshot(path: Option<&String>) -> Result<Snapshot, String> {
    let path = path.ok_or("Give a snapshot file.")?;
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("We could not read {path}: {e}"))?;
    Snapshot::from_json(&text)
}

fn read_diff(path: Option<&String>) -> Result<portableforge_core::Diff, String> {
    let path = path.ok_or("Give a changes file.")?;
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("We could not read {path}: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("That changes file could not be read: {e}"))
}

fn read_metadata(path: Option<&String>) -> Result<PafMetadata, String> {
    let path = path.ok_or("Give a details file.")?;
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("We could not read {path}: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("Those details could not be read: {e}"))
}
