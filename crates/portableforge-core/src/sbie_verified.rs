//! Dedicated Sandboxie capture sessions that can be sealed into a profile.

mod runtime;
mod sources;
mod windows;

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf, Prefix};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::compile::safe_name;
use crate::compile::windows::{
    copy_tree, local_directory, metadata_at, new_output_path, require_regular_path, within,
};
use crate::orchestrate::{post_capture, session_layout, SessionLayout};
use crate::profile::{fingerprint, inventory, local_file, read_small, tree_sha256, write_new};
use crate::regfile::{decode_reg_export, parse_reg_with_report};
use crate::snapshot::{normalise_path, sha256_hex, FileEntry, Snapshot};
use crate::{Phase, SandboxMode, Session};
pub use runtime::{test_built_paf, RuntimeDataStatus, RuntimeReport, RuntimeRequest};
use sources::{setup_args_sha256, PreparedSetup};
pub(crate) use sources::{verify_capture_sources, SetupCompanion};
pub use windows::CancellationToken;
use windows::{LaunchArguments, RunLimits, Sandboxie, VerifiedExit};

pub(crate) const RECEIPT_FILE: &str = "sandboxie-receipt.json";
pub(crate) const MAX_RECEIPT_BYTES: usize = 64 * 1024;
const VERSION: &str = "1.18.8";
const ROOT_NAME: &str = "box";
const MAX_NOTE_BYTES: usize = 32 * 1024 * 1024;
const QUIET_TIMEOUT: Duration = Duration::from_secs(30);
const EXPORT_TIMEOUT: Duration = Duration::from_secs(90);
const NOTE_FILES: [&str; 3] = ["before.json", "after-install.json", "after-runtime.json"];
const REGISTRY_ROOTS: [&str; 3] = [
    r"HKCU\Software",
    r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
    r"HKLM\SYSTEM\CurrentControlSet\Services",
];

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Receipt {
    schema_version: u32,
    sandboxie_plus_version: String,
    session_id: String,
    box_name: String,
    setup_sha256: String,
    note_sha256: [String; 3],
    harvest_sha256: String,
    setup_pid: u32,
    setup_exit_code: u32,
    exercise_pid: u32,
    exercise_exit_code: u32,
    cleanup_verified: bool,
    #[serde(default)]
    setup_name: Option<String>,
    #[serde(default)]
    pub(crate) setup_args_sha256: Option<String>,
    #[serde(default)]
    installed_application: Option<PathBuf>,
    #[serde(default)]
    pub(crate) companions: Vec<SetupCompanion>,
}

pub(crate) fn verify_receipt(
    bytes: &[u8],
    session_id: &str,
    setup_sha256: &str,
    note_sha256: [&str; 3],
    harvest_sha256: &str,
) -> Result<Receipt, String> {
    let receipt: Receipt = serde_json::from_slice(bytes)
        .map_err(|e| format!("Invalid Sandboxie capture receipt: {e}"))?;
    if !sources::valid_receipt_sources(&receipt)
        || receipt.sandboxie_plus_version != VERSION
        || receipt.session_id != session_id
        || !valid_box_name(&receipt.box_name)
        || receipt.setup_sha256 != setup_sha256
        || receipt.note_sha256 != note_sha256
        || receipt.harvest_sha256 != harvest_sha256
        || receipt.setup_pid == 0
        || receipt.exercise_pid == 0
        || receipt.setup_exit_code != 0
        || receipt.exercise_exit_code != 0
        || !receipt.cleanup_verified
    {
        return Err("The Sandboxie receipt does not match this capture or prove cleanup.".into());
    }
    Ok(receipt)
}

/// Check the current pinned Sandboxie installation without changing its configuration.
pub fn preflight() -> Result<(), String> {
    Sandboxie::load()?.verify_global()
}

/// Verify a capture and return the selected app and the digest of its receipt.
pub fn verify_capture_application(
    session: &Path,
    expected: Option<&str>,
) -> Result<(Option<PathBuf>, String), String> {
    let session = local_directory(session)?;
    let id = session
        .file_name()
        .and_then(|part| part.to_str())
        .ok_or("Invalid session name.")?;
    let exchange = session.join("exchange");
    verify_clean_storage(&session)?;
    let bytes = read_small(&session.join(RECEIPT_FILE), &session, MAX_RECEIPT_BYTES)?;
    let digest = sha256_hex(&bytes);
    if expected.is_some_and(|expected| expected != digest) {
        return Err("The capture receipt changed since approval.".into());
    }
    let input = session.join("input");
    let rows = inventory(&input)?;
    let [row] = rows.as_slice() else {
        return Err("The capture must contain exactly one setup file in input.".into());
    };
    let setup = row
        .file
        .as_ref()
        .ok_or("The capture setup is not a regular file.")?;
    let mut notes = [String::new(), String::new(), String::new()];
    for (index, name) in NOTE_FILES.iter().enumerate() {
        notes[index] = sha256_hex(&read_small(&exchange.join(name), &session, MAX_NOTE_BYTES)?);
    }
    let receipt = verify_receipt(
        &bytes,
        id,
        &setup.sha256,
        notes.each_ref().map(String::as_str),
        &tree_sha256(&exchange.join("harvest"))?,
    )?;
    verify_capture_sources(&session, &receipt)?;
    Ok((receipt.installed_application, digest))
}

