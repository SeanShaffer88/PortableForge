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
use portableforge_core::snapshot::Snapshot;
use portableforge_core::validate::{generate_tree, has_errors, validate_metadata, validate_tree};
use portableforge_core::wsb::{SandboxConfig, SandboxMode};
use portableforge_core::Session;
use std::path::Path;
use std::process::ExitCode;

/// The documented exit codes (also in the help text):
/// 0 — fine; 1 — something went wrong; 2 — the run worked but a check
/// found problems (build problems, validation problems, leaks, or a
/// doctor report with missing pieces). Commands that judge an
/// artifact set this instead of failing, so scripts can tell "the
/// tool broke" apart from "the answer is no".
static EXIT_CODE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

fn set_exit_code(code: u8) {
    EXIT_CODE.store(code, std::sync::atomic::Ordering::Relaxed);
}

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
        Ok(()) => ExitCode::from(EXIT_CODE.load(std::sync::atomic::Ordering::Relaxed)),
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
        // The edition logic lives in the core (it knows the IoT
        // Enterprise spellings); when EditionID is missing, the
        // product name is the fallback witness.
        let edition_supported = portableforge_core::orchestrate::edition_supported(&edition)
            || reg_value("ProductName")
                .map(|p| {
                    p.contains("IoT Enterprise")
                        || p.contains("Professional")
                        || p.contains("Enterprise")
                        || p.contains("Education")
                })
                .unwrap_or(false);
        let sandbox_exe_present =
            std::path::Path::new(r"C:\Windows\System32\WindowsSandbox.exe").exists();
        // The Sandbox feature's real state, asked from DISM itself
        // (needs elevation; a refused run stays "could not be checked").
        let feature_enabled = std::process::Command::new("dism.exe")
            .args([
                "/online",
                "/Get-FeatureInfo",
                "/FeatureName:Containers-DisposableClientVM",
                "/quiet",
            ])
            .output()
            .ok()
            .filter(|out| out.status.success())
            .and_then(|out| {
                portableforge_core::orchestrate::parse_dism_feature_state(&String::from_utf8_lossy(
                    &out.stdout,
                ))
            });
        // Virtualisation + real free disk/memory, asked from the
        // operating system directly (no new dependencies).
        let (virt_enabled, free_disk_mb, free_ram_mb) = windows_system_facts();
        // Restore-point presence for careful mode's honesty check:
        // vssadmin lists shadow copies; no listing at all means none
        // exist, and a failed run (needs elevation) stays "unknown".
        let restore_point = std::process::Command::new("vssadmin.exe")
            .args(["list", "shadows"])
            .output()
            .ok()
            .and_then(|out| {
                if !out.status.success() {
                    return None;
                }
                let text = String::from_utf8_lossy(&out.stdout).to_string();
                Some(text.contains("Shadow Copy Volume:"))
            });
        portableforge_core::orchestrate::PreflightFacts {
            is_windows: true,
            os_build: build,
            edition_supported,
            sandbox_exe_present,
            feature_enabled,
            free_disk_mb,
            free_ram_mb,
            restore_point,
            virt_enabled,
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
            restore_point: None,
            virt_enabled: None,
        }
    }
}

