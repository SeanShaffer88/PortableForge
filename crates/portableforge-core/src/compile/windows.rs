use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read, Write};
use std::os::windows::fs::MetadataExt;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{installer_settings, package_settings, safe_name, Package};
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
    pub app_id: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub application_icon_sha256: String,
    pub embedded_icons_verified: bool,
    pub installer_version: &'static str,
    pub verified_executables: &'static [(&'static str, &'static str)],
    pub source_package: PathBuf,
    pub tool_root: PathBuf,
    pub redistribution_asserted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<ProfileBuildEvidence>,
    pub steps: Vec<BuildStep>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ProfileBuildEvidence {
    pub manifest_sha256: String,
    pub setup_sha256: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inputs: Option<BuildInputEvidence>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BuildInputEvidence {
    pub setup_identity_sha256: String,
    pub package_sha256: String,
    pub payload_sha256: String,
    pub configuration_sha256: String,
    pub icons_sha256: String,
    pub installer_support_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub staged_file_times_sha256: Option<String>,
}

pub(crate) struct CompileApproval<'a> {
    pub evidence: ProfileBuildEvidence,
    pub verify_staged: &'a dyn Fn(&Path) -> Result<String, String>,
    pub verify_tool_support: &'a dyn Fn(&Path) -> Result<(), String>,
    #[cfg(target_arch = "x86_64")]
    pub wizard_checks: bool,
}

#[derive(Debug, Serialize)]
pub struct BuildStep {
    executable: PathBuf,
    arguments: Vec<PathBuf>,
    working_directory: PathBuf,
    log: PathBuf,
    exit_code: i32,
}

/// A local result whose artifact bytes match the selected build manifest.
/// This readback does not grant permission to launch the installer.
#[derive(Debug, Serialize)]
pub struct VerifiedBuild {
    pub artifact: PathBuf,
    pub manifest_sha256: String,
    pub app_id: Option<String>,
    pub size_bytes: u64,
    pub sha256: String,
    pub application_icon_sha256: String,
    pub installer_version: String,
    pub verified_executables: Vec<(String, String)>,
    pub profile: Option<ProfileBuildEvidence>,
    pub compiler_exit_codes: [i32; 2],
}

#[derive(Debug, Serialize)]
pub struct BuildComparison {
    pub left: VerifiedBuild,
    pub right: VerifiedBuild,
    pub same_setup: Option<bool>,
    pub same_package: Option<bool>,
    pub same_payload: Option<bool>,
    pub same_configuration: Option<bool>,
    pub same_icon_set: Option<bool>,
    pub same_primary_icon: bool,
    pub same_installer_support: Option<bool>,
    pub same_staged_file_times: Option<bool>,
    pub same_pinned_executables: bool,
    pub same_compiler_exit_codes: bool,
    pub same_artifact: bool,
}

#[derive(Deserialize)]
struct BuildResultManifest {
    artifact: PathBuf,
    app_id: Option<String>,
    size_bytes: u64,
    sha256: String,
    application_icon_sha256: String,
    embedded_icons_verified: bool,
    installer_version: String,
    verified_executables: Vec<(String, String)>,
    redistribution_asserted: bool,
    profile: Option<ProfileBuildEvidence>,
    steps: Vec<RecordedStep>,
}

#[derive(Deserialize)]
struct RecordedStep {
    executable: PathBuf,
    exit_code: i32,
}

