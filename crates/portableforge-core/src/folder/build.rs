//! Prepare a new package from a raw application folder without running it.

use super::{
    open_inventory_file, scan, validate, EntryKind, FolderEntry, FolderScan, IssueSeverity,
};
use crate::artwork::{self, Artwork, IconArtwork, Transform};
use crate::compile::windows::{
    compile_folder_installer, copy_new_file, local_directory, new_output_path,
    preflight_generated_settings, preflight_profile_package, within, FolderCompileChecks,
};
use crate::compile::{read_application_icon, verify_build_result};
use crate::profile::write_new;
use crate::snapshot::{sha256_hex, PeArch};
use crate::validate::{generate_tree, validate_tree};
use crate::wizard_package::{
    compile_folder_launcher, validate_pal_path, verify_installer_toolchain,
    verify_launcher_toolchain,
};
use crate::{Distribution, PafMetadata};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};
use std::time::{SystemTime, UNIX_EPOCH};

const READY: u8 = 0;
const RUNNING: u8 = 1;
const STOPPED: u8 = 2;
const COMMITTING: u8 = 3;
const DONE: u8 = 4;
const STOP_MESSAGE: &str = "Build stopped. No installer was published.";

pub struct FolderBuildRequest<'a> {
    pub scan: &'a FolderScan,
    pub selected: &'a Path,
    pub metadata: &'a PafMetadata,
    pub launcher_tools: &'a Path,
    pub installer_tools: &'a Path,
    pub output: &'a Path,
}

#[derive(Clone, Debug, Default)]
pub struct FolderBuildControl(Arc<AtomicU8>);

impl FolderBuildControl {
    /// Accept a cooperative stop before the publication handshake.
    /// A running trusted tool is allowed to finish its current step.
    pub fn request_stop(&self) -> bool {
        let mut state = self.0.load(Ordering::Acquire);
        while matches!(state, READY | RUNNING) {
            match self
                .0
                .compare_exchange_weak(state, STOPPED, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return true,
                Err(current) => state = current,
            }
        }
        false
    }

    fn begin(&self) -> Result<(), String> {
        self.0
            .compare_exchange(READY, RUNNING, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|state| {
                if state == STOPPED {
                    STOP_MESSAGE.into()
                } else {
                    "This build control has already been used.".into()
                }
            })
    }

    fn active(&self) -> Result<(), String> {
        if self.0.load(Ordering::Acquire) == STOPPED {
            return Err(STOP_MESSAGE.into());
        }
        Ok(())
    }

    fn commit(&self) -> Result<(), String> {
        self.0
            .compare_exchange(RUNNING, COMMITTING, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| STOP_MESSAGE.into())
    }
}

#[derive(Debug)]
pub struct FolderBuildReport {
    pub artifact: PathBuf,
    pub manifest: PathBuf,
    /// SHA-256 of the receipt verified immediately before atomic publication.
    pub manifest_sha256: String,
    pub sha256: String,
    pub size_bytes: u64,
}