pub(crate) fn verify_clean_storage(session: &Path) -> Result<(), String> {
    expect_absent(&session.join(ROOT_NAME))
        .map_err(|error| format!("The Sandboxie box still needs cleanup. {error}"))
}

fn valid_box_name(name: &str) -> bool {
    name.strip_prefix("PF_").is_some_and(|suffix| {
        suffix.len() == 28 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn new_box_name() -> Result<String, String> {
    // BCryptGenRandom fills a caller-owned buffer with system randomness.
    #[link(name = "bcrypt")]
    extern "system" {
        fn BCryptGenRandom(algorithm: isize, buffer: *mut u8, length: u32, flags: u32) -> i32;
    }
    let mut random = [0_u8; 14];
    // SAFETY: the buffer is writable for the exact requested length; the
    // system-preferred RNG flag requires a null algorithm handle.
    let status = unsafe { BCryptGenRandom(0, random.as_mut_ptr(), random.len() as u32, 2) };
    if status != 0 {
        return Err(format!(
            "Could not generate a unique box name (0x{status:08x})."
        ));
    }
    let suffix = random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .concat();
    Ok(format!("PF_{suffix}"))
}

fn io_error(path: &Path, error: io::Error) -> String {
    format!("{}: {error}", path.display())
}

fn mkdir(path: &Path) -> Result<(), String> {
    fs::create_dir(path).map_err(|error| io_error(path, error))
}

fn expect_absent(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(path, error)),
        Ok(_) => Err(format!("Expected {} to be absent.", path.display())),
    }
}

fn logical_executable(path: &Path) -> Result<PathBuf, String> {
    let mut parts = path.components();
    let Some(Component::Prefix(prefix)) = parts.next() else {
        return Err("Choose an absolute local installed executable path.".into());
    };
    if !matches!(prefix.kind(), Prefix::Disk(_)) || parts.next() != Some(Component::RootDir) {
        return Err("Choose an absolute local installed executable path.".into());
    }
    if !parts
        .all(|part| matches!(part, Component::Normal(name) if name.to_str().is_some_and(safe_name)))
        || !path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
    {
        return Err("Choose a normal installed .exe file inside the box.".into());
    }
    Ok(path.to_path_buf())
}

fn system_reg() -> Result<PathBuf, String> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetSystemDirectoryW(buffer: *mut u16, length: u32) -> u32;
    }
    let mut path = [0_u16; 261];
    // SAFETY: the output buffer has the supplied capacity in UTF-16 units.
    let length = unsafe { GetSystemDirectoryW(path.as_mut_ptr(), path.len() as u32) } as usize;
    if length == 0 || length >= path.len() {
        return Err("Could not resolve the Windows system directory.".into());
    }
    let directory = String::from_utf16(&path[..length]).map_err(|error| error.to_string())?;
    local_file(&PathBuf::from(directory).join("reg.exe"))
}

struct BoxRoots {
    system_drive: PathBuf,
    windows: PathBuf,
    user: [(PathBuf, PathBuf); 3],
    common_roots: Vec<PathBuf>,
    excluded_trees: Vec<PathBuf>,
}

impl BoxRoots {
    fn new(reg: &Path) -> Result<Self, String> {
        fn environment_directory(name: &str) -> Result<PathBuf, String> {
            let path = std::env::var_os(name).ok_or_else(|| format!("Missing {name}."))?;
            local_directory(Path::new(&path))
        }
        let system_drive = reg
            .ancestors()
            .last()
            .ok_or("Could not determine the system drive.")?
            .to_path_buf();
        let windows = reg
            .parent()
            .and_then(Path::parent)
            .ok_or("Could not determine the Windows directory.")?
            .to_path_buf();
        let user = [
            ("user/current".into(), environment_directory("USERPROFILE")?),
            ("user/all".into(), environment_directory("ProgramData")?),
            ("user/public".into(), environment_directory("PUBLIC")?),
        ];
        let profile = &user[0].1;
        let mut common_roots: Vec<_> = user.iter().map(|(_, host)| host.clone()).collect();
        common_roots.extend(
            [
                "Users",
                "ProgramData",
                "Program Files",
                "Program Files (x86)",
            ]
            .map(|name| system_drive.join(name)),
        );
        common_roots.extend(
            [
                "AppData",
                r"AppData\Local",
                r"AppData\LocalLow",
                r"AppData\Roaming",
                r"AppData\Local\Programs",
                "Desktop",
                "Downloads",
                "Documents",
            ]
            .map(|name| profile.join(name)),
        );
        common_roots.extend(
            [
                "ProgramFiles",
                "ProgramFiles(x86)",
                "ProgramW6432",
                "LOCALAPPDATA",
                "APPDATA",
            ]
            .into_iter()
            .filter_map(|name| std::env::var_os(name).map(PathBuf::from)),
        );
        let mut excluded_trees = vec![
            system_drive.join(r"Program Files\Common Files"),
            system_drive.join(r"Program Files (x86)\Common Files"),
            profile.join(r"AppData\Local\Temp"),
        ];
        excluded_trees.extend(
            [
                "CommonProgramFiles",
                "CommonProgramFiles(x86)",
                "CommonProgramW6432",
                "TEMP",
            ]
            .into_iter()
            .filter_map(|name| std::env::var_os(name).map(PathBuf::from))
            .filter(|path| path.is_absolute() && path.parent().is_some()),
        );
        Ok(Self {
            system_drive,
            windows,
            user,
            common_roots,
            excluded_trees,
        })
    }