/// Verify an installer under the selected result directory against its
/// bounded build manifest. Manifest paths cannot select another directory.
pub fn verify_build_result(result_directory: &Path) -> Result<VerifiedBuild, String> {
    let result = local_directory(result_directory)?;
    let path = result.join("build-manifest.json");
    require_regular_path(&path, &result)?;
    const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
    let mut bytes = Vec::new();
    File::open(&path)
        .and_then(|file| file.take(MAX_MANIFEST_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err("The build manifest exceeds its size limit.".into());
    }
    let manifest: BuildResultManifest = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Invalid build manifest: {error}"))?;
    let valid_digest =
        |value: &str| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit());
    let valid_profile = |profile: &ProfileBuildEvidence| {
        valid_digest(&profile.manifest_sha256)
            && valid_digest(&profile.setup_sha256)
            && !profile.session_id.trim().is_empty()
            && profile.inputs.as_ref().is_none_or(|inputs| {
                [
                    &inputs.setup_identity_sha256,
                    &inputs.package_sha256,
                    &inputs.payload_sha256,
                    &inputs.configuration_sha256,
                    &inputs.icons_sha256,
                    &inputs.installer_support_sha256,
                ]
                .into_iter()
                .all(|digest| valid_digest(digest))
                    && inputs
                        .staged_file_times_sha256
                        .as_deref()
                        .is_none_or(valid_digest)
            })
    };
    if !manifest.embedded_icons_verified
        || !manifest.redistribution_asserted
        || manifest.installer_version != TOOL_VERSION
        || !valid_digest(&manifest.sha256)
        || !valid_digest(&manifest.application_icon_sha256)
        || manifest
            .app_id
            .as_ref()
            .is_some_and(|id| !safe_name(id) || !crate::paf::valid_app_id(id))
        || manifest
            .profile
            .as_ref()
            .is_some_and(|profile| !valid_profile(profile))
        || manifest.verified_executables.len() != TOOLS.len()
        || manifest.verified_executables.iter().zip(TOOLS).any(
            |((file, digest), (known_file, known_digest))| {
                file != known_file || !digest.eq_ignore_ascii_case(known_digest)
            },
        )
        || manifest.steps.len() != 2
        || manifest
            .steps
            .iter()
            .zip([MAKER, "makensis.exe"])
            .any(|(step, expected)| {
                step.exit_code != 0
                    || step.executable.file_name().and_then(|name| name.to_str()) != Some(expected)
            })
    {
        return Err("The manifest does not describe a verified PortableForge build.".into());
    }
    let artifact = find_artifact(&result)?;
    if artifact.file_name() != manifest.artifact.file_name() {
        return Err("The installer name differs from the build manifest.".into());
    }
    if metadata_at(&artifact)?.len() != manifest.size_bytes {
        return Err("The installer size differs from the build manifest.".into());
    }
    let (sha256, size_bytes) = file_digest(&artifact)?;
    if size_bytes != manifest.size_bytes || !sha256.eq_ignore_ascii_case(&manifest.sha256) {
        return Err("The installer bytes differ from the build manifest.".into());
    }
    Ok(VerifiedBuild {
        artifact,
        manifest_sha256: crate::snapshot::sha256_hex(&bytes),
        app_id: manifest.app_id,
        size_bytes,
        sha256,
        application_icon_sha256: manifest.application_icon_sha256,
        installer_version: manifest.installer_version,
        verified_executables: manifest.verified_executables,
        profile: manifest.profile,
        compiler_exit_codes: [manifest.steps[0].exit_code, manifest.steps[1].exit_code],
    })
}

/// Compare independently read-back results. Missing input evidence from
/// older reports stays unknown. Artifact equality is a separate field.
pub fn compare_build_results(left: &Path, right: &Path) -> Result<BuildComparison, String> {
    let left = verify_build_result(left)?;
    let right = verify_build_result(right)?;
    fn inputs(build: &VerifiedBuild) -> Option<&BuildInputEvidence> {
        build
            .profile
            .as_ref()
            .and_then(|profile| profile.inputs.as_ref())
    }
    let stable_inputs = inputs(&left).zip(inputs(&right));
    let same = |field: fn(&BuildInputEvidence) -> &str| {
        stable_inputs.map(|(left, right)| field(left).eq_ignore_ascii_case(field(right)))
    };
    Ok(BuildComparison {
        same_setup: same(|inputs| &inputs.setup_identity_sha256),
        same_package: same(|inputs| &inputs.package_sha256),
        same_payload: same(|inputs| &inputs.payload_sha256),
        same_configuration: same(|inputs| &inputs.configuration_sha256),
        same_icon_set: same(|inputs| &inputs.icons_sha256),
        same_primary_icon: left
            .application_icon_sha256
            .eq_ignore_ascii_case(&right.application_icon_sha256),
        same_installer_support: same(|inputs| &inputs.installer_support_sha256),
        same_staged_file_times: stable_inputs.and_then(|(left, right)| {
            left.staged_file_times_sha256
                .as_deref()
                .zip(right.staged_file_times_sha256.as_deref())
                .map(|(left, right)| left.eq_ignore_ascii_case(right))
        }),
        same_pinned_executables: left.verified_executables == right.verified_executables,
        same_compiler_exit_codes: left.compiler_exit_codes == right.compiler_exit_codes,
        same_artifact: left.sha256 == right.sha256,
        left,
        right,
    })
}