/// Build an installer and receipt into a new directory. Files are staged and
/// verified first; only the final same-volume rename publishes the result.
pub fn build(
    request: FolderBuildRequest<'_>,
    control: &FolderBuildControl,
    progress: impl FnMut(&'static str),
) -> Result<FolderBuildReport, String> {
    build_with_artwork(request, None, control, progress)
}

/// Build with optional applied artwork. The images are rendered in the private
/// package stage and verified in the final installer before publication.
pub fn build_with_artwork(
    request: FolderBuildRequest<'_>,
    artwork: Option<&Artwork>,
    control: &FolderBuildControl,
    mut progress: impl FnMut(&'static str),
) -> Result<FolderBuildReport, String> {
    control.begin()?;
    let result = build_inner(request, artwork, control, &mut progress);
    let state = control.0.swap(DONE, Ordering::AcqRel);
    if state == STOPPED {
        match result {
            Err(message) if message.starts_with(STOP_MESSAGE) => Err(message),
            Err(message) => Err(format!("{STOP_MESSAGE}\n{message}")),
            Ok(_) => Err(STOP_MESSAGE.into()),
        }
    } else {
        result
    }
}

fn checkpoint(
    control: &FolderBuildControl,
    progress: &mut dyn FnMut(&'static str),
    stage: &'static str,
) -> Result<(), String> {
    control.active()?;
    progress(stage);
    control.active()
}

fn build_inner(
    request: FolderBuildRequest<'_>,
    artwork: Option<&Artwork>,
    control: &FolderBuildControl,
    progress: &mut dyn FnMut(&'static str),
) -> Result<FolderBuildReport, String> {
    checkpoint(control, progress, "Checking the folder and build options")?;
    let FolderBuildRequest {
        scan: original,
        selected,
        metadata,
        launcher_tools,
        installer_tools,
        output,
    } = request;
    if !metadata.shareable
        || metadata.distribution
            != (Distribution::Bundled {
                redistribution_asserted: true,
            })
    {
        return Err(
            "Confirm that you can redistribute the application files and mark them shareable."
                .into(),
        );
    }
    if original.is_prepared() {
        return Err("Folder build is for raw application folders. Use the prepared-package workflow for this folder.".into());
    }
    reject_validation_errors(original, selected)?;
    let original_source = std::path::absolute(&original.root).map_err(io_message)?;
    let _source_guard = super::hold_directory_chain_with(&original_source, |path| {
        if path == original_source {
            super::hold_directory(path)
        } else {
            super::windows::open_writable_ancestor(path)
        }
    })
    .map_err(|cause| cause.to_string())?;
    let source = local_directory(&original_source)?;
    let current = scan(&source).map_err(|cause| cause.to_string())?;
    if current.is_prepared() || !same_scan(original, &current) {
        return Err(
            "The application folder changed after the scan. Scan it again before building.".into(),
        );
    }
    reject_validation_errors(&current, selected)?;
    let candidate = current
        .candidates
        .iter()
        .find(|candidate| candidate.relative_path == selected)
        .ok_or("Choose a Windows executable from this folder's scan results.")?;
    if !matches!(candidate.architecture, PeArch::X86 | PeArch::X64) {
        return Err("This folder build supports x86 and x64 Windows applications. Choose a compatible executable.".into());
    }
    let mut metadata = metadata.clone();
    metadata.main_exe = launcher_relative(selected)?;
    metadata.main_exe_64bit = (candidate.architecture == PeArch::X64).then_some(true);
    validate_build_metadata(&metadata)?;

    let launcher_tools = local_directory(launcher_tools)?;
    let installer_tools = local_directory(installer_tools)?;
    let output = new_output_path(output)?;
    let output_parent = output
        .parent()
        .ok_or("Choose an output folder with a local parent.")?;
    let _destination_guard =
        super::hold_directory_chain_with(output_parent, super::windows::open_writable_ancestor)
            .map_err(|cause| cause.to_string())?;
    let temp = local_directory(&std::env::temp_dir())?;
    let _temp_guard =
        super::hold_directory_chain_with(&temp, super::windows::open_writable_ancestor)
            .map_err(|cause| cause.to_string())?;
    ensure_new(&output)?;
    require_separate(&[&source, &launcher_tools, &installer_tools, &output])?;
    if within(&temp, &source) || within(&temp, &launcher_tools) || within(&temp, &installer_tools) {
        return Err(
            "The local temporary folder must be outside the application and builder folders."
                .into(),
        );
    }
    checkpoint(control, progress, "Verifying the PortableApps.com builders")?;
    verify_launcher_toolchain(&launcher_tools)?;
    verify_installer_toolchain(&installer_tools)?;
    control.active()?;

    let scratch = owned_path(&temp)?;
    let delivery = owned_path(output_parent)?;
    validate_pal_path(&scratch)?;
    super::windows::create_private_directory(&scratch, true).map_err(|cause| {
        format!(
            "Could not create private build work at {}: {cause}",
            scratch.display()
        )
    })?;
    let mut scratch_identity = None;
    let mut scratch_owned = true;
    let mut delivery_created = false;
    let mut delivery_handle = None;
    let mut delivery_files = Vec::new();
    let outcome = (|| {
        let scratch_guard = super::hold_directory(&scratch).map_err(|cause| cause.to_string())?;
        scratch_identity = Some(super::windows::file_identity(&scratch_guard).map_err(io_message)?);
        let tool_temp = scratch.join("t");
        fs::create_dir(&tool_temp).map_err(io_message)?;
        let package = scratch.join(&metadata.app_id);
        let payload = package.join("App").join(&metadata.base_app_name);
        validate_pal_path(&package)?;
        checkpoint(control, progress, "Copying the application files")?;
        generate_tree(&package, &metadata, &[])?;
        fs::create_dir(&payload).map_err(io_message)?;
        let expected = transfer_payload(&source, &current.entries, Some(&payload), control)?;
        checkpoint(control, progress, "Verifying the copied application files")?;
        let after_copy = scan(&source).map_err(|cause| cause.to_string())?;
        if !same_scan(&current, &after_copy)
            || transfer_payload(&source, &current.entries, None, control)? != expected
        {
            return Err(
                "The application folder changed during the copy. Close the app and scan it again."
                    .into(),
            );
        }
        verify_payload(&payload, &current.entries, &expected, control)?;

        let app_info = package.join("App/AppInfo");
        let staged_artwork = stage_artwork(&package, &payload.join(selected), artwork)?;
        write_launcher_settings(&package, &metadata, selected)?;
        copy_license(&payload, &app_info, &current.entries)?;
        if let Some(issue) = validate_tree(&package, &metadata)
            .into_iter()
            .find(|issue| issue.error)
        {
            return Err(format!("{}: {}", issue.field, issue.message));
        }
        checkpoint(
            control,
            progress,
            "Preparing the PortableApps.com Launcher Generator",
        )?;
        let launcher = package.join(metadata.launcher_name());
        let launcher_step = compile_folder_launcher(
            &package,
            &launcher,
            &launcher_tools,
            &scratch.join("l"),
            &tool_temp,
            || checkpoint(control, progress, "Generating the portable launcher"),
        )?;
        control.active()?;
        preflight_profile_package(&package)?;
        verify_payload(&payload, &current.entries, &expected, control)?;
        staged_artwork.verify(&package)?;

        let verify_staged = |stage: &Path| {
            verify_payload(
                &stage.join("App").join(&metadata.base_app_name),
                &current.entries,
                &expected,
                control,
            )?;
            staged_artwork.verify(stage)
        };
        let mut compiler_checkpoint = |stage| checkpoint(control, progress, stage);
        let compile_output = scratch.join("c");
        let mut compiler = compile_folder_installer(
            &package,
            &installer_tools,
            &compile_output,
            FolderCompileChecks {
                verify_staged: &verify_staged,
                verify_tool_support: &crate::wizard_package::verify_installer_support,
                tool_temp: &tool_temp,
                checkpoint: &mut compiler_checkpoint,
            },
        )?;
        if compiler.application_icon_sha256 != staged_artwork.icon_sha256
            || !compiler.embedded_icons_verified
        {
            return Err("The installer icon does not match the staged application icon.".into());
        }
        let embedded_sidebar_sha256 = compiler
            .installer_artwork
            .as_ref()
            .filter(|evidence| evidence.embedded_verified)
            .map(|evidence| evidence.sha256.as_str());
        if embedded_sidebar_sha256 != staged_artwork.sidebar_sha256.as_deref() {
            return Err("The installer artwork does not match the staged sidebar.".into());
        }
        checkpoint(
            control,
            progress,
            "Preparing the verified installer for delivery",
        )?;

        let file_name = compiler
            .artifact
            .file_name()
            .ok_or("The installer has no file name.")?
            .to_os_string();
        let artifact = output.join(&file_name);
        let manifest = output.join("build-manifest.json");
        super::windows::create_private_directory(&delivery, false).map_err(|cause| {
            format!(
                "Could not create private delivery at {}: {cause}",
                delivery.display()
            )
        })?;
        delivery_created = true;
        delivery_handle = Some(super::windows::own_delivery(&delivery).map_err(io_message)?);
        let staged_artifact = delivery.join(&file_name);
        let mut source_artifact = File::open(&compiler.artifact).map_err(io_message)?;
        let mut artifact_out = create_delivery_file(&staged_artifact, &mut delivery_files)?;
        io::copy(&mut source_artifact, &mut artifact_out).map_err(io_message)?;
        artifact_out.sync_all().map_err(io_message)?;
        drop(artifact_out);
        drop(source_artifact);
        compiler.artifact = artifact.clone();
        let receipt_path = delivery.join("build-manifest.json");
        let mut receipt = serde_json::to_value(&compiler).map_err(|cause| cause.to_string())?;
        if let Some(fields) = receipt.as_object_mut() {
            fields.remove("source_package");
        }
        receipt["folder_build"] = serde_json::json!({
            "schema": 1,
            "source": source,
            "selected_executable": metadata.main_exe,
            "payload_sha256": expected,
            "icon_source": staged_artwork.icon_source,
            "launcher_tools": launcher_tools,
            "launcher_step": launcher_step,
            "support_trees_verified": true,
            "source_application_executed": false,
            "runtime_portability_verified": false,
            "tool_paths_are_historical": true,
            "tool_logs_retained": false,
        });
        let receipt_bytes =
            serde_json::to_vec_pretty(&receipt).map_err(|cause| cause.to_string())?;
        let receipt_sha256 = sha256_hex(&receipt_bytes);
        let mut receipt_out = create_delivery_file(&receipt_path, &mut delivery_files)?;
        receipt_out.write_all(&receipt_bytes).map_err(io_message)?;
        receipt_out.sync_all().map_err(io_message)?;
        drop(receipt_out);
        let verified = verify_build_result(&delivery)?;
        if verified.sha256 != compiler.sha256 || verified.size_bytes != compiler.size_bytes {
            return Err("The delivered installer differs from the verified build.".into());
        }

        drop(scratch_guard);
        remove_owned(&scratch, scratch_identity)?;
        scratch_owned = false;
        checkpoint(control, progress, "Publishing the installer")?;
        let mut files = delivery_files.iter().map(|(path, identity)| {
            super::windows::lock_delivery_file(path, *identity).map_err(io_message)
        });
        let mut artifact_guard = files.next().expect("created installer")?;
        let mut receipt_guard = files.next().expect("created receipt")?;
        let delivered = digest_file(&mut artifact_guard)?;
        if delivered != (compiler.sha256.clone(), compiler.size_bytes)
            || digest_file(&mut receipt_guard)?.0 != receipt_sha256
        {
            return Err("The delivery files changed before publication.".into());
        }
        control.commit()?;
        // Windows requires closed child handles to rename a directory.
        drop((artifact_guard, receipt_guard));
        super::windows::publish_directory(
            delivery_handle.as_ref().expect("owned delivery"),
            &output,
        )
        .map_err(|cause| format!("Could not publish to {}: {cause}", output.display()))?;
        delivery_created = false;
        drop(delivery_handle.take());
        Ok(FolderBuildReport {
            artifact,
            manifest,
            manifest_sha256: receipt_sha256,
            sha256: compiler.sha256,
            size_bytes: compiler.size_bytes,
        })
    })();

    if outcome.is_ok() {
        return outcome;
    }
    let mut message = outcome.expect_err("failed build");
    if control.0.load(Ordering::Acquire) == STOPPED && !message.starts_with(STOP_MESSAGE) {
        message = format!("{STOP_MESSAGE}\n{message}");
    }
    if scratch_owned {
        if let Err(cause) = remove_owned(&scratch, scratch_identity) {
            message.push('\n');
            message.push_str(&cause);
        }
    }
    if delivery_created {
        let discarded = (|| {
            let handle = delivery_handle
                .as_ref()
                .ok_or_else(|| io::Error::other("Ownership could not be confirmed."))?;
            for (path, identity) in delivery_files {
                super::windows::discard_owned_file(&path, identity)?;
            }
            super::windows::discard_owned(handle)
        })();
        if let Err(cause) = discarded {
            message.push_str(&format!(
                "\nCould not clean the delivery folder {}: {cause}",
                delivery.display()
            ));
        }
    }
    Err(message)
}

fn digest_file(file: &mut File) -> Result<(String, u64), String> {
    let mut digest = Sha256::new();
    let size = io::copy(file, &mut digest).map_err(io_message)?;
    Ok((format!("{:x}", digest.finalize()), size))
}

fn create_delivery_file(path: &Path, owned: &mut Vec<(PathBuf, [u32; 3])>) -> Result<File, String> {
    let file = File::options()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(io_message)?;
    let identity = super::windows::file_identity(&file).map_err(io_message)?;
    owned.push((path.into(), identity));
    Ok(file)
}

fn reject_validation_errors(scan: &FolderScan, selected: &Path) -> Result<(), String> {
    if let Some(issue) = validate(scan, Some(selected))
        .into_iter()
        .find(|issue| issue.severity == IssueSeverity::Error)
    {
        return Err(issue.message);
    }
    Ok(())
}

fn same_scan(before: &FolderScan, after: &FolderScan) -> bool {
    before.complete
        && after.complete
        && before.entries == after.entries
        && before.candidates == after.candidates
}

fn require_separate(paths: &[&Path]) -> Result<(), String> {
    for (index, left) in paths.iter().enumerate() {
        if paths[index + 1..]
            .iter()
            .any(|right| within(left, right) || within(right, left))
        {
            return Err("Keep the application, launcher builder, installer builder, and output folders separate.".into());
        }
    }
    Ok(())
}

fn ensure_new(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(cause) if cause.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(cause) => Err(io_message(cause)),
        Ok(_) => Err("Choose a new output folder. Existing files and folders are kept.".into()),
    }
}

fn owned_path(parent: &Path) -> Result<PathBuf, String> {
    let clock = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|cause| cause.to_string())?;
    let name = format!(
        "{}:{}:{}",
        std::process::id(),
        clock.as_nanos(),
        parent.display()
    );
    Ok(parent.join(format!("pf-{}", &sha256_hex(name.as_bytes())[..16])))
}

fn remove_owned(path: &Path, expected: Option<[u32; 3]>) -> Result<(), String> {
    let cleanup = || {
        let guard = super::hold_directory(path).map_err(|cause| cause.to_string())?;
        let actual = super::windows::file_identity(&guard).map_err(io_message)?;
        if Some(actual) != expected {
            return Err("Ownership could not be confirmed.".to_owned());
        }
        drop(guard);
        fs::remove_dir_all(path).map_err(io_message)
    };
    if fs::symlink_metadata(path).is_err_and(|cause| cause.kind() == io::ErrorKind::NotFound) {
        return Ok(());
    }
    cleanup().map_err(|cause| {
        format!(
            "Could not clean the work folder {}: {cause}",
            path.display()
        )
    })
}

fn validate_build_metadata(metadata: &PafMetadata) -> Result<(), String> {
    if !super::safe_name(&metadata.base_app_name)
        || ["AppInfo", "DefaultData"]
            .iter()
            .any(|name| metadata.base_app_name.eq_ignore_ascii_case(name))
        || !launcher_component(&metadata.base_app_name)
    {
        return Err(
            "Use a plain application folder name different from AppInfo or DefaultData.".into(),
        );
    }
    preflight_generated_settings(&metadata.appinfo_ini(), &metadata.installer_ini_with(&[]))
}

fn launcher_component(value: &str) -> bool {
    super::safe_name(value)
        && value
            .chars()
            .all(|ch| ch.is_ascii() && !matches!(ch, '%' | '$' | '\'' | '\u{60}'))
}

fn launcher_relative(path: &Path) -> Result<String, String> {
    let mut names = Vec::new();
    for component in path.components() {
        let std::path::Component::Normal(name) = component else {
            return Err("Choose a file from inside the application folder.".into());
        };
        names.push(
            name.to_str()
                .filter(|name| launcher_component(name))
                .ok_or("Use an executable path without characters reserved by the launcher.")?,
        );
    }
    if names.is_empty() {
        return Err("Choose the executable that starts the application.".into());
    }
    Ok(names.join("\\"))
}

struct StagedArtwork {
    icon_source: &'static str,
    icon_sha256: String,
    icon_size: usize,
    sidebar_sha256: Option<String>,
}

impl StagedArtwork {
    fn verify(&self, package: &Path) -> Result<(), String> {
        let icon_path = package.join("App/AppInfo/appicon.ico");
        let icon = crate::profile::read_small(&icon_path, package, self.icon_size)?;
        if sha256_hex(&icon) != self.icon_sha256 {
            return Err("The application icon changed before installer compilation.".into());
        }

        let sidebar_path = package.join(artwork::SIDEBAR_PATH);
        if let Some(expected) = &self.sidebar_sha256 {
            let sidebar = crate::profile::read_small(&sidebar_path, package, artwork::BMP_BYTES)?;
            artwork::validate_sidebar_bmp(&sidebar)?;
            if sha256_hex(&sidebar) != *expected {
                return Err("The installer sidebar changed before compilation.".into());
            }
        } else if !fs::symlink_metadata(sidebar_path)
            .is_err_and(|error| error.kind() == io::ErrorKind::NotFound)
        {
            return Err("An unexpected installer sidebar appeared before compilation.".into());
        }
        Ok(())
    }
}

fn stage_artwork(
    package: &Path,
    application: &Path,
    artwork: Option<&Artwork>,
) -> Result<StagedArtwork, String> {
    let custom_icon = artwork.and_then(|choice| choice.icon.as_ref());
    let (icon, icon_source) = if let Some(custom) = custom_icon {
        (custom.clone(), "custom")
    } else {
        let (source, origin) = match read_application_icon(application) {
            Ok(source) => (source, "application"),
            Err(_) => (artwork::default_application_icon(), "default"),
        };
        (
            IconArtwork {
                source,
                transform: Transform::default(),
            },
            origin,
        )
    };
    let icon_set = artwork::package_icons(&icon)?;
    let app_info = package.join("App/AppInfo");
    write_new(&app_info.join("appicon.ico"), &icon_set.ico)?;
    for (size, bytes) in &icon_set.png {
        write_new(&app_info.join(format!("appicon_{size}.png")), bytes)?;
    }

    let sidebar_sha256 = artwork
        .and_then(|choice| choice.sidebar.as_ref())
        .map(|sidebar| {
            let effective = if sidebar.source.is_some() {
                None
            } else if custom_icon.is_some() {
                Some(artwork::render_icon(&icon, 256)?)
            } else {
                Some(icon.source)
            };
            let bytes = artwork::sidebar_bmp(sidebar, effective.as_ref())?;
            write_new(&package.join(artwork::SIDEBAR_PATH), &bytes)?;
            Ok::<_, String>(sha256_hex(&bytes))
        })
        .transpose()?;
    Ok(StagedArtwork {
        icon_source,
        icon_sha256: sha256_hex(&icon_set.ico),
        icon_size: icon_set.ico.len(),
        sidebar_sha256,
    })
}

fn write_launcher_settings(
    package: &Path,
    meta: &PafMetadata,
    selected: &Path,
) -> Result<(), String> {
    let ini = package
        .join("App/AppInfo/Launcher")
        .join(format!("{}.ini", meta.app_id));
    let working = selected
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .map(launcher_relative)
        .transpose()?
        .map(|parent| format!("\\{parent}"))
        .unwrap_or_default();
    let settings = meta
        .launcher_ini(&[])
        .replace("DirectoryMoveOK=yes", "DirectoryMoveOK=warn")
        .replace("SupportsUNC=yes", "SupportsUNC=warn");
    let settings = format!(
        "{}WorkingDirectory=%PAL:AppDir%\\{}{working}\n",
        settings, meta.base_app_name
    );
    fs::write(ini, settings).map_err(io_message)?;
    let appinfo = package.join("App/AppInfo/appinfo.ini");
    let original = fs::read_to_string(&appinfo).map_err(io_message)?;
    let mut info = String::new();
    for line in original
        .lines()
        .filter(|line| !line.starts_with("ExtractIcon="))
    {
        info.push_str(line);
        info.push('\n');
    }
    fs::write(appinfo, info).map_err(io_message)
}

fn copy_license(payload: &Path, app_info: &Path, entries: &[FolderEntry]) -> Result<(), String> {
    let license = entries.iter().find(|entry| {
        entry.kind == EntryKind::File
            && entry.relative_path.components().count() == 1
            && entry.relative_path.to_str().is_some_and(|name| {
                [
                    "LICENSE",
                    "LICENSE.txt",
                    "License.rtf",
                    "COPYING",
                    "COPYING.txt",
                ]
                .iter()
                .any(|candidate| name.eq_ignore_ascii_case(candidate))
            })
    });
    if let Some(license) = license {
        copy_new_file(
            &payload.join(&license.relative_path),
            &app_info.join(&license.relative_path),
        )?;
    }
    Ok(())
}

fn verify_payload(
    root: &Path,
    entries: &[FolderEntry],
    expected: &str,
    control: &FolderBuildControl,
) -> Result<(), String> {
    let staged = scan(root).map_err(|cause| cause.to_string())?;
    if !staged.complete
        || staged.entries != entries
        || transfer_payload(root, entries, None, control)? != expected
    {
        return Err("The staged application files differ from the verified folder copy.".into());
    }
    Ok(())
}

fn transfer_payload(
    source: &Path,
    entries: &[FolderEntry],
    destination: Option<&Path>,
    control: &FolderBuildControl,
) -> Result<String, String> {
    let mut digest = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    for entry in entries {
        control.active()?;
        let relative = entry
            .relative_path
            .to_str()
            .ok_or("The scanned file name cannot be represented.")?;
        digest.update((relative.len() as u64).to_le_bytes());
        digest.update(relative.as_bytes());
        if entry.kind == EntryKind::Directory {
            digest.update([0]);
            if let Some(destination) = destination {
                fs::create_dir(destination.join(&entry.relative_path)).map_err(io_message)?;
            }
            continue;
        }
        digest.update([1]);
        digest.update(entry.size_bytes.to_le_bytes());
        let (_guard, input) =
            open_inventory_file(source, &entry.relative_path).map_err(|cause| cause.to_string())?;
        let metadata = input.metadata().map_err(io_message)?;
        if metadata.len() != entry.size_bytes {
            return Err(format!(
                "The application file changed: {}.",
                entry.relative_path.display()
            ));
        }
        let mut output = destination
            .map(|destination| {
                File::options()
                    .create_new(true)
                    .write(true)
                    .open(destination.join(&entry.relative_path))
            })
            .transpose()
            .map_err(io_message)?;
        let mut input = input.take(entry.size_bytes + 1);
        let mut read = 0u64;
        loop {
            control.active()?;
            let count = input.read(&mut buffer).map_err(io_message)?;
            if count == 0 {
                break;
            }
            read += count as u64;
            if read > entry.size_bytes {
                return Err(format!(
                    "The application file grew during the scan: {}.",
                    entry.relative_path.display()
                ));
            }
            digest.update(&buffer[..count]);
            if let Some(output) = &mut output {
                output.write_all(&buffer[..count]).map_err(io_message)?;
            }
        }
        if read != entry.size_bytes {
            return Err(format!(
                "The application file changed: {}.",
                entry.relative_path.display()
            ));
        }
        if let (Some(output), Ok(modified)) = (output, metadata.modified()) {
            output.set_modified(modified).map_err(io_message)?;
        }
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn io_message(cause: io::Error) -> String {
    format!("Could not read or prepare the package files: {cause}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_artwork_staging_uses_the_applied_icon_for_sidebar_composition() {
        let stage = owned_path(&std::env::temp_dir()).unwrap();
        fs::create_dir(&stage).unwrap();
        fs::create_dir_all(stage.join("App/AppInfo")).unwrap();
        fs::create_dir_all(stage.join("Other/Source")).unwrap();

        let icon = IconArtwork {
            source: artwork::default_application_icon(),
            transform: Transform {
                quarter_turns: 1,
                ..Transform::default()
            },
        };
        let sidebar = artwork::SidebarArtwork {
            source: None,
            transform: Transform::default(),
            background: [38, 40, 52],
        };
        let effective = artwork::render_icon(&icon, 256).unwrap();
        let expected_sidebar = artwork::sidebar_bmp(&sidebar, Some(&effective)).unwrap();
        let expected_icon = artwork::package_icons(&icon).unwrap();
        let choice = Artwork {
            icon: Some(icon),
            sidebar: Some(sidebar),
        };
        let staged = stage_artwork(&stage, &stage.join("missing.exe"), Some(&choice)).unwrap();
        assert_eq!(staged.icon_source, "custom");
        assert_eq!(staged.icon_sha256, sha256_hex(&expected_icon.ico));
        assert_eq!(staged.sidebar_sha256, Some(sha256_hex(&expected_sidebar)));
        staged.verify(&stage).unwrap();
        let mut changed = expected_sidebar;
        *changed.last_mut().unwrap() ^= 1;
        fs::write(stage.join(artwork::SIDEBAR_PATH), changed).unwrap();
        let verification = staged.verify(&stage);
        fs::remove_dir_all(&stage).unwrap();
        assert_eq!(
            verification.unwrap_err(),
            "The installer sidebar changed before compilation."
        );
    }

    #[test]
    fn delivery_requires_closed_children_and_protects_existing_output() {
        let parent = owned_path(&std::env::temp_dir()).unwrap();
        fs::create_dir(&parent).unwrap();
        let source = parent.join("stage");
        let destination = parent.join("ready");
        fs::create_dir(&source).unwrap();
        let _parent = super::super::windows::open_writable_ancestor(&parent).unwrap();
        let handle = super::super::windows::own_delivery(&source).unwrap();
        let path = source.join("file");
        let mut owned = Vec::new();
        create_delivery_file(&path, &mut owned)
            .unwrap()
            .write_all(b"owned")
            .unwrap();
        let lock = super::super::windows::lock_delivery_file(&path, owned[0].1).unwrap();
        assert!(File::options().write(true).open(&path).is_err());
        assert!(fs::remove_file(&path).is_err());
        drop(lock);
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("file"), b"existing").unwrap();
        let conflict = super::super::windows::publish_directory(&handle, &destination);
        assert_eq!(fs::read(destination.join("file")).unwrap(), b"existing");
        fs::remove_file(destination.join("file")).unwrap();
        fs::remove_dir(&destination).unwrap();
        let published = super::super::windows::publish_directory(&handle, &destination);
        drop((handle, _parent));
        let bytes = fs::read(destination.join("file"));
        fs::remove_dir_all(&parent).unwrap();
        assert!(conflict.is_err());
        published.unwrap();
        assert_eq!(bytes.unwrap(), b"owned");
    }

    #[test]
    fn published_directory_keeps_exact_utf16_name_and_identity() {
        use std::os::windows::ffi::OsStrExt;

        let parent = owned_path(&std::env::temp_dir()).unwrap();
        fs::create_dir(&parent).unwrap();
        let parent_guard = super::super::windows::open_writable_ancestor(&parent).unwrap();
        let mut residues = Vec::new();
        let mut results = Vec::new();
        for suffix in ["", "a", "ab", "abc"] {
            let source = parent.join("stage");
            let name = format!("Ready-Å-𐐀{suffix}");
            let destination = parent.join(&name);
            residues.push(destination.as_os_str().encode_wide().count() % 4);
            fs::create_dir(&source).unwrap();
            fs::write(source.join("Sample.paf.exe"), b"fixture").unwrap();
            fs::write(source.join("build-manifest.json"), b"receipt").unwrap();
            let handle = super::super::windows::own_delivery(&source).unwrap();
            let identity = super::super::windows::file_identity(&handle).unwrap();
            let published = super::super::windows::publish_directory(&handle, &destination);
            drop(handle);
            let actual: Vec<_> = fs::read_dir(&parent)
                .unwrap()
                .map(|entry| entry.unwrap())
                .collect();
            let exact_name = actual.len() == 1 && actual[0].file_name() == name.as_str();
            let actual_identity = super::super::windows::own_delivery(&destination)
                .and_then(|file| super::super::windows::file_identity(&file));
            let artifact = fs::read(destination.join("Sample.paf.exe"));
            let manifest = fs::read(destination.join("build-manifest.json"));
            for entry in actual {
                fs::remove_dir_all(entry.path()).unwrap();
            }
            results.push((
                published,
                exact_name,
                actual_identity,
                identity,
                artifact,
                manifest,
            ));
        }
        drop(parent_guard);
        fs::remove_dir(&parent).unwrap();
        residues.sort_unstable();
        assert_eq!(residues, [0, 1, 2, 3]);
        for (published, exact_name, actual_identity, identity, artifact, manifest) in results {
            published.unwrap();
            assert!(exact_name);
            assert_eq!(actual_identity.unwrap(), identity);
            assert_eq!(artifact.unwrap(), b"fixture");
            assert_eq!(manifest.unwrap(), b"receipt");
        }
    }
}
