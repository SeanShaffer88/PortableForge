use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read, Write};
use std::os::windows::fs::MetadataExt;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{installer_settings, package_settings, safe_name};
use crate::paf::Distribution;
use crate::snapshot::pe_arch;

const MAKER: &str = "PortableApps.comInstaller.exe";
const COMPILER: &str = "App/nsis/Bin/makensis.exe";
const TOOL_VERSION: &str = "3.9.19";
const TOOLS: &[(&str, &str)] = &[
    (
        MAKER,
        "e32cb548713566a2a1f64d51f34b0b9827a4eb8b96256b56a8ecc3ba237c9491",
    ),
    (
        "App/nsis/makensis.exe",
        "b043e554afefbfc56315669d0b4779793aeae67f0f2a7a790e2ea91f05298eff",
    ),
    (
        COMPILER,
        "7144fb5e10113752a92c743b55d0d7bea9c842d4d9cdbfe3d63b092f3c9debf3",
    ),
    (
        "App/bin/MakeHeader.exe",
        "c4d270ddcebd77df693f5cb9112cc5b5372cd1d19a3aec601320d9e982747b6d",
    ),
];
const TIMEOUT_MILLIS: u32 = 120_000;
const INSTALLER_SCRIPT: &str = "PortableApps.comInstaller.nsi";
const REPARSE_POINT: u32 = 0x400;

#[derive(Debug, Serialize)]
pub struct CompileReport {
    pub artifact: PathBuf,
    pub size_bytes: u64,
    pub sha256: String,
    pub application_icon_sha256: String,
    pub embedded_icons_verified: bool,
    pub installer_version: &'static str,
    pub verified_executables: &'static [(&'static str, &'static str)],
    pub source_package: PathBuf,
    pub tool_root: PathBuf,
    pub redistribution_asserted: bool,
    pub steps: Vec<BuildStep>,
}

#[derive(Debug, Serialize)]
pub struct BuildStep {
    executable: PathBuf,
    arguments: Vec<PathBuf>,
    working_directory: PathBuf,
    log: PathBuf,
    exit_code: i32,
}

/// Build an offline installer from a trusted prepared package and trusted
/// installation of PortableApps.com Installer. Inputs must remain stable for
/// the duration of this operation. The final installer and its manifest appear
/// together under `output/result` after successful compilation.
pub fn compile_installer(
    package: &Path,
    tool_root: &Path,
    output: &Path,
    redistribution_asserted: bool,
) -> Result<CompileReport, String> {
    Distribution::Bundled {
        redistribution_asserted,
    }
    .check_guardrail()?;

    let package = local_directory(package)?;
    let tool_root = local_directory(tool_root)?;
    let output_name = output
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| safe_name(name))
        .ok_or("Choose a plain name for the new output folder.")?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let output = local_directory(parent)?.join(output_name);
    if within(&output, &package)
        || within(&output, &tool_root)
        || within(&package, &tool_root)
        || within(&tool_root, &package)
    {
        return Err("Keep the package, installed tool and new output folders separate.".into());
    }

    check_tools(&tool_root)?;
    // Reserve the entire run directory with a single exclusive create. A failed
    // attempt retains its work and cannot be mistaken for a fresh attempt.
    fs::create_dir(&output).map_err(|e| {
        format!(
            "Could not create a new output folder at {}: {e}",
            output.display()
        )
    })?;
    compile_in(&package, &tool_root, &output, redistribution_asserted).map_err(|e| {
        format!(
            "{e}\nBuild files, if any, are in {}.",
            output.join("work").display()
        )
    })
}