    fn dedicated_payload_parent(&self, parent: &Path) -> bool {
        let allowed = |path: &Path| {
            !within(path, &self.windows)
                && !self
                    .excluded_trees
                    .iter()
                    .any(|shared| within(path, shared))
                && !self
                    .common_roots
                    .iter()
                    .any(|shared| within(path, shared) && within(shared, path))
        };
        allowed(parent) && !local_directory(parent).is_ok_and(|resolved| !allowed(&resolved))
    }

    fn drive_mirror(drive_root: &Path) -> Result<PathBuf, String> {
        match drive_root.components().next() {
            Some(Component::Prefix(prefix)) => match prefix.kind() {
                Prefix::Disk(letter) => {
                    Ok(PathBuf::from("drive").join(char::from(letter).to_string()))
                }
                _ => Err("Unexpected sandbox drive mapping.".into()),
            },
            _ => Err("Unexpected sandbox drive mapping.".into()),
        }
    }

    fn logical_file(&self, root: &Path, logical: &Path) -> Result<PathBuf, String> {
        let drive_root = logical
            .ancestors()
            .last()
            .ok_or("Missing installed drive.")?;
        let mut candidates = vec![root.join(Self::drive_mirror(drive_root)?).join(
            logical
                .strip_prefix(drive_root)
                .map_err(|e| e.to_string())?,
        )];
        for (physical, host) in &self.user {
            if within(logical, host) {
                let tail: PathBuf = logical
                    .components()
                    .skip(host.components().count())
                    .collect();
                candidates.push(root.join(physical).join(tail));
            }
        }
        let mut found = None;
        for path in candidates {
            // Inspect from the root before following any candidate parent.
            let mut current = root.to_path_buf();
            let relative = path.strip_prefix(root).map_err(|e| e.to_string())?;
            let mut exists = true;
            for part in relative.components() {
                current.push(part);
                match fs::symlink_metadata(&current) {
                    Ok(_) => {
                        metadata_at(&current)?;
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        exists = false;
                        break;
                    }
                    Err(error) => return Err(io_error(&current, error)),
                }
            }
            if exists {
                require_regular_path(&path, root)?;
                if found.replace(path).is_some() {
                    return Err("The installed executable has ambiguous redirected paths.".into());
                }
            }
        }
        found.ok_or_else(|| format!("Setup did not install {} in the box.", logical.display()))
    }

    fn snapshot_files(
        &self,
        root: &Path,
        exports: &str,
        snapshot: &mut Snapshot,
    ) -> Result<(), String> {
        let mut scan_roots = Vec::new();
        let drive = root.join("drive");
        if optional_directory(&drive)? {
            for entry in fs::read_dir(&drive).map_err(|e| io_error(&drive, e))? {
                let entry = entry.map_err(|e| io_error(&drive, e))?;
                let name = entry.file_name();
                let name = name
                    .to_str()
                    .filter(|name| name.len() == 1 && name.as_bytes()[0].is_ascii_alphabetic())
                    .ok_or("Unsupported Sandboxie drive layout.")?;
                scan_roots.push((entry.path(), PathBuf::from(format!("{name}:\\"))));
            }
        }
        let user = root.join("user");
        if optional_directory(&user)? {
            for entry in fs::read_dir(&user).map_err(|e| io_error(&user, e))? {
                let entry = entry.map_err(|e| io_error(&user, e))?;
                let Some((_, host)) = self
                    .user
                    .iter()
                    .find(|(part, _)| part.file_name() == Some(entry.file_name().as_os_str()))
                else {
                    return Err("Unsupported Sandboxie user-folder layout.".into());
                };
                scan_roots.push((entry.path(), host.clone()));
            }
        }
        if optional_directory(&root.join("share"))? {
            return Err("The offline capture contains redirected network-share files.".into());
        }
        let private_notes = self.system_drive.join(exports);
        let mut files = BTreeMap::new();
        for (physical, logical) in scan_roots {
            let rows = inventory(&physical)?;
            for row in rows {
                let Some(fp) = row.file else { continue };
                let path = logical.join(row.path);
                if within(&path, &private_notes) {
                    continue;
                }
                let path = normalise_path(&path.to_string_lossy());
                let record = FileEntry {
                    path: path.clone(),
                    size: fp.size_bytes,
                    sha256: fp.sha256,
                    mtime_secs: 0,
                };
                if files.insert(path.to_uppercase(), record).is_some() {
                    return Err(format!("Ambiguous redirected capture path: {path}."));
                }
                if files.len() > 20_000 {
                    return Err("The box capture exceeds the file-count limit.".into());
                }
            }
        }
        snapshot.files = files.into_values().collect();
        Ok(())
    }
}