#[cfg(test)]
mod result_tests {
    use super::*;
    use crate::snapshot::sha256_hex;

    #[test]
    fn build_readback_uses_selected_folder_and_rejects_changed_bytes() {
        let result = std::env::temp_dir().join(format!("pf-build-readback-{}", std::process::id()));
        fs::create_dir(&result).unwrap();
        let mut pe = vec![0; 80];
        pe[..2].copy_from_slice(b"MZ");
        pe[0x3c] = 64;
        pe[64..70].copy_from_slice(b"PE\0\0\x4c\x01");
        let installer = result.join("Widget.paf.exe");
        fs::write(&installer, &pe).unwrap();
        let manifest = serde_json::json!({
            "artifact": r"C:\Earlier location\Widget.paf.exe",
            "size_bytes": pe.len(),
            "sha256": sha256_hex(&pe),
            "application_icon_sha256": sha256_hex(b"icon"),
            "embedded_icons_verified": true,
            "installer_version": TOOL_VERSION,
            "verified_executables": TOOLS,
            "redistribution_asserted": true,
            "steps": [
                { "executable": MAKER, "exit_code": 0 },
                { "executable": COMPILER, "exit_code": 0 }
            ],
        });
        let bytes = serde_json::to_vec(&manifest).unwrap();
        let manifest = result.join("build-manifest.json");
        fs::write(&manifest, &bytes).unwrap();
        let verified = verify_build_result(&result).unwrap();
        assert_eq!(
            verified.artifact,
            local_directory(&result).unwrap().join("Widget.paf.exe")
        );
        assert_eq!(verified.manifest_sha256, sha256_hex(&bytes));
        assert!(verified.app_id.is_none());
        let compare = compare_build_results(&result, &result).unwrap();
        assert!(compare.same_artifact);
        assert!(compare.same_package.is_none());
        pe[79] = 1;
        fs::write(&installer, pe).unwrap();
        assert!(verify_build_result(&result).is_err());
        fs::remove_file(installer).unwrap();
        fs::remove_file(manifest).unwrap();
        fs::remove_dir(result).unwrap();
    }
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
    compile_installer_checked(package, tool_root, output, redistribution_asserted, None)
}

pub(crate) fn compile_installer_checked(
    package: &Path,
    tool_root: &Path,
    output: &Path,
    redistribution_asserted: bool,
    approval: Option<CompileApproval<'_>>,
) -> Result<CompileReport, String> {
    Distribution::Bundled {
        redistribution_asserted,
    }
    .check_guardrail()?;

    let package = local_directory(package)?;
    let tool_root = local_directory(tool_root)?;
    let output = new_output_path(output)?;
    if within(&output, &package)
        || within(&output, &tool_root)
        || within(&package, &tool_root)
        || within(&tool_root, &package)
    {
        return Err("Keep the package, installed tool and new output folders separate.".into());
    }

    check_tools(&tool_root)?;
    if let Some(approval) = &approval {
        (approval.verify_tool_support)(&tool_root)?;
    }
    // Reserve the entire run directory with a single exclusive create. A failed
    // attempt retains its work and cannot be mistaken for a fresh attempt.
    fs::create_dir(&output).map_err(|e| {
        format!(
            "Could not create a new output folder at {}: {e}",
            output.display()
        )
    })?;
    compile_in(
        &package,
        &tool_root,
        &output,
        redistribution_asserted,
        approval,
    )
    .map_err(|e| {
        format!(
            "{e}\nBuild files, if any, are in {}.",
            output.join("work").display()
        )
    })
}