fn compile_in(
    package: &Path,
    tool_root: &Path,
    output: &Path,
    redistribution_asserted: bool,
) -> Result<CompileReport, String> {
    let work = output.join("work");
    create_dir(&work)?;
    let package_work = work.join("package");
    create_dir(&package_work)?;
    // Package name is meaningful to the maker. Read it from the original,
    // then validate the authoritative staged copy before invoking any tool.
    let initial_appinfo = package.join("App/AppInfo/appinfo.ini");
    require_regular_path(&initial_appinfo, package)?;
    let initial = package_settings(&ini_text(&initial_appinfo)?)?;
    let stage = package_work.join(&initial.app_id);
    copy_tree(package, &stage)?;
    let appinfo = stage.join("App/AppInfo");
    let metadata = package_settings(&ini_text(&appinfo.join("appinfo.ini"))?)?;
    if metadata.app_id != initial.app_id {
        return Err("The package changed while it was being copied.".into());
    }
    fs::write(appinfo.join("appinfo.ini"), metadata.appinfo)
        .map_err(|e| format!("Could not normalize the staged appinfo.ini: {e}"))?;
    require_signature(&stage.join(&metadata.launcher), is_pe, "a Windows launcher")?;
    require_signature(&stage.join("help.html"), |_| true, "a help file")?;
    if let Some(icon_exe) = metadata.extract_icon {
        require_signature(&stage.join(icon_exe), is_pe, "a Windows icon source")?;
    }
    let app_icon = appinfo.join("appicon.ico");
    require_signature(
        &app_icon,
        |bytes| {
            bytes.len() > 22 && bytes.starts_with(&[0, 0, 1, 0]) && (bytes[4] != 0 || bytes[5] != 0)
        },
        "an ICO icon",
    )?;
    for file in ["appicon_16.png", "appicon_32.png"] {
        require_signature(
            &appinfo.join(file),
            |bytes| bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
            "a PNG icon",
        )?;
    }
    let icon_hash = sha256(&app_icon)?;
    let source = stage.join("Other/Source");
    reject_installer_hooks(&source)?;
    if appinfo.join("plugininstaller.ini").exists() {
        return Err("This command supports full application packages.".into());
    }
    let config = appinfo.join("installer.ini");
    let original = if config.exists() {
        ini_text(&config)?
    } else {
        String::new()
    };
    fs::write(&config, installer_settings(&original)?)
        .map_err(|e| format!("Could not write {}: {e}", config.display()))?;

    // Copy only the maker's inputs. Its mutable Data folder is private to this run.
    let staged_tools = work.join("maker");
    create_dir(&staged_tools)?;
    copy_new_file(&tool_root.join(MAKER), &staged_tools.join(MAKER))?;
    copy_tree(&tool_root.join("App"), &staged_tools.join("App"))?;
    check_tools(&staged_tools)?;
    let maker = run_step(
        &staged_tools.join(MAKER),
        vec![stage.clone()],
        &staged_tools,
        &work.join("maker.log"),
    )?;
    let initial_artifact = find_artifact(&package_work)?;
    let filename = initial_artifact
        .file_name()
        .ok_or("The installer has no file name.")?;
    let script = source.join(INSTALLER_SCRIPT);
    if sha256(&script)? != sha256(&staged_tools.join("App/installer").join(INSTALLER_SCRIPT))? {
        return Err("The generated installer script differs from the maker's template.".into());
    }
    if sha256(&app_icon)? != icon_hash {
        return Err("The application icon changed during installer generation.".into());
    }
    fs::copy(&app_icon, source.join("PortableApps.comInstaller.ico"))
        .map_err(|e| format!("Could not replace the staged installer icon: {e}"))?;
    fs::remove_file(&initial_artifact).map_err(|e| {
        format!("Could not remove the first installer from the working folder: {e}")
    })?;
    check_tools(&staged_tools)?;
    let compiler = run_step(
        &staged_tools.join(COMPILER),
        vec![PathBuf::from("/NOCONFIG"), script],
        &staged_tools.join("App/nsis"),
        &work.join("compiler.log"),
    )?;
    let artifact = find_artifact(&package_work)?;
    if artifact != initial_artifact {
        return Err("The final compiler produced an unexpected installer name.".into());
    }
    super::icons::verify(&app_icon, &artifact)?;
    let size_bytes = metadata_at(&artifact)?.len();
    let digest = sha256(&artifact)?;
    let delivery = work.join("delivery");
    create_dir(&delivery)?;
    fs::rename(&artifact, delivery.join(filename))
        .map_err(|e| format!("Could not prepare the finished installer for delivery: {e}"))?;
    let result = output.join("result");
    let report = CompileReport {
        artifact: result.join(filename),
        size_bytes,
        sha256: digest,
        application_icon_sha256: icon_hash,
        embedded_icons_verified: true,
        installer_version: TOOL_VERSION,
        verified_executables: TOOLS,
        source_package: package.into(),
        tool_root: tool_root.into(),
        redistribution_asserted,
        steps: vec![maker, compiler],
    };
    let manifest = delivery.join("build-manifest.json");
    let data = serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?;
    fs::write(&manifest, data)
        .map_err(|e| format!("Could not write {}: {e}", manifest.display()))?;
    // A Windows directory rename fails when the destination already exists.
    fs::rename(&delivery, &result).map_err(|e| {
        format!(
            "Could not publish the finished installer at {}: {e}",
            result.display()
        )
    })?;
    Ok(report)
}