fn optional_directory(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error(path, error)),
        Ok(_) if metadata_at(path)?.is_dir() => Ok(true),
        Ok(_) => Err(format!("Expected a directory at {}.", path.display())),
    }
}

fn checked_exit(exit: VerifiedExit, stage: &str) -> Result<VerifiedExit, String> {
    if exit.pid == 0 || exit.exit_code != 0 {
        return Err(format!(
            "Boxed {stage} did not finish successfully (exit {}).",
            exit.exit_code
        ));
    }
    Ok(exit)
}

// The boxed registry view includes host values. Preserve only identities
// that changed across a captured stage, never the full baseline exports.
fn retain_registry_changes(snapshots: &mut [Snapshot; 3]) {
    let (changed_values, changed_services) = {
        let values = snapshots.each_ref().map(Snapshot::reg_map);
        let keys: BTreeSet<_> = values.iter().flat_map(|map| map.keys()).collect();
        let changed_values: BTreeSet<_> = keys
            .into_iter()
            .filter(|key| {
                values[0].get(*key) != values[1].get(*key)
                    || values[1].get(*key) != values[2].get(*key)
            })
            .cloned()
            .collect();
        let services = snapshots.each_ref().map(|snapshot| {
            snapshot
                .services
                .iter()
                .map(String::as_str)
                .collect::<BTreeSet<_>>()
        });
        let changed_services: BTreeSet<_> = services
            .iter()
            .flat_map(|set| set.iter().copied())
            .filter(|name| {
                services[0].contains(name) != services[1].contains(name)
                    || services[1].contains(name) != services[2].contains(name)
            })
            .map(str::to_owned)
            .collect();
        (changed_values, changed_services)
    };
    for snapshot in snapshots {
        snapshot.registry.retain(|entry| {
            changed_values.contains(&(
                entry.key.to_ascii_lowercase(),
                entry.name.to_ascii_lowercase(),
            ))
        });
        snapshot
            .services
            .retain(|name| changed_services.contains(name));
    }
}

struct ActiveCapture<'a> {
    sbie: &'a Sandboxie,
    name: &'a str,
    root: &'a Path,
    layout: &'a SessionLayout,
    roots: BoxRoots,
    reg: PathBuf,
    exports: String,
    cancellation: &'a CancellationToken,
}