pub(crate) fn new_output_path(output: &Path) -> Result<PathBuf, String> {
    let output_name = output
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| safe_name(name))
        .ok_or("Choose a plain name for the new output folder.")?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok(local_directory(parent)?.join(output_name))
}

fn compile_in(
    package: &Path,
    tool_root: &Path,
    output: &Path,
    redistribution_asserted: bool,
    mut approval: Option<CompileApproval<'_>>,
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
    if let Some(approval) = &mut approval {
        let file_times = (approval.verify_staged)(&stage)?;
        if let Some(inputs) = &mut approval.evidence.inputs {
            inputs.staged_file_times_sha256 = Some(file_times);
        }
    }
    let appinfo = stage.join("App/AppInfo");
    let (metadata, installer_ini) = preflight_package(&stage)?;
    #[cfg(target_arch = "x86_64")]
    if approval
        .as_ref()
        .is_some_and(|approval| approval.wizard_checks)
    {
        preflight_wizard_details(&metadata.appinfo).map_err(|error| {
            format!("{error} Save a new reviewed profile with these details before rebuilding.")
        })?;
    }
    if metadata.app_id != initial.app_id {
        return Err("The package changed while it was being copied.".into());
    }
    fs::write(appinfo.join("appinfo.ini"), metadata.appinfo)
        .map_err(|e| format!("Could not normalize the staged appinfo.ini: {e}"))?;
    let app_icon = appinfo.join("appicon.ico");
    let icon_hash = sha256(&app_icon)?;
    let source = stage.join("Other/Source");
    let config = appinfo.join("installer.ini");
    fs::write(&config, installer_ini)
        .map_err(|e| format!("Could not write {}: {e}", config.display()))?;

    // Copy only the maker's inputs. Its mutable Data folder is private to this run.
    let staged_tools = work.join("maker");
    create_dir(&staged_tools)?;
    copy_new_file(&tool_root.join(MAKER), &staged_tools.join(MAKER))?;
    copy_tree(&tool_root.join("App"), &staged_tools.join("App"))?;
    check_tools(&staged_tools)?;
    if let Some(approval) = &approval {
        (approval.verify_tool_support)(&staged_tools)?;
    }
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
    if let Some(approval) = &approval {
        (approval.verify_tool_support)(&staged_tools)?;
    }
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
        app_id: metadata.app_id,
        size_bytes,
        sha256: digest,
        application_icon_sha256: icon_hash,
        embedded_icons_verified: true,
        installer_version: TOOL_VERSION,
        verified_executables: TOOLS,
        source_package: package.into(),
        tool_root: tool_root.into(),
        redistribution_asserted,
        profile: approval.map(|approval| approval.evidence),
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

fn preflight_package(stage: &Path) -> Result<(Package, String), String> {
    let appinfo = stage.join("App/AppInfo");
    let metadata = package_settings(&ini_text(&appinfo.join("appinfo.ini"))?)?;
    require_signature(&stage.join(&metadata.launcher), is_pe, "a Windows launcher")?;
    require_signature(&stage.join("help.html"), |_| true, "a help file")?;
    if let Some(icon_exe) = &metadata.extract_icon {
        require_signature(&stage.join(icon_exe), is_pe, "a Windows icon source")?;
    }
    require_signature(
        &appinfo.join("appicon.ico"),
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
    reject_installer_hooks(&stage.join("Other/Source"))?;
    if appinfo.join("plugininstaller.ini").exists() {
        return Err("This command supports full application packages.".into());
    }
    let config = appinfo.join("installer.ini");
    let original = if config.exists() {
        ini_text(&config)?
    } else {
        String::new()
    };
    Ok((metadata, installer_settings(&original)?))
}

pub(crate) fn preflight_profile_package(stage: &Path) -> Result<String, String> {
    let (package, _) = preflight_package(stage)?;
    if !safe_name(&package.base_app_name)
        || ["AppInfo", "DefaultData"]
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&package.base_app_name))
    {
        return Err("Give the prepared package a safe BaseAppName for its captured files.".into());
    }
    Ok(package.base_app_name)
}