fn within(path: &Path, root: &Path) -> bool {
    path.components().zip(root.components()).all(|(a, b)| {
        a.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
    }) && path.components().count() >= root.components().count()
}

fn local_directory(path: &Path) -> Result<PathBuf, String> {
    let absolute = std::path::absolute(path).map_err(|e| e.to_string())?;
    for part in absolute.ancestors() {
        if !metadata_at(part)?.is_dir() {
            return Err(format!("Expected a directory at {}.", part.display()));
        }
    }
    let canonical = fs::canonicalize(&absolute)
        .map_err(|e| format!("Could not resolve {}: {e}", path.display()))?;
    let ordinary = canonical
        .to_str()
        .and_then(|p| p.strip_prefix(r"\\?\"))
        .filter(|p| p.as_bytes().get(1..3) == Some(b":\\"))
        .ok_or("Use a local drive path for the package, tool and output folders.")?;
    Ok(ordinary.into())
}

fn metadata_at(path: &Path) -> Result<Metadata, String> {
    let meta = fs::symlink_metadata(path)
        .map_err(|e| format!("Could not read {}: {e}", path.display()))?;
    if meta.file_attributes() & REPARSE_POINT != 0 {
        return Err(format!(
            "Links and redirected paths are unsupported: {}.",
            path.display()
        ));
    }
    Ok(meta)
}

fn require_regular_path(path: &Path, root: &Path) -> Result<(), String> {
    if !metadata_at(path)?.is_file() {
        return Err(format!("Expected a regular file at {}.", path.display()));
    }
    for ancestor in path.ancestors().skip(1).take_while(|p| *p != root) {
        if !metadata_at(ancestor)?.is_dir() {
            return Err(format!("Expected a directory at {}.", ancestor.display()));
        }
    }
    Ok(())
}

fn create_dir(path: &Path) -> Result<(), String> {
    fs::create_dir(path).map_err(|e| format!("Could not create {}: {e}", path.display()))
}

fn entries(path: &Path) -> Result<fs::ReadDir, String> {
    fs::read_dir(path).map_err(|e| format!("Could not list {}: {e}", path.display()))
}

fn copy_new_file(source: &Path, destination: &Path) -> Result<(), String> {
    fn copy(source: &Path, destination: &Path) -> io::Result<()> {
        let mut input = File::open(source)?;
        // Fail closed when a case-sensitive source contains names that collide
        // in the Windows destination.
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(destination)?;
        io::copy(&mut input, &mut output)?;
        if let Ok(modified) = input.metadata()?.modified() {
            output.set_modified(modified)?;
        }
        Ok(())
    }
    copy(source, destination).map_err(|e| {
        format!(
            "Could not copy {} to {}: {e}",
            source.display(),
            destination.display()
        )
    })
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    if !metadata_at(source)?.is_dir() {
        return Err(format!("Expected a directory at {}.", source.display()));
    }
    create_dir(destination)?;
    for entry in entries(source)? {
        let entry = entry.map_err(|e| format!("Could not list {}: {e}", source.display()))?;
        let path = entry.path();
        let name = entry.file_name();
        if !name.to_str().is_some_and(safe_name) {
            return Err(format!(
                "Unsupported file or folder name: {}.",
                path.display()
            ));
        }
        let meta = metadata_at(&path)?;
        let target = destination.join(name);
        if meta.is_dir() {
            copy_tree(&path, &target)?;
        } else if meta.is_file() {
            copy_new_file(&path, &target)?;
        } else {
            return Err(format!(
                "Expected a normal file or directory at {}.",
                path.display()
            ));
        }
    }
    Ok(())
}

fn ini_text(path: &Path) -> Result<String, String> {
    let mut text = String::new();
    File::open(path)
        .and_then(|f| f.take(65_537).read_to_string(&mut text))
        .map_err(|e| format!("Could not read {} as UTF-8: {e}", path.display()))?;
    if text.len() > 65_536 {
        return Err(format!(
            "The settings file {} is too large.",
            path.display()
        ));
    }
    Ok(text)
}

fn reject_installer_hooks(source: &Path) -> Result<(), String> {
    if !source.exists() {
        return Ok(());
    }
    for entry in entries(source)? {
        let entry = entry.map_err(|e| format!("Could not list {}: {e}", source.display()))?;
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if name.ends_with(".nsi")
            || name.ends_with(".nsh")
            || name == "plugininstaller.ini"
            || name == "portableapps.cominstallerlanguages"
        {
            return Err(format!(
                "Remove preexisting installer build inputs from the prepared copy: {}.",
                entry.path().display()
            ));
        }
    }
    Ok(())
}

fn is_pe(bytes: &[u8]) -> bool {
    let Some(offset) = bytes.get(0x3c..0x40).and_then(|b| b.try_into().ok()) else {
        return false;
    };
    // Bound the offset before calling the shared header reader, including
    // when PortableForge is compiled for a 32-bit host.
    u32::from_le_bytes(offset) as usize <= bytes.len().saturating_sub(6) && pe_arch(bytes).is_some()
}

fn require_signature(
    path: &Path,
    test: impl FnOnce(&[u8]) -> bool,
    expected: &str,
) -> Result<(), String> {
    if !metadata_at(path)?.is_file() {
        return Err(format!("Expected {expected} at {}.", path.display()));
    }
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|f| f.take(65_536).read_to_end(&mut bytes))
        .map_err(|e| format!("Could not read {}: {e}", path.display()))?;
    if !test(&bytes) {
        return Err(format!("{} does not look like {expected}.", path.display()));
    }
    Ok(())
}

fn sha256(path: &Path) -> Result<String, String> {
    let mut reader =
        File::open(path).map_err(|e| format!("Could not open {}: {e}", path.display()))?;
    let mut digest = Sha256::new();
    io::copy(&mut reader, &mut digest)
        .map_err(|e| format!("Could not hash {}: {e}", path.display()))?;
    Ok(format!("{:x}", digest.finalize()))
}

fn check_tools(root: &Path) -> Result<(), String> {
    for (relative, expected) in TOOLS {
        let file = root.join(relative);
        require_regular_path(&file, root)?;
        if sha256(&file)? != *expected {
            return Err(format!("Fingerprint mismatch for {}. Use the supported PortableApps.com Installer {TOOL_VERSION} tool folder.", file.display()));
        }
    }
    Ok(())
}

fn find_artifact(work: &Path) -> Result<PathBuf, String> {
    let mut artifact = None;
    for entry in entries(work)? {
        let entry = entry.map_err(|e| format!("Could not list {}: {e}", work.display()))?;
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if name.ends_with(".paf.exe") && !metadata_at(&entry.path())?.is_dir() {
            if artifact.is_some() {
                return Err("More than one installer appeared during the build.".into());
            }
            artifact = Some(entry.path());
        }
    }
    let path = artifact.ok_or("The builder finished without creating an installer.")?;
    require_signature(&path, is_pe, "a Windows installer")?;
    Ok(path)
}

fn run_step(
    executable: &Path,
    arguments: Vec<PathBuf>,
    cwd: &Path,
    log: &Path,
) -> Result<BuildStep, String> {
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let mut output =
        File::create(log).map_err(|e| format!("Could not create {}: {e}", log.display()))?;
    writeln!(
        output,
        "Program: {}\nArguments: {arguments:?}\nWorking folder: {}",
        executable.display(),
        cwd.display()
    )
    .map_err(|e| format!("Could not write {}: {e}", log.display()))?;
    let stderr = output.try_clone().map_err(|e| e.to_string())?;
    let stdout = output.try_clone().map_err(|e| e.to_string())?;
    let mut child = Command::new(executable)
        .args(&arguments)
        .current_dir(cwd)
        .env_remove("NSISDIR")
        .env_remove("NSISCONFDIR")
        .env_remove("APPDATA")
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
        .map_err(|e| format!("Could not start {}: {e}", executable.display()))?;
    extern "system" {
        fn WaitForSingleObject(handle: std::os::windows::io::RawHandle, milliseconds: u32) -> u32;
    }
    const WAIT_OBJECT_0: u32 = 0;
    const WAIT_TIMEOUT: u32 = 258;
    // The owned child keeps the process handle valid throughout this wait.
    let waited = unsafe { WaitForSingleObject(child.as_raw_handle(), TIMEOUT_MILLIS) };
    if waited != WAIT_OBJECT_0 {
        let reason = if waited == WAIT_TIMEOUT {
            format!("Builder timed out after {} seconds", TIMEOUT_MILLIS / 1000)
        } else {
            format!(
                "Could not observe builder exit: {}",
                io::Error::last_os_error()
            )
        };
        let cleanup = child.kill().map_or_else(
            |e| format!("could not stop the builder: {e}"),
            |()| "builder termination requested".into(),
        );
        let _ = writeln!(output, "{reason}. {cleanup}.");
        return Err(format!("{reason}. {cleanup}. See {}.", log.display()));
    }
    let status = child
        .wait()
        .map_err(|e| format!("Could not read builder exit status: {e}"))?;
    writeln!(output, "\nExit status: {status}").map_err(|e| e.to_string())?;
    let exit_code = status.code().ok_or("The builder returned no exit code.")?;
    if !status.success() {
        return Err(format!(
            "{} failed with {status}. See {}.",
            executable.display(),
            log.display()
        ));
    }
    Ok(BuildStep {
        executable: executable.into(),
        arguments,
        working_directory: cwd.into(),
        log: log.into(),
        exit_code,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn staging_preserves_existing_files_and_blocks_installer_hooks() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp = std::env::temp_dir().join(format!(
            "portableforge-compile-{}-{stamp}",
            std::process::id()
        ));
        create_dir(&temp).unwrap();
        let hook = temp.join("pOrTaBlEaPpS.cOmInStAlLeRcUsToM.nSh");
        let destination = temp.join("destination");
        fs::write(&hook, "test fixture").unwrap();
        fs::write(&destination, "original").unwrap();
        assert!(reject_installer_hooks(&temp).is_err());
        assert!(copy_new_file(&hook, &destination).is_err());
        assert_eq!(fs::read_to_string(&destination).unwrap(), "original");
        assert!(find_artifact(&temp).is_err());
        let invalid = temp.join("invalid.paf.exe");
        fs::write(&invalid, b"MZ").unwrap();
        assert!(find_artifact(&temp).is_err());
        fs::remove_file(hook).unwrap();
        fs::remove_file(destination).unwrap();
        fs::remove_file(invalid).unwrap();
        fs::remove_dir(temp).unwrap();
    }
}