impl ActiveCapture<'_> {
    fn snapshot(&self, point: usize) -> Result<Snapshot, String> {
        self.cancellation.check()?;
        self.sbie.quiescent(self.name, QUIET_TIMEOUT)?;
        let filename = NOTE_FILES[point];
        let mut snapshot = Snapshot::new(filename.trim_end_matches(".json"));
        let logical_notes = self.roots.system_drive.join(&self.exports);
        let physical_notes = self
            .root
            .join(BoxRoots::drive_mirror(&self.roots.system_drive)?)
            .join(&self.exports);
        let mut services = BTreeSet::new();
        for (index, hive) in REGISTRY_ROOTS.iter().enumerate() {
            if local_directory(&physical_notes)? != physical_notes {
                return Err("The redirected registry export folder changed.".into());
            }
            expect_absent(&logical_notes)?;
            let filename = format!("{point}-{index}.reg");
            let physical = physical_notes.join(&filename);
            expect_absent(&physical)?;
            // The caller creates only the verified redirected parent. An
            // ordinary unboxed reg.exe would not have this logical folder.
            let logical = logical_notes.join(&filename);
            let args = [
                OsString::from("export"),
                OsString::from(hive),
                logical.into_os_string(),
            ];
            checked_exit(
                self.sbie.run(
                    self.name,
                    self.root,
                    &self.reg,
                    LaunchArguments::Tokens(&args),
                    &self.layout.input,
                    RunLimits {
                        timeout: EXPORT_TIMEOUT,
                        cancellation: self.cancellation,
                    },
                )?,
                "registry export",
            )?;
            self.sbie.quiescent(self.name, QUIET_TIMEOUT)?;
            expect_absent(&logical_notes)?;
            let bytes = read_small(&physical, self.root, MAX_NOTE_BYTES)?;
            fs::remove_file(&physical).map_err(|error| io_error(&physical, error))?;
            if !bytes.starts_with(&[0xff, 0xfe]) || bytes.len() % 2 != 0 {
                return Err("A boxed registry export has an unexpected encoding.".into());
            }
            let decoded = decode_reg_export(&bytes)?;
            if !decoded.starts_with("Windows Registry Editor Version 5.00") {
                return Err("A boxed registry export has an invalid header.".into());
            }
            let (entries, skipped) = parse_reg_with_report(&decoded);
            if skipped != 0 {
                return Err(format!(
                    "A boxed registry export could not be read completely ({hive}, {filename}, {skipped} skipped)."
                ));
            }
            if index == 2 {
                const PREFIX: &str = r"hkey_local_machine\system\currentcontrolset\services\";
                for entry in entries {
                    let lower = entry.key.to_ascii_lowercase();
                    if let Some(tail) = lower.strip_prefix(PREFIX) {
                        if let Some(service) =
                            tail.split('\\').next().filter(|name| !name.is_empty())
                        {
                            services.insert(service.to_string());
                        }
                    }
                }
            } else {
                snapshot.registry.extend(entries);
            }
        }
        snapshot.services = services.into_iter().collect();
        self.roots
            .snapshot_files(self.root, &self.exports, &mut snapshot)?;
        Ok(snapshot)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureStage {
    Preparing,
    Installing,
    SelectingApplication,
    Exercising,
    CleaningUp,
}

/// Run a trusted setup in a new dedicated box and save a review-ready session.
pub fn capture(
    setup: &Path,
    installed: &Path,
    output: &Path,
    setup_args: &[OsString],
    timeout: Duration,
) -> Result<PathBuf, String> {
    capture_with_progress(
        setup,
        installed,
        output,
        setup_args,
        timeout,
        |stage| match stage {
            CaptureStage::Installing => {
                eprintln!("The verified box is ready. Finish the setup in its own window.");
            }
            CaptureStage::Exercising => {
                eprintln!("Try the installed app, then close it to finish the capture.");
            }
            CaptureStage::Preparing
            | CaptureStage::SelectingApplication
            | CaptureStage::CleaningUp => {}
        },
    )
}

/// Run the same verified capture while reporting major stages to a host UI.
/// The progress callback should return promptly. A callback panic aborts
/// the capture and still cleans up the dedicated box.
pub fn capture_with_progress(
    setup: &Path,
    installed: &Path,
    output: &Path,
    setup_args: &[OsString],
    timeout: Duration,
    progress: impl FnMut(CaptureStage),
) -> Result<PathBuf, String> {
    capture_with_options(
        CaptureRequest {
            setup,
            installed: Some(installed),
            output,
            setup_args,
            companions: &[],
            timeout,
        },
        progress,
        |_| Err("Choose an installed executable.".into()),
        &CancellationToken::new()?,
    )
}

pub struct CaptureRequest<'a> {
    pub setup: &'a Path,
    pub installed: Option<&'a Path>,
    pub output: &'a Path,
    pub setup_args: &'a [OsString],
    pub companions: &'a [PathBuf],
    pub timeout: Duration,
}