/// Virtualisation state, free disk and free memory, straight from
/// the operating system (kernel32, no new dependencies). Any figure
/// that cannot be read stays `None` — never a guess.
#[cfg(windows)]
fn windows_system_facts() -> (Option<bool>, Option<u64>, Option<u64>) {
    #[repr(C)]
    struct MemoryStatusEx {
        length: u32,
        memory_load: u32,
        total_phys: u64,
        avail_phys: u64,
        total_page_file: u64,
        avail_page_file: u64,
        total_virtual: u64,
        avail_virtual: u64,
        avail_extended_virtual: u64,
    }
    extern "system" {
        fn GlobalMemoryStatusEx(status: *mut MemoryStatusEx) -> i32;
        fn GetDiskFreeSpaceExW(
            dir: *const u16,
            free_available: *mut u64,
            total: *mut u64,
            total_free: *mut u64,
        ) -> i32;
        fn IsProcessorFeaturePresent(feature: i32) -> i32;
    }
    // PF_VIRT_FIRMWARE_ENABLED = 21 (documented Win32 constant).
    let virt = unsafe { IsProcessorFeaturePresent(21) };
    let virt_enabled = Some(virt != 0);
    let free_ram_mb = unsafe {
        let mut status = MemoryStatusEx {
            length: 64,
            memory_load: 0,
            total_phys: 0,
            avail_phys: 0,
            total_page_file: 0,
            avail_page_file: 0,
            total_virtual: 0,
            avail_virtual: 0,
            avail_extended_virtual: 0,
        };
        if GlobalMemoryStatusEx(&mut status) != 0 {
            Some(status.avail_phys / (1024 * 1024))
        } else {
            None
        }
    };
    let free_disk_mb = unsafe {
        // The volume the user's profile lives on.
        let profile = std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\".to_string());
        let root: Vec<u16> = profile
            .chars()
            .take(3)
            .collect::<String>()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let (mut free_available, mut total, mut total_free) = (0u64, 0u64, 0u64);
        if GetDiskFreeSpaceExW(
            root.as_ptr(),
            &mut free_available,
            &mut total,
            &mut total_free,
        ) != 0
        {
            Some(free_available / (1024 * 1024))
        } else {
            None
        }
    };
    (virt_enabled, free_disk_mb, free_ram_mb)
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
  snapshot <folder> --prefix <path> --out <file.json> [--label <name>] [--quiet]  (or several --root <folder>)
  parse-reg <file.reg> --out <file.json> [--label <name>]
  diff <before.json> <after.json> --out <file.json>
  classify <install-diff.json> <runtime-diff.json> [--details]
  wsb --input <folder> --tools <folder> --exchange <folder> --out <file.wsb> [--online]
  generate <details.json> --out <folder> [--review <review.json>]
  build <details.json> --out <folder> [--session <session-folder>] [--review <review.json>] [--app-files <folder>] [--default-data <folder>] [--icon <file.ico>] [--zip] [--force] [--json]
                                          Build the finished package: the session's notes (or a saved review) supply the accepted fixes, the harvested program files and starting settings are copied in, and the finished tree is checked
  validate [<details.json>] [--tree <folder>]   (with no details file, --tree reads the package's own appinfo.ini)
  session-new --id <name> --out <file.json>
  request-snapshot <session-folder> <before|after-install|after-runtime>
                                          Ask the helper in the test box to take one note now
  agent-state <session-folder>          Show what the helper in the test box is doing and what should happen next
  mode-b --out <folder>                 Make a packet (helper program + plain instructions) for testing in your own virtual machine, with no test box
  review <session-folder>               Show the review list for a finished capture as plain cards, and save the choices file
  launch <session-folder>               Start the test box for a prepared session folder
  post-capture <session-folder> [--json]   Read a finished capture's notes into a review
  preflight [--json]                      Check whether this computer can run the safe test box
  tools [--cache <folder>]              List the pinned PortableApps.com builder tools and check any found in the cache folder
  tools run-installer <package> --cache <folder> [--generator <program>]   Run the checked PortableApps.com setup maker on a finished package folder
  tools run-launcher <package> --cache <folder> [--generator <program>]   Run the checked PortableApps.com starter maker on a finished package folder
  specimens <runtime-diff.json>         List first-run files that would seed the package's starting settings
  leaks <package-folder-in-box> <before.json> <after.json> [--json] [--out <file>]   Judge a finished-copy test run
  rollback-plan <diff.json>             List how a Careful-mode run would be put back
  sessions [--base <folder>]            List earlier runs with their step, age, and what should happen next
  verify <details.json> --out <folder> --session <run> [--app-files <folder>] [--box-root <path>]   Build, then judge the run's leak-test notes when they are there
  rollback <diff.json> [--apply]        Put back what a careful-mode run changed (a rehearsal unless --apply is given)
  sessions-cleanup [--older-than-days 30] [--yes]   List old run folders, and remove them with --yes
  leak-packet --package <folder> --out <folder>   Make the leak-test packet: package copy, helper, instructions, and the note names
  doctor [--cache <folder>] [--json]    One health report: this computer, the maker programs, and the helper program

Every command also answers to --help (for example: portableforge build --help). portableforge --version prints the version and nothing else.
Exit codes: 0 means fine, 1 means something went wrong, 2 means the run worked but a check found problems (build problems, validation problems, leaks, or missing pieces in doctor).
Everywhere: paths are ordinary file paths; nothing is uploaded anywhere.";

/// Short usage for one command (`portableforge build --help`, or
/// `portableforge help build`). Plain words first; the flags after.
fn command_help(cmd: &str) -> Option<String> {
    let text = match cmd {
        "build" => "build <details.json> --out <folder> [--session <session-folder>] [--review <review.json>] [--app-files <folder>] [--default-data <folder>] [--icon <file.ico>] [--zip] [--force] [--json] [--quiet]\n  Make the finished package folder from the details file. --session takes the accepted fixes, name tags, and first-run files from a finished capture. --zip also leaves a zip copy beside the folder. --force allows building into a non-empty folder that is not an earlier package. Ends with exit code 2 when the package has problems to fix.",
        "generate" => "generate <details.json> --out <folder> [--review <review.json>]\n  Write only the text files of a package (no program files copied). Without --review the starter rules carry no fixes, and it says so.",
        "validate" => "validate [<details.json>] [--tree <package-folder>]\n  Check package details, and with --tree also a built package folder. With no details file, --tree reads the package's own appinfo.ini. Exit code 2 when there are problems.",
        "preflight" => "preflight [--json]\n  Check this computer: Windows edition, the test box feature, memory, disk, and which testing way is available.",
        "doctor" => "doctor [--cache <tools-folder>] [--json]\n  One health report: the preflight checks, the official maker programs (checked in --cache when given), and whether the helper program is beside PortableForge. Exit code 2 when something is missing.",
        "sessions" => "sessions [--base <folder>]\n  List the runs in the sessions folder (default: the PortableForge Sessions folder in your home folder) with their step, how long since they were last touched, and what should happen next.",
        "snapshot" => "snapshot (<folder> --prefix <path-prefix> | --root <folder> [--root <folder> ...]) --out <note.json> [--label <name>] [--quiet]\n  Take a note of a folder's files. Several --root folders are read into one note.",
        "parse-reg" => "parse-reg <file.reg> --out <note.json> [--label <name>]\n  Read a settings export (.reg, UTF-8 text) into a note.",
        "diff" => "diff <before.json> <after.json> --out <diff.json>\n  Compare two notes and save the changes between them.",
        "classify" => "classify <install-diff.json> <runtime-diff.json> [--details]\n  Show the plain review cards for two diffs. --details also prints the technical row behind each card.",
        "specimens" => "specimens <diff.json>\n  List the first-run files a diff suggests, and where each goes in Data\\.",
        "leaks" => "leaks <package-folder> <before.json> <after.json> [--json] [--out <file.json>]\n  Judge a leak test: what the package left behind outside itself. Exit code 2 when leaks are found. --out also saves the report as a file.",
        "post-capture" => "post-capture <session-folder> [--json]\n  Turn a finished capture into the review list, the name tags, and the first-run files. Also saves the working files into the session folder.",
        "review" => "review <session-folder>\n  Show the review list for a finished capture as plain cards, and save the choices file.",
        "agent-state" => "agent-state <session-folder>\n  Show what the helper in the test box is doing, when it last reported in, and what should happen next.",
        "request-snapshot" => "request-snapshot <session-folder> <before|after-install|after-runtime>\n  Ask the helper in the test box to take one of the three notes.",
        "mode-b" => "mode-b --out <folder>\n  Make a packet (helper program + plain instructions) for testing in your own virtual machine, with no test box.",
        "launch" => "launch <session-folder>\n  Open the session's test box (Windows only).",
        "tools" => "tools [--cache <folder>]\n  List the official maker programs PortableForge works with, pinned and fingerprinted; with --cache, check what is already downloaded.\ntools run-installer <package> --cache <folder> [--generator <program>]\ntools run-launcher <package> --cache <folder> [--generator <program>]\n  Run one of the makers (fingerprint-checked first) on a finished package folder.",
        "wsb" => "wsb --input <folder> --tools <folder> --exchange <folder> --out <file.wsb> [--online]\n  Write the test box settings file for a session.",
        "session-new" => "session-new --id <name> --out <session.json>\n  Start a fresh run record.",
        "rollback-plan" => "rollback-plan <diff.json>\n  List the steps that would put back everything a diff changed (careful way, watched on the real computer).",
        "guide" => "guide\n  Print the twelve guided steps with what happens and what you do at each.",
        _ => return None,
    };
    Some(format!("portableforge {text}"))
}

fn run_cli(args: &[String]) -> Result<(), String> {
    // Per-command help: `portableforge build --help`.
    if args.len() > 1 && (args[1] == "--help" || args[1] == "-h") {
        return match command_help(&args[0]) {
            Some(text) => {
                println!("{text}");
                Ok(())
            }
            None => Err(format!(
                "We do not know the command '{}'.\n\n{HELP}",
                args[0]
            )),
        };
    }
    match args[0].as_str() {
        "--version" | "-V" | "version" => {
            println!("PortableForge {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "help" | "--help" | "-h" => {
            if let Some(topic) = args.get(1) {
                match command_help(topic) {
                    Some(text) => println!("{text}"),
                    None => println!("{HELP}"),
                }
            } else {
                println!("{HELP}");
            }
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
            let out = need_flag(args, "--out")?;
            let label = flag(args, "--label").unwrap_or_else(|| "snapshot".into());
            let quiet = args.iter().any(|a| a == "--quiet" || a == "-q");
            // Several folders at once: each --root is scanned with its
            // own path as the prefix (the way the helper scans the
            // standard places). The single-folder form stays as it was.
            let roots: Vec<(std::path::PathBuf, String)> = if args.iter().any(|a| a == "--root") {
                let mut v = Vec::new();
                let mut it = args.iter().peekable();
                while let Some(a) = it.next() {
                    if a == "--root" {
                        let r = it.next().ok_or("--root needs a folder after it.")?;
                        v.push((std::path::PathBuf::from(r), r.replace('/', "\\")));
                    }
                }
                v
            } else {
                let root = args.get(1).ok_or("Give the folder to look through.")?;
                let prefix = need_flag(args, "--prefix")?;
                vec![(std::path::PathBuf::from(root), prefix)]
            };
            let mut no_progress = |_: &str, _: u64| {};
            let (snap, _cache) = portableforge_core::snapshot::scan_roots_full(
                &roots,
                &label,
                None,
                &mut no_progress,
            )
            .map_err(|e| format!("We could not look through those folders: {e}"))?;
            std::fs::write(&out, snap.to_json())
                .map_err(|e| format!("We could not save {out}: {e}"))?;
            if !quiet {
                println!("Saved {} files to {out}.", snap.files.len());
                if snap.skipped_files > 0 {
                    println!(
                        "{} files could not be read and were left out of the note.",
                        snap.skipped_files
                    );
                }
            }
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
            let details = args.iter().any(|a| a == "--details");
            for c in &findings {
                let (title, body) = portableforge_core::guide::plain_card(&c.finding);
                println!("- {title}: {body}");
                if details {
                    println!(
                        "  [{:?}, confidence {:.2}] {:?}",
                        c.severity, c.confidence, c.finding
                    );
                    println!("  why: {}", c.why);
                }
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
            cfg.clipboard = args.iter().any(|a| a == "--clipboard");
            if let Some(mb) = flag(args, "--memory").and_then(|s| s.parse::<u32>().ok()) {
                cfg.memory_mb = mb;
            }
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
            // A saved review supplies the accepted fixes; without one
            // the package is generated with no fixes (and says so).
            let accepted = match flag(args, "--review") {
                Some(path) => read_review(&path)?.accepted(),
                None => {
                    println!("No review given — the starter rules carry no fixes.");
                    Vec::new()
                }
            };
            let written = generate_tree(Path::new(&out), &meta, &accepted)?;
            println!("Made {} files in {out}.", written.len());
            Ok(())
        }
        "build" => {
            let json = args.iter().any(|a| a == "--json");
            let quiet = args.iter().any(|a| a == "--quiet" || a == "-q");
            let meta = read_metadata(args.get(1))?;
            let out = need_flag(args, "--out")?;
            let mut accepted: Option<Vec<portableforge_core::classify::Classification>> = None;
            let mut specimens: Vec<portableforge_core::orchestrate::Specimen> = Vec::new();
            if let Some(path) = flag(args, "--review") {
                accepted = Some(read_review(&path)?.accepted());
            }
            if let Some(dir) = flag(args, "--session") {
                let root = std::path::PathBuf::from(&dir);
                let layout = session_layout_for(&root);
                let outcome = portableforge_core::orchestrate::post_capture(&layout)?;
                specimens = outcome.specimens;
                if accepted.is_none() {
                    accepted = Some(outcome.review.accepted());
                }
                if !json && !quiet {
                    println!("{}", outcome.review.summary());
                }
            }
            let accepted = accepted.unwrap_or_default();
            let app_files = flag(args, "--app-files");
            let mut default_data = flag(args, "--default-data");
            // Session builds seed the first-run files automatically:
            // every specimen whose contents were captured (in the
            // harvest, or dropped in exchange\specimens) is laid out
            // the way Data\ will hold it.
            let mut seeded: Vec<String> = Vec::new();
            let mut unseeded: Vec<String> = Vec::new();
            let mut seed_dir: Option<std::path::PathBuf> = None;
            if default_data.is_none() && !specimens.is_empty() {
                if let Some(dir) = flag(args, "--session") {
                    let exchange = session_layout_for(&std::path::PathBuf::from(&dir)).exchange;
                    let dir_path =
                        std::env::temp_dir().join(format!("pf-seed-{}", std::process::id()));
                    let _ = std::fs::remove_dir_all(&dir_path);
                    for spec in &specimens {
                        let rel = spec
                            .data_relative
                            .replace('\\', std::path::MAIN_SEPARATOR_STR);
                        // Candidate contents, in order: an explicit
                        // specimens copy, then any harvest file with
                        // the same name (first-run files commonly live
                        // beside the program).
                        let file_name = spec
                            .sandbox_path
                            .rsplit('\\')
                            .next()
                            .unwrap_or("")
                            .to_string();
                        let mut source: Option<std::path::PathBuf> =
                            Some(exchange.join("specimens").join(&rel)).filter(|p| p.is_file());
                        if source.is_none() {
                            let harvest = exchange.join("harvest");
                            let mut stack = vec![harvest];
                            while let Some(d) = stack.pop() {
                                let Ok(entries) = std::fs::read_dir(&d) else {
                                    continue;
                                };
                                for entry in entries.flatten() {
                                    let p = entry.path();
                                    if p.is_dir() {
                                        stack.push(p);
                                    } else if p.is_file()
                                        && p.file_name()
                                            .map(|n| {
                                                n.to_string_lossy().eq_ignore_ascii_case(&file_name)
                                            })
                                            .unwrap_or(false)
                                    {
                                        source = Some(p);
                                        break;
                                    }
                                }
                                if source.is_some() {
                                    break;
                                }
                            }
                        }
                        match source {
                            Some(src_path) => {
                                let dest = dir_path.join(&rel);
                                if let Some(parent) = dest.parent() {
                                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                                }
                                std::fs::copy(&src_path, &dest).map_err(|e| e.to_string())?;
                                seeded.push(spec.data_relative.clone());
                            }
                            None => unseeded.push(spec.data_relative.clone()),
                        }
                    }
                    if !seeded.is_empty() {
                        default_data = Some(dir_path.to_string_lossy().into_owned());
                        seed_dir = Some(dir_path);
                    } else {
                        let _ = std::fs::remove_dir_all(&dir_path);
                    }
                }
            }
            let icon = flag(args, "--icon");
            let force = args.iter().any(|a| a == "--force");
            let outcome = portableforge_core::build::build_package(
                Path::new(&out),
                &meta,
                &accepted,
                app_files.as_deref().map(Path::new),
                default_data.as_deref().map(Path::new),
                icon.as_deref().map(Path::new),
                force,
            )?;
            if let Some(dir) = &seed_dir {
                let _ = std::fs::remove_dir_all(dir);
            }
            let zip_path = if args.iter().any(|a| a == "--zip") {
                let zp = format!("{out}.zip");
                let n =
                    portableforge_core::build::write_package_zip(Path::new(&out), Path::new(&zp))?;
                if !json && !quiet {
                    println!("Zip copy: {zp} ({n} files)");
                }
                Some(zp)
            } else {
                None
            };
            if json {
                let value = serde_json::json!({
                    "summary": outcome.summary(),
                    "written": outcome.written,
                    "app_files_copied": outcome.app_files_copied,
                    "app_bytes_copied": outcome.app_bytes_copied,
                    "default_data_files_copied": outcome.default_data_files_copied,
                    "skipped": outcome.skipped,
                    "license_file": outcome.license_file,
                    "icon_installed": outcome.icon_installed,
                    "icon_source": outcome.icon_source,
                    "main_exe_arch": outcome.main_exe_arch,
                    "zip": zip_path,
                    "specimens_seeded": seeded,
                    "specimens_unseeded": unseeded,
                    "preserved_data_files": outcome.preserved_data_files,
                    "total_bytes": outcome.total_bytes(),
                    "sizes": outcome.size_breakdown.iter().map(|(folder, bytes, files)| serde_json::json!({
                        "folder": folder, "bytes": bytes, "files": files
                    })).collect::<Vec<_>>(),
                    "specimens": specimens.iter().map(|s| s.data_relative.clone()).collect::<Vec<_>>(),
                    "issues": outcome.issues.iter().map(|i| serde_json::json!({
                        "error": i.error, "field": i.field, "message": i.message
                    })).collect::<Vec<_>>(),
                    "valid": !outcome.has_errors(),
                });
                println!(
                    "{}",
                    serde_json::to_string_pretty(&value).unwrap_or_default()
                );
                if outcome.has_errors() {
                    set_exit_code(2);
                }
                return Ok(());
            }
            if !quiet {
                println!("{}", outcome.summary());
                for s in &seeded {
                    println!("Starting-settings seed planted: Data\\{s}");
                }
                for s in &unseeded {
                    println!(
                        "Starting-settings seed noted (its contents were not captured): Data\\{s}"
                    );
                }
                for issue in &outcome.issues {
                    println!(
                        "{} {}: {}",
                        if issue.error { "Problem" } else { "Note" },
                        issue.field,
                        issue.message
                    );
                }
                for skipped in &outcome.skipped {
                    println!("Skipped: {skipped}");
                }
            }
            if outcome.has_errors() {
                // Exit code 2: the build ran; the package has problems.
                set_exit_code(2);
            }
            Ok(())
        }
        "request-snapshot" => {
            let dir = args
                .get(1)
                .ok_or("Give the session folder (the one containing exchange\\).")?;
            let point_name = args
                .get(2)
                .ok_or("Say which note: before, after-install, or after-runtime.")?;
            let point = portableforge_core::orchestrate::CapturePoint::from_name(point_name)
                .ok_or("Say which note: before, after-install, or after-runtime.")?;
            let layout = session_layout_for(&std::path::PathBuf::from(dir));
            portableforge_core::orchestrate::request_snapshot(&layout, point)?;
            println!(
                "Asked the test box for the {} note. It appears as {} in the session's exchange folder.",
                point.label(),
                point.note_name()
            );
            Ok(())
        }
        "agent-state" => {
            let dir = args
                .get(1)
                .ok_or("Give the session folder (the one containing exchange\\).")?;
            let layout = session_layout_for(&std::path::PathBuf::from(dir));
            let state = portableforge_core::orchestrate::read_agent_state(&layout);
            match &state {
                None => println!("The helper in the test box has not reported in yet."),
                Some(s) => {
                    println!("Step: {:?}", s.phase);
                    if let Some(w) = portableforge_core::orchestrate::agent_version_warning(
                        s,
                        env!("CARGO_PKG_VERSION"),
                    ) {
                        println!("{w}");
                    }
                    if let Some(action) = &s.agent_action {
                        println!("Doing now: {action}");
                    }
                    if let Some(hb) = s.agent_heartbeat_secs {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs() as i64)
                            .unwrap_or(hb);
                        let age = now - hb;
                        println!("Last sign of life: {age} seconds ago.");
                        if age > 60
                            && portableforge_core::orchestrate::next_host_action(Some(s))
                                == portableforge_core::orchestrate::HostAction::Wait
                        {
                            println!("That is a long silence — the test box may have been closed.");
                        }
                    }
                    for line in &s.log {
                        println!("  {line}");
                    }
                }
            }
            if let Some((point, age)) = portableforge_core::orchestrate::pending_request(&layout) {
                println!(
                    "Waiting for the {} note — asked {age} seconds ago.",
                    point.note_name()
                );
                if age > portableforge_core::orchestrate::CAPTURE_WAIT_WARN_SECS {
                    println!("That wait is longer than usual — the test box may have been closed.");
                }
            }
            match portableforge_core::orchestrate::next_host_action(state.as_ref()) {
                portableforge_core::orchestrate::HostAction::Wait => {
                    println!("Next: wait for the helper to finish.")
                }
                portableforge_core::orchestrate::HostAction::Harvest => {
                    println!("Next: the capture is finished — run post-capture or build.")
                }
                portableforge_core::orchestrate::HostAction::Failed(line) => {
                    println!("Next: the helper reported a problem: {line}")
                }
            }
            Ok(())
        }
        "review" => {
            let dir = args
                .get(1)
                .ok_or("Give the session folder (the one containing exchange\\).")?;
            let layout = session_layout_for(&std::path::PathBuf::from(dir));
            let outcome = portableforge_core::orchestrate::post_capture(&layout)?;
            println!("{}", outcome.review.summary());
            for item in &outcome.review.items {
                let (title, body) =
                    portableforge_core::guide::plain_card(&item.classification.finding);
                let state = match item.decision {
                    portableforge_core::review::Decision::Fix { on: true } => "fix on",
                    portableforge_core::review::Decision::Fix { on: false } => "fix off",
                    portableforge_core::review::Decision::Acknowledge { .. } => "warning",
                    portableforge_core::review::Decision::Info => "information",
                };
                println!("[{state}] {title}\n  {body}");
            }
            println!(
                "The choices file is saved at {}",
                layout.root.join("review.json").display()
            );
            Ok(())
        }
        "mode-b" => {
            let out = std::path::PathBuf::from(need_flag(args, "--out")?);
            let exchange = out.join("exchange");
            std::fs::create_dir_all(&exchange).map_err(|e| e.to_string())?;
            // The helper program, copied from beside this program
            // when it is there; a plain note stands in when it is not.
            let agent_names = ["portableforge-agent.exe", "portableforge-agent"];
            let mut copied = false;
            if let Ok(exe) = std::env::current_exe() {
                for name in agent_names {
                    let src = exe.with_file_name(name);
                    if src.is_file() {
                        std::fs::copy(&src, out.join(name)).map_err(|e| e.to_string())?;
                        copied = true;
                        break;
                    }
                }
            }
            if !copied {
                std::fs::write(
                    out.join("AGENT-MISSING.txt"),
                    "The helper program (portableforge-agent) was not next to \
                     PortableForge, so it could not be copied into this packet.\n\
                     Copy portableforge-agent.exe into this folder yourself, \
                     next to READ-ME-FIRST.txt, before using the packet.\n",
                )
                .map_err(|e| e.to_string())?;
            }
            std::fs::write(out.join("READ-ME-FIRST.txt"), MODE_B_INSTRUCTIONS)
                .map_err(|e| e.to_string())?;
            println!(
                "Mode B packet made in {} — read READ-ME-FIRST.txt inside it.",
                out.display()
            );
            if !copied {
                println!("Note: the helper program was not beside PortableForge, so read AGENT-MISSING.txt too.");
            }
            Ok(())
        }
        "launch" => {
            let dir = args
                .get(1)
                .ok_or("Give the session folder (the one containing box.wsb).")?;
            let layout = session_layout_for(&std::path::PathBuf::from(dir));
            launch_sandbox(&layout)
        }
        "validate" => {
            // With a details file: check the details, plus a tree when
            // --tree is given. With --tree alone: read the package's
            // own appinfo.ini and check the tree against it.
            let details_arg = args.get(1).filter(|a| !a.starts_with("--"));
            let mut issues = Vec::new();
            match (details_arg, flag(args, "--tree")) {
                (Some(path), tree) => {
                    let meta = read_metadata(Some(path))?;
                    issues.extend(validate_metadata(&meta));
                    if let Some(tree) = tree {
                        issues.extend(validate_tree(Path::new(&tree), &meta));
                    }
                }
                (None, Some(tree)) => {
                    let root = Path::new(&tree);
                    let appinfo_path = root
                        .join("App")
                        .join("AppInfo")
                        .join("appinfo.ini")
                        .to_string_lossy()
                        .replace('/', std::path::MAIN_SEPARATOR_STR);
                    let text = std::fs::read_to_string(&appinfo_path).map_err(|e| {
                        format!(
                            "We could not read the package's own appinfo.ini ({appinfo_path}): {e}"
                        )
                    })?;
                    let meta = meta_from_appinfo(&text);
                    issues.extend(validate_tree(root, &meta));
                }
                (None, None) => {
                    return Err(
                        "Give a details file, or --tree <package-folder> to check a built package."
                            .into(),
                    )
                }
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
                // Exit code 2: the check ran; it found problems.
                set_exit_code(2);
            } else {
                println!("Everything checks out.");
            }
            Ok(())
        }
        "preflight" => {
            let report =
                portableforge_core::orchestrate::evaluate_preflight(&collect_preflight_facts());
            let (name, honesty) = report.best_mode.describe();
            if args.iter().any(|a| a == "--json") {
                let value = serde_json::json!({
                    "checks": report.checks.iter().map(|c| serde_json::json!({
                        "label": c.label, "ok": c.ok, "fix": c.fix, "copy_text": c.copy_text
                    })).collect::<Vec<_>>(),
                    "best_mode": { "name": name, "honesty": honesty },
                });
                println!(
                    "{}",
                    serde_json::to_string_pretty(&value).unwrap_or_default()
                );
                return Ok(());
            }
            for c in &report.checks {
                println!("{} {}", if c.ok { "OK  " } else { "MISS" }, c.label);
                if !c.fix.is_empty() {
                    println!("     Fix: {}", c.fix);
                }
                if let Some(copy) = &c.copy_text {
                    println!("     Copy this: {copy}");
                }
            }
            println!("Best available way: {name} — {honesty}");
            Ok(())
        }
        "tools" => {
            // `tools run-installer|run-launcher <package-folder>
            // --cache <folder> [--generator <program>]`: run one of
            // the pinned, fingerprint-checked PortableApps.com maker
            // programs on a finished package. Explicitly invoked —
            // nothing runs them automatically.
            let run_kind = match args.get(1).map(String::as_str) {
                Some("run-installer") => {
                    Some(portableforge_core::tools::ToolId::InstallerGenerator)
                }
                Some("run-launcher") => Some(portableforge_core::tools::ToolId::LauncherGenerator),
                _ => None,
            };
            if let Some(tool_id) = run_kind {
                let package = args
                    .get(2)
                    .ok_or("Give the finished package folder to run the maker on.")?;
                let cache = flag(args, "--cache")
                    .map(std::path::PathBuf::from)
                    .ok_or("Give --cache <folder>, the folder the tools were downloaded into.")?;
                let pin = portableforge_core::tools::pin(tool_id);
                let download = portableforge_core::tools::find_verified_tool(&cache, pin)
                    .ok_or_else(|| format!(
                        "The {} {} is not in {} yet, or its fingerprint does not match — download it from {} first.",
                        pin.plain_name, pin.version, cache.display(), pin.download_url
                    ))?;
                // The maker program itself: the path supplied with
                // --generator wins; otherwise look in the cache folder
                // and one level down (where its own installer puts it).
                let mut generator: Option<std::path::PathBuf> =
                    flag(args, "--generator").map(std::path::PathBuf::from);
                if generator.is_none() {
                    let direct = cache.join(pin.generator_exe);
                    if direct.is_file() {
                        generator = Some(direct);
                    } else if let Ok(entries) = std::fs::read_dir(&cache) {
                        for entry in entries.flatten() {
                            let candidate = entry.path().join(pin.generator_exe);
                            if candidate.is_file() {
                                generator = Some(candidate);
                                break;
                            }
                        }
                    }
                }
                let Some(generator) = generator else {
                    println!(
                        "The checked download is ready at {}, but the maker program itself ({}) was not found. Run the download once and install it into a folder inside {}, or say where it is with --generator <program>.",
                        download.display(),
                        pin.generator_exe,
                        cache.display()
                    );
                    return Ok(());
                };
                if !generator.is_file() {
                    return Err(format!(
                        "The maker program {} could not be found.",
                        generator.display()
                    ));
                }
                let invocation =
                    portableforge_core::tools::generator_invocation(pin, Path::new(package));
                let program = generator.with_file_name(&invocation.program);
                println!(
                    "{} {} (download checked against the recorded fingerprint)",
                    pin.plain_name, pin.version
                );
                println!("Package: {package}");
                #[cfg(windows)]
                {
                    let status = std::process::Command::new(&program)
                        .args(&invocation.args)
                        .status()
                        .map_err(|e| format!("The maker could not be started: {e}"))?;
                    println!("The maker finished with: {status}");
                }
                #[cfg(not(windows))]
                {
                    println!(
                        "On Windows this runs: {} {}",
                        program.display(),
                        invocation.args.join(" ")
                    );
                    println!("(This computer is not Windows, so nothing was run.)");
                }
                return Ok(());
            }
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
            let value = serde_json::json!({
                "verdict": report.verdict(),
                "leaks": report.leaks.iter().map(|l| serde_json::json!({
                    "kind": l.kind, "plain": l.plain, "fix": l.fix_line()
                })).collect::<Vec<_>>(),
                "groups": report.grouped().iter().map(|(label, leaks)| serde_json::json!({
                    "group": label, "count": leaks.len(),
                    "leaks": leaks.iter().map(|l| serde_json::json!({
                        "kind": l.kind, "plain": l.plain, "fix": l.fix_line()
                    })).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
            });
            if let Some(out_file) = flag(args, "--out") {
                std::fs::write(
                    &out_file,
                    serde_json::to_string_pretty(&value).unwrap_or_default(),
                )
                .map_err(|e| format!("We could not save {out_file}: {e}"))?;
                println!("Saved the leak report to {out_file}.");
            }
            if args.iter().any(|a| a == "--json") {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&value).unwrap_or_default()
                );
            } else {
                println!("{}", report.verdict());
                for (label, leaks) in report.grouped() {
                    println!("{label} ({})", leaks.len());
                    for leak in leaks {
                        println!("- {}", leak.plain);
                        println!("  Fix: {}", leak.fix_line());
                    }
                }
            }
            if !report.leaks.is_empty() {
                // Exit code 2: the test ran; it found leaks.
                set_exit_code(2);
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
        "rollback" => {
            let d = read_diff(args.get(1))?;
            let plan = portableforge_core::rollback::rollback_plan(&d);
            let apply = args.iter().any(|a| a == "--apply");
            if !apply {
                println!(
                    "This is a rehearsal — nothing on this computer will change. Add --apply to really put things back."
                );
            }
            let log =
                portableforge_core::rollback::execute_rollback(&plan, apply, &mut registry_undo);
            for line in &log {
                println!("{line}");
            }
            println!(
                "{} steps listed. Careful mode is watched and put back — it is NOT sealed off.",
                plan.len()
            );
            Ok(())
        }
        "verify" => {
            let details = args
                .get(1)
                .filter(|a| !a.starts_with("--"))
                .ok_or("Give a details file.")?;
            let out = need_flag(args, "--out")?;
            let session_dir = need_flag(args, "--session")?;
            let meta = read_metadata(Some(details))?;
            let app_files = flag(args, "--app-files").map(std::path::PathBuf::from);
            let outcome = portableforge_core::build::build_package(
                Path::new(&out),
                &meta,
                &[],
                app_files.as_deref(),
                None,
                None,
                false,
            )?;
            println!("{}", outcome.summary());
            let mut problems = outcome.has_errors();
            // Judge the leak notes when the run left them: the packet
            // names them leak-before.json / leak-after.json in the
            // session's exchange folder.
            let layout = session_layout_for(Path::new(&session_dir));
            let before_path = layout.exchange.join("leak-before.json");
            let after_path = layout.exchange.join("leak-after.json");
            if before_path.is_file() && after_path.is_file() {
                let before = read_snapshot(Some(&before_path.to_string_lossy().into_owned()))?;
                let after = read_snapshot(Some(&after_path.to_string_lossy().into_owned()))?;
                let d = diff_snapshots(&before, &after);
                let app_id = Path::new(&out)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let box_root = flag(args, "--box-root")
                    .unwrap_or_else(|| format!(r"C:\Users\WDAGUtilityAccount\Desktop\{app_id}"));
                let report =
                    portableforge_core::leaktest::analyse_leaks(&box_root, &before, &after, &d);
                println!("{}", report.verdict());
                for (label, leaks) in report.grouped() {
                    println!("{label} ({})", leaks.len());
                    for leak in leaks {
                        println!("- {}", leak.plain);
                        println!("  Fix: {}", leak.fix_line());
                    }
                }
                if !report.clean() {
                    problems = true;
                }
            } else {
                println!(
                    "No leak-test notes in this run yet (leak-before.json and leak-after.json in its exchange folder) — the package is built; the leak test still needs its own box run."
                );
            }
            if problems {
                // Exit code 2: verify ran; it found problems or leaks.
                set_exit_code(2);
            }
            Ok(())
        }
        "sessions-cleanup" => {
            let home = std::env::var("USERPROFILE")
                .or_else(|_| std::env::var("HOME"))
                .unwrap_or_else(|_| ".".to_string());
            let base = flag(args, "--base")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::path::PathBuf::from(&home).join("PortableForge Sessions"));
            let days: u64 = flag(args, "--older-than-days")
                .and_then(|s| s.parse().ok())
                .unwrap_or(30);
            let yes = args.iter().any(|a| a == "--yes");
            let cutoff =
                std::time::SystemTime::now() - std::time::Duration::from_secs(days * 86_400);
            let mut listed = 0usize;
            if let Ok(entries) = std::fs::read_dir(&base) {
                for entry in entries.flatten() {
                    let dir = entry.path();
                    if !dir.is_dir() {
                        continue;
                    }
                    let layout = session_layout_for(&dir);
                    if !layout.exchange.is_dir() && !layout.session_path.is_file() {
                        continue;
                    }
                    let newest = [
                        layout.session_path.clone(),
                        layout.exchange.join("state.json"),
                        dir.clone(),
                    ]
                    .iter()
                    .filter_map(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
                    .max();
                    let Some(newest) = newest else { continue };
                    if newest >= cutoff {
                        continue;
                    }
                    listed += 1;
                    let name = dir
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    if yes {
                        match std::fs::remove_dir_all(&dir) {
                            Ok(()) => println!("Removed the old run folder: {name}"),
                            Err(e) => println!("Could not remove {name}: {e}"),
                        }
                    } else {
                        println!("Old run folder (untouched for over {days} days): {name}");
                    }
                }
            }
            if listed == 0 {
                println!(
                    "No run folders older than {days} days in {}.",
                    base.display()
                );
            } else if !yes {
                println!(
                    "Nothing was removed. Add --yes to remove the {listed} folder{} listed.",
                    if listed == 1 { "" } else { "s" }
                );
            }
            Ok(())
        }
        "leak-packet" => {
            let package = need_flag(args, "--package")?;
            let out = need_flag(args, "--out")?;
            let out_dir = Path::new(&out);
            let pkg_src = Path::new(&package);
            if !pkg_src.is_dir() {
                return Err(format!(
                    "The finished package folder {package} could not be found."
                ));
            }
            let exchange = out_dir.join("exchange");
            std::fs::create_dir_all(&exchange)
                .map_err(|e| format!("We could not make the packet folder: {e}"))?;
            let pkg_name = pkg_src
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Package".to_string());
            let pkg_dest = out_dir.join(&pkg_name);
            copy_tree(pkg_src, &pkg_dest)?;
            let agent = place_agent(out_dir);
            std::fs::write(
                out_dir.join("READ-ME-FIRST.txt"),
                format!(
                    "Leak test for {pkg_name}\n\
                     ======================\n\n\
                     1. Start a fresh, empty test box (Windows Sandbox) or a clean virtual machine.\n\
                     2. Copy this whole packet folder into it.\n\
                     3. Move the {pkg_name} folder onto the Desktop inside the box.\n\
                     4. Before opening it, take the first note (from this folder, inside the box):\n\
                     \x20    portableforge-agent.exe capture --exchange exchange --name leak-before.json\n\
                     5. Open {pkg_name}, use it the way its owner would, then close it.\n\
                     6. Take the second note the same way, with --name leak-after.json\n\
                     7. Copy the exchange folder back out and put it in the run's own exchange folder.\n\
                     8. Back in PortableForge: portableforge verify <details.json> --out <package> --session <run folder>\n\
                     \x20    — or just re-open the run in the wizard and read the result on the test step.\n\n\
                     A clean portable copy changes nothing outside its own folder. Anything else is a leak.\n"
                ),
            )
            .map_err(|e| format!("We could not write the packet instructions: {e}"))?;
            println!(
                "Leak-test packet ready at {out}: the package copy, the instructions{}, and an exchange folder for the two notes (leak-before.json, leak-after.json).",
                if agent {
                    ", the helper program"
                } else {
                    " (the helper program was not beside PortableForge, so copy portableforge-agent.exe in by hand)"
                }
            );
            Ok(())
        }
        "post-capture" => {
            let dir = args
                .get(1)
                .ok_or("Give the session folder (the one containing exchange\\).")?;
            let root = std::path::PathBuf::from(dir);
            let layout = session_layout_for(&root);
            let outcome = portableforge_core::orchestrate::post_capture(&layout)?;
            if args.iter().any(|a| a == "--json") {
                let value = serde_json::json!({
                    "review_summary": outcome.review.summary(),
                    "review": outcome.review,
                    "install_changes": outcome.install_diff.real_file_changes().count()
                        + outcome.install_diff.real_reg_changes().count(),
                    "runtime_changes": outcome.runtime_diff.real_file_changes().count()
                        + outcome.runtime_diff.real_reg_changes().count(),
                    "specimens": outcome.specimens.iter().map(|s| serde_json::json!({
                        "sandbox_path": s.sandbox_path, "data_relative": s.data_relative
                    })).collect::<Vec<_>>(),
                    "shortcut_candidates": outcome.shortcut_candidates,
                    "hints": {
                        "display_name": outcome.hints.display_name,
                        "publisher": outcome.hints.publisher,
                        "display_version": outcome.hints.display_version,
                    },
                });
                println!(
                    "{}",
                    serde_json::to_string_pretty(&value).unwrap_or_default()
                );
                return Ok(());
            }
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
            if !outcome.shortcut_candidates.is_empty() {
                println!(
                    "Start Menu shortcut found for: {} — the main file is often the same name with .exe on the end.",
                    outcome.shortcut_candidates.join(", ")
                );
            }
            if let Some(name) = &outcome.hints.display_name {
                println!("Program's own name tag: {name}");
            }
            if let Some(pub_) = &outcome.hints.publisher {
                println!("Program's own maker tag: {pub_}");
            }
            if let Some(ver) = &outcome.hints.display_version {
                println!("Program's own version tag: {ver}");
            }
            Ok(())
        }
        "sessions" => {
            let home = std::env::var("USERPROFILE")
                .or_else(|_| std::env::var("HOME"))
                .unwrap_or_else(|_| ".".to_string());
            let base = flag(args, "--base")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::path::PathBuf::from(&home).join("PortableForge Sessions"));
            let mut rows: Vec<(std::time::SystemTime, String)> = Vec::new();
            if let Ok(entries) = std::fs::read_dir(&base) {
                for entry in entries.flatten() {
                    let dir = entry.path();
                    if !dir.is_dir() {
                        continue;
                    }
                    let layout = session_layout_for(&dir);
                    if !layout.exchange.is_dir() && !layout.session_path.is_file() {
                        continue;
                    }
                    // Age from the newest thing the run left behind.
                    let mut newest = std::time::SystemTime::UNIX_EPOCH;
                    for p in [
                        layout.session_path.clone(),
                        layout.exchange.join("state.json"),
                        dir.clone(),
                    ] {
                        if let Ok(m) = std::fs::metadata(&p) {
                            if let Ok(t) = m.modified() {
                                if t > newest {
                                    newest = t;
                                }
                            }
                        }
                    }
                    let age = std::time::SystemTime::now()
                        .duration_since(newest)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let age_text = if age < 120 {
                        "just now".to_string()
                    } else if age < 3600 {
                        format!("{} min ago", age / 60)
                    } else if age < 86_400 {
                        format!("{} h ago", age / 3600)
                    } else {
                        format!("{} d ago", age / 86_400)
                    };
                    let state = portableforge_core::orchestrate::read_agent_state(&layout);
                    let phase = state
                        .as_ref()
                        .map(|s| format!("{:?}", s.phase))
                        .unwrap_or_else(|| "not started".to_string());
                    let next =
                        match portableforge_core::orchestrate::next_host_action(state.as_ref()) {
                            portableforge_core::orchestrate::HostAction::Wait => {
                                "wait for the helper".to_string()
                            }
                            portableforge_core::orchestrate::HostAction::Harvest => {
                                "ready to gather the results".to_string()
                            }
                            portableforge_core::orchestrate::HostAction::Failed(msg) => {
                                format!("the helper reported a problem: {msg}")
                            }
                        };
                    rows.push((
                        newest,
                        format!(
                            "{} — step: {phase}, last touched {age_text}, next: {next}",
                            dir.file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| dir.display().to_string())
                        ),
                    ));
                }
            }
            rows.sort_by_key(|row| std::cmp::Reverse(row.0));
            if rows.is_empty() {
                println!("No runs found in {}.", base.display());
            } else {
                for (_, line) in &rows {
                    println!("{line}");
                }
            }
            Ok(())
        }
        "doctor" => {
            let report =
                portableforge_core::orchestrate::evaluate_preflight(&collect_preflight_facts());
            let (mode_name, honesty) = report.best_mode.describe();
            let missed = report.checks.iter().filter(|c| !c.ok).count();
            // The helper program lives beside this one.
            let helper = std::env::current_exe().ok().and_then(|exe| {
                ["portableforge-agent.exe", "portableforge-agent"]
                    .iter()
                    .map(|n| exe.with_file_name(n))
                    .find(|p| p.is_file())
            });
            // The maker programs, when a cache folder is given.
            let cache = flag(args, "--cache").map(std::path::PathBuf::from);
            let mut tools_json = Vec::new();
            let mut tools_missing = 0usize;
            for pin in portableforge_core::tools::TOOL_PINS {
                let ready = cache
                    .as_ref()
                    .and_then(|dir| portableforge_core::tools::find_verified_tool(dir, pin));
                if cache.is_some() && ready.is_none() {
                    tools_missing += 1;
                }
                tools_json.push(serde_json::json!({
                    "name": pin.plain_name, "version": pin.version,
                    "ready": ready.is_some(),
                    "checked": cache.is_some(),
                }));
            }
            let problems = missed + tools_missing + usize::from(helper.is_none());
            if args.iter().any(|a| a == "--json") {
                let value = serde_json::json!({
                    "checks": report.checks.iter().map(|c| serde_json::json!({
                        "label": c.label, "ok": c.ok, "fix": c.fix, "copy_text": c.copy_text
                    })).collect::<Vec<_>>(),
                    "best_mode": { "name": mode_name, "honesty": honesty },
                    "tools": tools_json,
                    "helper_present": helper.is_some(),
                    "helper_path": helper.as_ref().map(|p| p.display().to_string()),
                });
                println!(
                    "{}",
                    serde_json::to_string_pretty(&value).unwrap_or_default()
                );
            } else {
                println!(
                    "Testing way: {mode_name} — {honesty} ({} check{} missed)",
                    missed,
                    if missed == 1 { "" } else { "s" }
                );
                for (i, pin) in portableforge_core::tools::TOOL_PINS.iter().enumerate() {
                    let state = if cache.is_none() {
                        "not checked (no --cache given)".to_string()
                    } else if tools_json[i]["ready"].as_bool() == Some(true) {
                        "ready and checked".to_string()
                    } else {
                        "missing or changed — see tools --cache".to_string()
                    };
                    println!("{} {}: {state}", pin.plain_name, pin.version);
                }
                match &helper {
                    Some(p) => println!("Helper program: ready at {}", p.display()),
                    None => println!(
                        "Helper program: NOT beside PortableForge — the test box cannot be run without it."
                    ),
                }
            }
            if problems > 0 {
                // Exit code 2: the report ran; something is missing.
                set_exit_code(2);
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

/// The session layout for a session *root* folder (the folder that
/// contains `exchange\`, `box.wsb`, and the rest).
/// Copy the helper program from beside this one into `out`.
/// Returns true when it was there to copy.
pub fn place_agent(out: &Path) -> bool {
    if let Ok(exe) = std::env::current_exe() {
        for name in ["portableforge-agent.exe", "portableforge-agent"] {
            let src = exe.with_file_name(name);
            if src.is_file() && std::fs::copy(&src, out.join(name)).is_ok() {
                return true;
            }
        }
    }
    false
}

/// Recursively copy a folder tree (packet building).
fn copy_tree(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst)
        .map_err(|e| format!("We could not make {}: {e}", dst.display()))?;
    let entries =
        std::fs::read_dir(src).map_err(|e| format!("We could not read {}: {e}", src.display()))?;
    for entry in entries.flatten() {
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)
                .map_err(|e| format!("We could not copy {}: {e}", from.display()))?;
        }
    }
    Ok(())
}

/// The registry half of a rollback: put one setting back via the
/// operating system's own `reg.exe`. Only runs on Windows; anywhere
/// else it refuses plainly so the executor can log it.
fn registry_undo(action: &portableforge_core::rollback::RollbackAction) -> Result<(), String> {
    #[cfg(windows)]
    {
        use portableforge_core::rollback::RollbackAction;
        let run = |args: &[&str]| -> Result<(), String> {
            let out = std::process::Command::new("reg.exe")
                .args(args)
                .output()
                .map_err(|e| format!("the settings tool could not start: {e}"))?;
            if out.status.success() {
                Ok(())
            } else {
                Err("the settings tool refused".to_string())
            }
        };
        match action {
            RollbackAction::DeleteSetting { key, name } => run(&["delete", key, "/v", name, "/f"]),
            RollbackAction::RestoreSetting {
                key,
                name,
                old_data,
            } => {
                let (ty, data) =
                    portableforge_core::rollback::restore_value(old_data).ok_or_else(|| {
                        "the old value is in a form that cannot be put back automatically"
                            .to_string()
                    })?;
                run(&["add", key, "/v", name, "/t", ty, "/d", &data, "/f"])
            }
            _ => Err("not a setting".to_string()),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = action;
        Err("settings can only be put back on Windows".to_string())
    }
}

pub fn session_layout_for(root: &Path) -> portableforge_core::orchestrate::SessionLayout {
    portableforge_core::orchestrate::SessionLayout {
        wsb_path: root.join("box.wsb"),
        session_path: root.join("session.json"),
        input: root.join("input"),
        tools: root.join("tools"),
        exchange: root.join("exchange"),
        output: root.join("output"),
        root: root.to_path_buf(),
    }
}

fn read_review(path: &str) -> Result<portableforge_core::review::Review, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("We could not read {path}: {e}"))?;
    portableforge_core::review::Review::from_json(&text)
}

/// Start the test box for a prepared session. Windows only — on any
/// other system the command is printed and the caller told plainly
/// that nothing was started.
/// The plain instructions inside a Mode B packet. Mode B means: your
/// own virtual machine, its own snapshot feature, and these steps by
/// hand — PortableForge still does the thinking afterwards.
const MODE_B_INSTRUCTIONS: &str = "\
Testing with your own virtual machine (Mode B)\n\
==============================================\n\
\n\
This way uses a virtual machine you run yourself, instead of the\n\
built-in test box. Your real computer is never touched: everything\n\
happens inside the virtual machine, and its own snapshot feature is\n\
what keeps it safe — take a snapshot of the clean machine first.\n\
\n\
You need: this folder (with the helper program in it), the setup\n\
file of the program you want to make portable, and a Windows\n\
virtual machine that can see this folder (a shared folder is fine).\n\
\n\
1. Take a snapshot of the clean virtual machine, so you can go back.\n\
2. Copy this folder into the virtual machine.\n\
3. In the virtual machine, open a command window in this folder and\n\
   take the clean note:\n\
     portableforge-agent capture --exchange exchange --name before.json\n\
4. Run the setup file and install the program, then take the\n\
   after-installing note. EITHER run it by hand:\n\
     portableforge-agent capture --exchange exchange --name after-install.json\n\
   OR let the helper take it when things settle — start this BEFORE\n\
   running the setup file:\n\
     portableforge-agent watch --exchange exchange\n\
5. Start the installed program once, use it a little, close it, then:\n\
     portableforge-agent capture --exchange exchange --name after-runtime.json\n\
6. Copy the whole exchange folder back to your real computer, into\n\
   the session's exchange folder (or a fresh folder), and on the\n\
   real computer run:\n\
     portableforge post-capture <that folder's session>\n\
     portableforge build <details.json> --session <session> --app-files <the program's folder>\n\
\n\
Tip: the three notes are just files in the exchange folder. If one\n\
is missing, PortableForge will tell you exactly which one.\n";

pub fn launch_sandbox(
    layout: &portableforge_core::orchestrate::SessionLayout,
) -> Result<(), String> {
    let (program, arg) = portableforge_core::orchestrate::sandbox_launch_command(layout);
    if !layout.wsb_path.is_file() {
        return Err(format!(
            "There is no test box settings file at {} — make the session first (the guided window does this on the safety step).",
            layout.wsb_path.display()
        ));
    }
    #[cfg(windows)]
    {
        std::process::Command::new(&program)
            .arg(&arg)
            .spawn()
            .map_err(|e| format!("The test box could not be started: {e}"))?;
        println!("The test box is starting. Ask it for each note with request-snapshot: before, after-install, then after-runtime.");
        Ok(())
    }
    #[cfg(not(windows))]
    {
        Err(format!(
            "The test box can only be started on Windows. On Windows this runs: {program} {arg}"
        ))
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

/// Rebuild just enough package details from a built package's own
/// `appinfo.ini` to check its tree (used by `validate --tree` when no
/// details file is at hand). The main program's name comes from the
/// icon-extraction path, which names it exactly.
fn meta_from_appinfo(text: &str) -> PafMetadata {
    let get = |key: &str| -> String {
        for line in text.lines() {
            if let Some((k, v)) = line.split_once('=') {
                if k.trim().eq_ignore_ascii_case(key) {
                    return v.trim().to_string();
                }
            }
        }
        String::new()
    };
    let extract_icon = get("ExtractIcon");
    let main_exe = extract_icon
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or("")
        .to_string();
    PafMetadata {
        name: get("Name"),
        app_id: get("AppID"),
        base_app_name: get("BaseAppName"),
        publisher: get("Publisher"),
        homepage: get("Homepage"),
        category: get("Category"),
        description: get("Description"),
        language: get("Language"),
        shareable: true,
        open_source: false,
        freeware: false,
        commercial_use: false,
        package_version: get("PackageVersion"),
        display_version: get("DisplayVersion"),
        requires_admin: false,
        main_exe,
        main_exe_64bit: None,
        distribution: portableforge_core::paf::Distribution::Bundled {
            redistribution_asserted: true,
        },
    }
}