#[cfg(target_arch = "x86_64")]
pub(crate) fn preflight_generated_settings(appinfo: &str, installer: &str) -> Result<(), String> {
    package_settings(appinfo)?;
    preflight_wizard_details(appinfo)?;
    installer_settings(installer)?;
    Ok(())
}

// Installer 3.9.19 can prompt for these fields even in a positional build.
// The package parser already validates all other prompted app fields.
#[cfg(target_arch = "x86_64")]
fn preflight_wizard_details(appinfo: &str) -> Result<(), String> {
    use super::{read_ini, value, APPINFO_FIELDS};

    let fields = read_ini(appinfo, APPINFO_FIELDS)?;
    if value(&fields, "details", "publisher").is_empty() {
        return Err("Enter the application's Publisher for an unattended build.".into());
    }
    if !valid_homepage(value(&fields, "details", "homepage")) {
        return Err("Enter the application's Homepage as an HTTP(S) address or a domain with an optional path.".into());
    }
    if value(&fields, "details", "description").is_empty() {
        return Err("Enter the application's Description for an unattended build.".into());
    }
    Ok(())
}

/// Accept a website address in the form used by PortableApps.com metadata.
#[cfg(target_arch = "x86_64")]
pub fn valid_homepage(homepage: &str) -> bool {
    let homepage = homepage.trim();
    if !homepage
        .bytes()
        .all(|c| c.is_ascii_graphic() && !b"\\\"'$`<>".contains(&c))
    {
        return false;
    }
    let address = if let Some((scheme, rest)) = homepage.split_once("://") {
        if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
            return false;
        }
        rest
    } else {
        homepage
    };
    let authority = address.split(['/', '?', '#']).next().unwrap_or_default();
    let domain = if let Some((domain, port)) = authority.split_once(':') {
        if !port.parse::<u16>().is_ok_and(|port| port > 0) {
            return false;
        }
        domain
    } else {
        authority
    };
    domain.len() <= 253
        && domain.contains('.')
        && domain.split('.').all(|part| {
            !part.is_empty()
                && part.len() <= 63
                && !part.starts_with('-')
                && !part.ends_with('-')
                && part.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
        })
}

pub(crate) fn within(path: &Path, root: &Path) -> bool {
    path.components().zip(root.components()).all(|(a, b)| {
        a.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
    }) && path.components().count() >= root.components().count()
}

pub(crate) fn local_directory(path: &Path) -> Result<PathBuf, String> {
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

pub(crate) fn metadata_at(path: &Path) -> Result<Metadata, String> {
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

pub(crate) fn require_regular_path(path: &Path, root: &Path) -> Result<(), String> {
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

pub(crate) fn copy_new_file(source: &Path, destination: &Path) -> Result<(), String> {
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

pub(crate) fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
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
    file_digest(path).map(|(sha256, _)| sha256)
}

pub(crate) fn file_digest(path: &Path) -> Result<(String, u64), String> {
    let mut reader =
        File::open(path).map_err(|e| format!("Could not open {}: {e}", path.display()))?;
    let mut digest = Sha256::new();
    let size = io::copy(&mut reader, &mut digest)
        .map_err(|e| format!("Could not hash {}: {e}", path.display()))?;
    Ok((format!("{:x}", digest.finalize()), size))
}

pub(crate) fn check_tools(root: &Path) -> Result<(), String> {
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

pub(crate) fn run_step(
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