/// Capture a setup with explicitly staged files and an optional interactive
/// choice of installed executable. The box is cleaned up on cancellation.
pub fn capture_with_options(
    request: CaptureRequest<'_>,
    mut progress: impl FnMut(CaptureStage),
    mut select: impl FnMut(&[PathBuf]) -> Result<PathBuf, String>,
    cancellation: &CancellationToken,
) -> Result<PathBuf, String> {
    let CaptureRequest {
        setup,
        installed,
        output,
        setup_args,
        companions,
        timeout,
    } = request;
    let mut notify = |stage| {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| progress(stage)))
            .map_err(|_| format!("Could not report Sandboxie capture stage {stage:?}."))
    };
    notify(CaptureStage::Preparing)?;
    cancellation.check()?;
    let setup = local_file(setup)?;
    if setup
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("msi"))
    {
        return Err("Use Windows Sandbox for MSI capture. The Windows Installer service is not verified in Sandboxie.".into());
    }
    logical_executable(&setup)?;
    let installed = installed.map(logical_executable).transpose()?;
    let setup_args_sha256 = setup_args_sha256(setup_args)?;
    let setup_fp = fingerprint(&setup)?;
    if setup_fp.size_bytes == 0 {
        return Err("The setup file is empty.".into());
    }
    let output = new_output_path(output)?;
    if within(&setup, &output) {
        return Err("Choose a new session folder separate from the setup.".into());
    }
    let sbie = Sandboxie::load()?;
    let reg = system_reg()?;
    let roots = BoxRoots::new(&reg)?;
    let host = local_file(&std::env::current_exe().map_err(|e| e.to_string())?)?;
    let name = new_box_name()?;
    let parent = output.parent().ok_or("Choose a session folder.")?;
    let id = output
        .file_name()
        .and_then(|part| part.to_str())
        .ok_or("Invalid session name.")?;
    let layout = session_layout(parent, id);
    mkdir(&layout.root)?;
    mkdir(&layout.input)?;
    mkdir(&layout.exchange)?;
    mkdir(&layout.output)?;
    let source = PreparedSetup::stage(&setup, &setup_fp, companions, &layout)?;
    cancellation.check()?;
    let root = layout.root.join(ROOT_NAME);
    expect_absent(&root)?;
    let exports = format!("{name}_Notes");
    expect_absent(&roots.system_drive.join(&exports))?;
    let active = ActiveCapture {
        sbie: &sbie,
        name: &name,
        root: &root,
        layout: &layout,
        roots,
        reg,
        exports,
        cancellation,
    };
    let owned = serde_json::to_vec_pretty(&serde_json::json!({ "box_name": name, "root": root }))
        .map_err(|e| e.to_string())?;
    write_new(&layout.root.join("sandboxie-owned.json"), &owned)?;
    sbie.provision(&name, &root)?;
    let captured = (|| {
        let notes = root
            .join(BoxRoots::drive_mirror(&active.roots.system_drive)?)
            .join(&active.exports);
        // The unique owned root was absent at provision. Every newly created
        // ancestor is checked before collecting redirected data.
        fs::create_dir_all(&notes).map_err(|e| io_error(&notes, e))?;
        local_directory(&notes)?;
        let before = active.snapshot(0)?;
        if fingerprint(&source.image)? != setup_fp {
            return Err("Setup bytes changed before launch.".into());
        }
        notify(CaptureStage::Installing)?;
        let setup_exit = checked_exit(
            sbie.run(
                &name,
                &root,
                &source.image,
                LaunchArguments::Tokens(setup_args),
                &source.cwd,
                RunLimits {
                    timeout,
                    cancellation,
                },
            )?,
            "setup",
        )?;
        let after_install = active.snapshot(1)?;
        let installed = match installed {
            Some(installed) => installed,
            None => {
                notify(CaptureStage::SelectingApplication)?;
                let candidates = application_candidates(&active, &before, &after_install)?;
                let selected =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| select(&candidates)))
                        .map_err(|_| "Could not select the boxed application.")??;
                cancellation.check()?;
                if !candidates.contains(&selected) {
                    return Err("Choose an executable from the captured applications.".into());
                }
                selected
            }
        };
        payload_parent(&active.roots, &root, &installed)?;
        notify(CaptureStage::Exercising)?;
        let app_args = [
            OsString::from("__sandboxie-exercise"),
            OsString::from(&name),
            root.clone().into_os_string(),
            sbie.runtime_root().as_os_str().to_os_string(),
            installed.clone().into_os_string(),
        ];
        let exercise = checked_exit(
            sbie.run(
                &name,
                &root,
                &host,
                LaunchArguments::Tokens(&app_args),
                &layout.input,
                RunLimits {
                    timeout,
                    cancellation,
                },
            )?,
            "application exercise",
        )?;
        let after_runtime = active.snapshot(2)?;
        let payload = payload_parent(&active.roots, &root, &installed)?;
        let harvest = layout.exchange.join("harvest");
        let expected = tree_sha256(&payload)?;
        copy_tree(&payload, &harvest)?;
        if tree_sha256(&harvest)? != expected {
            return Err("The boxed payload changed during capture.".into());
        }
        if fingerprint(&source.image)? != setup_fp {
            return Err("Setup bytes changed during capture.".into());
        }
        Ok::<_, String>((
            [before, after_install, after_runtime],
            expected,
            setup_exit,
            exercise,
            installed,
        ))
    })();
    let notified = notify(CaptureStage::CleaningUp);
    let cleaned = cleanup(&sbie, &name, &root, &layout);
    let (mut snapshots, harvest_sha256, setup_exit, exercise, installed) = match (
        captured.and_then(|result| notified.map(|()| result)),
        cleaned,
    ) {
        (Ok(captured), Ok(())) => captured,
        (Err(error), Ok(())) => return Err(format!("{error} The dedicated box was cleaned up.")),
        (result, Err(cleanup_error)) => {
            let cause = result
                .err()
                .unwrap_or_else(|| "Capture finished.".to_string());
            return Err(format!(
                "{cause} Cleanup failed for {name} at {}. {cleanup_error}",
                root.display(),
            ));
        }
    };

    cancellation.check()?;
    retain_registry_changes(&mut snapshots);
    let mut note_sha256 = [String::new(), String::new(), String::new()];
    for (point, snapshot) in snapshots.into_iter().enumerate() {
        let bytes = snapshot.to_json().into_bytes();
        if bytes.len() > MAX_NOTE_BYTES {
            return Err("The boxed snapshot exceeds the capture note limit.".into());
        }
        write_new(&layout.exchange.join(NOTE_FILES[point]), &bytes)?;
        note_sha256[point] = sha256_hex(&bytes);
    }
    let mut session = Session::new(id, SandboxMode::Sandboxie);
    session.installer_path = Some(setup.to_string_lossy().into_owned());
    session.snapshots_done = NOTE_FILES
        .iter()
        .map(|name| name.trim_end_matches(".json").into())
        .collect();
    session.advance_to(Phase::ReviewChanges);
    session.say("Captured in a dedicated Sandboxie-Plus box. Its processes, configuration and files were checked after cleanup.");
    write_new(&layout.session_path, session.to_json().as_bytes())?;
    post_capture(&layout)?;
    let receipt = Receipt {
        schema_version: 2,
        sandboxie_plus_version: VERSION.into(),
        session_id: id.into(),
        box_name: name,
        setup_sha256: setup_fp.sha256,
        note_sha256,
        harvest_sha256,
        setup_pid: setup_exit.pid,
        setup_exit_code: setup_exit.exit_code,
        exercise_pid: exercise.pid,
        exercise_exit_code: exercise.exit_code,
        cleanup_verified: true,
        setup_name: Some(source.name),
        setup_args_sha256: Some(setup_args_sha256),
        installed_application: Some(installed),
        companions: source.companions,
    };
    verify_capture_sources(&layout.root, &receipt)?;
    let receipt = serde_json::to_vec_pretty(&receipt).map_err(|e| e.to_string())?;
    if receipt.len() > MAX_RECEIPT_BYTES {
        return Err("The Sandboxie capture receipt exceeds its size limit.".into());
    }
    cancellation.check()?;
    write_new(&layout.root.join(RECEIPT_FILE), &receipt)?;
    Ok(layout.root)
}

fn payload_parent(roots: &BoxRoots, root: &Path, logical: &Path) -> Result<PathBuf, String> {
    let logical_parent = logical
        .parent()
        .ok_or("Could not determine the installed app folder.")?;
    if !roots.dedicated_payload_parent(logical_parent) {
        return Err("Choose an executable in a dedicated application folder.".into());
    }
    let physical = roots.logical_file(root, logical)?;
    let payload = physical
        .parent()
        .ok_or("Could not determine the installed app folder.")?;
    if payload
        .strip_prefix(root)
        .map_err(|e| e.to_string())?
        .components()
        .count()
        < 3
    {
        return Err("Choose an executable in a dedicated application folder.".into());
    }
    Ok(payload.to_path_buf())
}

fn application_candidates(
    active: &ActiveCapture<'_>,
    before: &Snapshot,
    after: &Snapshot,
) -> Result<Vec<PathBuf>, String> {
    let prior: BTreeMap<_, _> = before
        .files
        .iter()
        .map(|file| (&file.path, &file.sha256))
        .collect();
    let mut candidates = Vec::new();
    for file in &after.files {
        let path = Path::new(&file.path);
        if prior
            .get(&file.path)
            .is_some_and(|hash| **hash == file.sha256)
            || logical_executable(path).is_err()
            || payload_parent(&active.roots, active.root, path).is_err()
        {
            continue;
        }
        candidates.push(path.to_path_buf());
        if candidates.len() > 128 {
            return Err("More than 128 installed executables were captured. Specify the app path explicitly.".into());
        }
    }
    if candidates.is_empty() {
        return Err("No installed executables were found in a dedicated app folder.".into());
    }
    Ok(candidates)
}

fn cleanup(
    sbie: &Sandboxie,
    name: &str,
    root: &Path,
    layout: &SessionLayout,
) -> Result<(), String> {
    sbie.stop(name, root)?;
    // Verify the entire owned storage path before recursive removal. No
    // reparse points or oversized/unsupported trees are traversed.
    if root != layout.root.join(ROOT_NAME) {
        return Err("The box root differs from this session's owned path.".into());
    }
    sbie.remove(name, root)?;
    if optional_directory(root)? {
        let verified = local_directory(root)?;
        if verified != root {
            return Err("The owned box path changed before removal.".into());
        }
        inventory(&verified)?;
        fs::remove_dir_all(&verified).map_err(|error| io_error(&verified, error))?;
    }
    expect_absent(root)
}

/// Internal subcommand entered only from a driver-verified process.
pub fn exercise_boxed(
    name: &str,
    root: &Path,
    runtime_root: &Path,
    program: &Path,
) -> Result<(), String> {
    if !valid_box_name(name) {
        return Err("Invalid dedicated box identity.".into());
    }
    let program = logical_executable(program)?;
    let helper = std::env::current_exe().map_err(|error| error.to_string())?;
    Sandboxie::verify_current(name, root, runtime_root, &helper)?;
    let status = Command::new(&program)
        .current_dir(program.parent().ok_or("Choose an installed app folder.")?)
        .stdin(Stdio::null())
        .status()
        .map_err(|error| io_error(&program, error))?;
    if !status.success() {
        return Err(format!("The boxed application returned {status}."));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_binds_notes_harvest_and_confirmed_cleanup() {
        let mut receipt = Receipt {
            schema_version: 1,
            sandboxie_plus_version: VERSION.into(),
            session_id: "session".into(),
            box_name: format!("PF_{}", "1".repeat(28)),
            setup_sha256: "setup".into(),
            note_sha256: ["a".into(), "b".into(), "c".into()],
            harvest_sha256: "payload".into(),
            setup_pid: 1,
            setup_exit_code: 0,
            exercise_pid: 2,
            exercise_exit_code: 0,
            cleanup_verified: true,
            setup_name: None,
            setup_args_sha256: None,
            installed_application: None,
            companions: vec![],
        };
        let args = ["a", "b", "c"];
        let encode = |receipt: &Receipt| serde_json::to_vec(receipt).unwrap();
        assert!(verify_receipt(&encode(&receipt), "session", "setup", args, "payload").is_ok());
        assert!(
            verify_receipt(&encode(&receipt), "session", "different", args, "payload").is_err()
        );
        assert!(verify_receipt(
            &encode(&receipt),
            "session",
            "setup",
            ["a", "changed", "c"],
            "payload"
        )
        .is_err());
        receipt.cleanup_verified = false;
        assert!(verify_receipt(&encode(&receipt), "session", "setup", args, "payload").is_err());
    }

    #[test]
    fn receipt_v2_binds_safe_setup_source_names_and_explicit_app() {
        let mut receipt = Receipt {
            schema_version: 2,
            sandboxie_plus_version: VERSION.into(),
            session_id: "session".into(),
            box_name: format!("PF_{}", "1".repeat(28)),
            setup_sha256: "setup".into(),
            note_sha256: ["a".into(), "b".into(), "c".into()],
            harvest_sha256: "payload".into(),
            setup_pid: 1,
            setup_exit_code: 0,
            exercise_pid: 2,
            exercise_exit_code: 0,
            cleanup_verified: true,
            setup_name: Some("Setup.exe".into()),
            setup_args_sha256: Some(
                setup_args_sha256(&[OsString::from("/owned-fixture")]).unwrap(),
            ),
            installed_application: Some(PathBuf::from(r"C:\Widget\Widget.exe")),
            companions: vec![SetupCompanion {
                name: "input.cab".into(),
                sha256: "f".repeat(64),
                size_bytes: 8,
            }],
        };
        let verify = |receipt: &Receipt| {
            verify_receipt(
                &serde_json::to_vec(receipt).unwrap(),
                "session",
                "setup",
                ["a", "b", "c"],
                "payload",
            )
        };
        assert!(verify(&receipt).is_ok());
        receipt.companions[0].name = "SETUP.EXE".into();
        assert!(verify(&receipt).is_err());
        receipt.companions[0].name = "../input.cab".into();
        assert!(verify(&receipt).is_err());
        receipt.companions.clear();
        receipt.installed_application = None;
        assert!(verify(&receipt).is_err());
    }

    #[test]
    fn installed_executable_paths_must_be_absolute_local_paths() {
        assert!(logical_executable(Path::new(r"C:\Program Files\Widget\Widget.exe")).is_ok());
        for invalid in [
            r"C:Widget.exe",
            r"C:\Widget\..\Widget.exe",
            r"\\host\share\Widget.exe",
            r"C:\Widget\Widget.exe:stream",
        ] {
            assert!(logical_executable(Path::new(invalid)).is_err(), "{invalid}");
        }
    }

    #[test]
    fn shared_payload_roots_are_rejected_but_dedicated_app_folders_are_allowed() {
        let roots = BoxRoots::new(Path::new(r"C:\Windows\System32\reg.exe")).unwrap();
        for shared in [
            r"C:\Program Files",
            r"C:\ProgramData",
            r"C:\Program Files (x86)",
            r"C:\Program Files\Common Files",
            r"C:\Program Files (x86)\Common Files\Vendor",
            r"C:\Windows\System32",
        ] {
            assert!(
                !roots.dedicated_payload_parent(Path::new(shared)),
                "{shared}"
            );
        }
        assert!(roots.dedicated_payload_parent(Path::new(r"C:\Program Files\Widget")));
        let local_vendor = roots.user[0].1.join(r"AppData\Local\Vendor\bin");
        assert!(roots.dedicated_payload_parent(&local_vendor));
    }

    #[test]
    fn unchanged_host_registry_values_are_not_retained() {
        let entry = |name: &str, data: &str| crate::RegEntry {
            key: r"HKEY_CURRENT_USER\Software\Widget".into(),
            name: name.into(),
            data: data.into(),
        };
        let mut snapshots = [
            Snapshot {
                registry: vec![entry("host", "private"), entry("app", "old")],
                ..Snapshot::default()
            },
            Snapshot {
                registry: vec![entry("host", "private"), entry("app", "new")],
                ..Snapshot::default()
            },
            Snapshot {
                registry: vec![entry("host", "private")],
                ..Snapshot::default()
            },
        ];
        retain_registry_changes(&mut snapshots);
        assert_eq!(snapshots[0].registry, vec![entry("app", "old")]);
        assert_eq!(snapshots[1].registry, vec![entry("app", "new")]);
        assert!(snapshots[2].registry.is_empty());
    }
}
