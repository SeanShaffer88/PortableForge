//! Prepare the wizard's reviewed package and seal it using the CLI's profile format.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::build::build_package;
use crate::compile::windows::{
    copy_new_file, copy_tree, local_directory, new_output_path, preflight_generated_settings,
    preflight_profile_package, run_step, within, CompileReport,
};
use crate::compile::{extract_application_icons, safe_name};
use crate::profile::{
    self, fingerprint, inventory, local_file, tree_sha256, verify_capture, write_new,
};
use crate::review::Review;
use crate::{Classification, Distribution, Finding, PafMetadata};

pub use crate::compile::windows::valid_homepage;

const PAL_EXE: &str = "PortableApps.comLauncherGenerator.exe";
const PAL_SHA256: &str = "66794a62f8bdc8df8a05fa36b9812fc34cac02967ab478e2654c647fe530578d";
const MAKENSIS_EXE: &str = "App/NSIS/makensis.exe";
const MAKENSIS_SHA256: &str = "47cd625d2d90f0cfdff411ba18c39c88ab7e9b28730fc81e5e14ed66ead6fb21";
const INSTALLER_APP_SHA256: &str =
    "326fc0b3d5d5bf2fb47d25c48b92ced9dce6f5706428130731c5e33bba44176f";
// Exact App and Other inventories from the tested official PortableApps.com
// Launcher Generator 2.2.9 bundle. The launcher build runs the NSIS includes
// and plugins from these trees, so executable hashes alone are insufficient.
const PAL_TREES: [(&str, &str); 2] = [
    (
        "App",
        "5fd843353c0247ec70d4a1c24565b916bb5f2e4ea6fb2911458f529f15b59b36",
    ),
    (
        "Other",
        "c196d585c12345e678fd91c5ab4cabca799c7e45c12c2293c2edaddf4cc0867e",
    ),
];

/// Save a profile for the exact capture and review. Invoke this after the
/// operator explicitly approves the reviewed package and redistribution.
/// Select a trusted, installed PortableApps.com Launcher Generator 2.2.9.
pub fn save_reviewed_profile(
    session: &Path,
    meta: &PafMetadata,
    review: &Review,
    launcher_tools: &Path,
    profile_out: &Path,
) -> Result<profile::SavedProfile, String> {
    if !meta.shareable
        || meta.distribution
            != (Distribution::Bundled {
                redistribution_asserted: true,
            })
    {
        return Err("Confirm redistribution and set Shareable for a bundled profile.".into());
    }
    if !safe_name(&meta.app_id)
        || !crate::paf::valid_app_id(&meta.app_id)
        || !safe_name(&meta.base_app_name)
        || ["AppInfo", "DefaultData"]
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&meta.base_app_name))
        || !safe_name(&meta.main_exe)
        || !meta.main_exe.to_ascii_lowercase().ends_with(".exe")
    {
        return Err(
            "Use a plain BaseAppName and select a captured .exe at the program root.".into(),
        );
    }
    let accepted = review.accepted();
    validate_launcher_choices(&accepted)?;
    preflight_generated_settings(&meta.appinfo_ini(), &meta.installer_ini_with(&accepted))?;

    let session = local_directory(session)?;
    let launcher_tools = local_directory(launcher_tools)?;
    let profile_out = new_output_path(profile_out)?;
    if [&session, &launcher_tools]
        .into_iter()
        .any(|root| within(&profile_out, root) || within(root, &profile_out))
        || within(&session, &launcher_tools)
        || within(&launcher_tools, &session)
    {
        return Err(
            "Keep the capture, Launcher Generator and new profile folders separate.".into(),
        );
    }
    match fs::symlink_metadata(&profile_out) {
        Ok(_) => return Err("Choose a new profile folder.".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "Could not check {}: {error}",
                profile_out.display()
            ))
        }
    }

    let capture = verify_capture(&session, review)?;
    // Older receipts do not record the logical executable path. Match
    // installed executables to the verified harvest by name and bytes.
    let harvested_programs: BTreeSet<_> = capture
        .harvest_inventory
        .iter()
        .filter_map(|entry| {
            let file = entry.file.as_ref()?;
            let name = entry.path.rsplit('/').next()?.to_ascii_lowercase();
            name.ends_with(".exe")
                .then_some((name, file.sha256.as_str()))
        })
        .collect();
    for file in capture
        .install_diff
        .real_file_changes()
        .filter_map(|file| file.after.as_ref())
    {
        let name = file
            .path
            .rsplit('\\')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if harvested_programs.contains(&(name, file.sha256.as_str()))
            && accepted
                .iter()
                .any(|item| !supports_review_fix(&item.finding, Some(Path::new(&file.path))))
        {
            return Err("Turn off any reviewed directory move that would move the installed program folder.".into());
        }
    }
    let main_entry = capture
        .harvest_inventory
        .iter()
        .find(|entry| entry.path.eq_ignore_ascii_case(&meta.main_exe))
        .and_then(|entry| entry.file.as_ref())
        .ok_or("Select a captured .exe at the program root.")?;
    let main_exe = local_file(&capture.harvest.join(&meta.main_exe))?;
    if fingerprint(&main_exe)? != *main_entry {
        return Err("The selected executable changed after capture.".into());
    }
    check_launcher_tools(&launcher_tools)?;

    let output = local_directory(&session.join("output"))?;
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("Could not read the build clock: {e}"))?;
    let work = output.join(format!(
        "profile-preparation-{}-{}",
        std::process::id(),
        time.as_nanos()
    ));
    let package = work.join(&meta.app_id);
    validate_pal_path(&work)?;
    validate_pal_path(&package)?;
    create_dir(&work)?;

    let result = (|| {
        let icons = work.join("icons");
        create_dir(&icons)?;
        extract_application_icons(&main_exe, &icons)?;
        if fingerprint(&main_exe)? != *main_entry {
            return Err("The selected executable changed while its icons were read.".into());
        }
        let built = build_package(
            &package,
            meta,
            &accepted,
            Some(&capture.harvest),
            None,
            Some(&icons.join("appicon.ico")),
            false,
        )?;
        if built.has_errors() || !built.skipped.is_empty() {
            return Err(
                "The capture could not be packaged without errors or skipped files.".into(),
            );
        }
        let app_info = package.join("App").join("AppInfo");
        for name in ["appicon_16.png", "appicon_32.png"] {
            copy_new_file(&icons.join(name), &app_info.join(name))?;
        }
        let payload = package.join("App").join(&meta.base_app_name);
        for directory in capture
            .harvest_inventory
            .iter()
            .filter(|entry| entry.file.is_none())
        {
            let path = payload.join(&directory.path);
            fs::create_dir_all(&path)
                .map_err(|e| format!("Could not restore {}: {e}", path.display()))?;
        }
        if inventory(&payload)? != capture.harvest_inventory {
            return Err("The staged program files differ from the verified capture.".into());
        }

        let maker = work.join("maker");
        stage_launcher_tools(&launcher_tools, &maker)?;
        let review_path = work.join("review.json");
        write_new(&review_path, review.to_json().as_bytes())?;
        let launcher = package.join(meta.launcher_name());
        if fs::symlink_metadata(&launcher).is_ok() {
            return Err("The prepared launcher path must be empty before compilation.".into());
        }
        run_step(
            &maker.join(PAL_EXE),
            vec![package.clone()],
            &maker,
            &work.join("launcher-generator.log"),
        )?;
        preflight_profile_package(&package).map_err(|e| {
            format!(
                "{e}\nSee {} for Launcher Generator output.",
                maker
                    .join("Data")
                    .join("PortableApps.comLauncherGeneratorLog.txt")
                    .display()
            )
        })?;
        profile::save(&session, &review_path, &package, &profile_out, true, true)
    })();
    result.map_err(|e| format!("{e}\nPreparation files are in {}.", work.display()))
}

/// Replay a saved approval using the verified PortableApps.com Installer
/// 3.9.19 bundle. The setup file is fingerprinted and is never launched.
pub fn build_reviewed_profile(
    setup: &Path,
    profile: &Path,
    manifest_sha256: &str,
    installer_tools: &Path,
    output: &Path,
) -> Result<CompileReport, String> {
    let output = new_output_path(output)?;
    validate_pal_path(&output)?;
    profile::build_with_tool_check(
        setup,
        profile,
        manifest_sha256,
        installer_tools,
        &output,
        Some(verify_installer_support),
    )
}

fn verify_installer_support(root: &Path) -> Result<(), String> {
    if tree_sha256(&root.join("App"))? != INSTALLER_APP_SHA256 {
        return Err("Select the official PortableApps.com Installer 3.9.19. Its App support files differ from the verified bundle.".into());
    }
    Ok(())
}

fn create_dir(path: &Path) -> Result<(), String> {
    fs::create_dir(path).map_err(|e| format!("Could not create {}: {e}", path.display()))
}

fn validate_pal_path(path: &Path) -> Result<(), String> {
    if !path.to_str().is_some_and(|path| {
        path.chars()
            .all(|c| c.is_ascii() && !c.is_control() && !matches!(c, '$' | '"' | '\'' | '`' | '%'))
    }) {
        return Err(format!(
            "Unsupported characters in Launcher Generator path {}.",
            path.display()
        ));
    }
    Ok(())
}

fn check_launcher_tools(root: &Path) -> Result<(), String> {
    for (file, sha256) in [(PAL_EXE, PAL_SHA256), (MAKENSIS_EXE, MAKENSIS_SHA256)] {
        let path = local_file(&root.join(file))?;
        if fingerprint(&path)?.sha256 != sha256 {
            return Err(format!(
                "Select the official PortableApps.com Launcher Generator 2.2.9. The SHA-256 of {file} differs."
            ));
        }
    }
    Ok(())
}

fn stage_launcher_tools(source: &Path, stage: &Path) -> Result<(), String> {
    create_dir(stage)?;
    for (name, sha256) in PAL_TREES {
        let source_tree = source.join(name);
        let stage_tree = stage.join(name);
        if tree_sha256(&source_tree)? != sha256 {
            return Err(format!(
                "Select the official PortableApps.com Launcher Generator 2.2.9. Its {name} support files differ from the verified bundle."
            ));
        }
        copy_tree(&source_tree, &stage_tree)?;
        if tree_sha256(&stage_tree)? != sha256 {
            return Err(format!(
                "Launcher Generator support files changed while copying {name}."
            ));
        }
    }
    copy_new_file(&source.join(PAL_EXE), &stage.join(PAL_EXE))?;
    check_launcher_tools(stage)
}

fn validate_launcher_choices(accepted: &[Classification]) -> Result<(), String> {
    let mut targets = BTreeSet::new();
    for item in accepted {
        if !supports_review_fix(&item.finding, None) {
            return Err(format!(
                "Turn off the unsupported reviewed fix: {}",
                item.why
            ));
        }
        match &item.finding {
            Finding::DirectoriesMove {
                host_pattern,
                data_target,
                ..
            }
            | Finding::FilesMove {
                host_pattern,
                data_target,
            } => {
                let (prefix, tail) = host_pattern.split_once('\\').unwrap_or((host_pattern, ""));
                if !["%APPDATA%", "%LOCALAPPDATA%", "%USERPROFILE%"].contains(&prefix)
                    || (!tail.is_empty() && !tail.split('\\').all(safe_launcher_component))
                    || !safe_launcher_component(data_target)
                {
                    return Err(format!(
                        "Unsupported reviewed move from {host_pattern} to {data_target}."
                    ));
                }
                let mut target = data_target.to_ascii_lowercase();
                if target == "settings" {
                    if matches!(&item.finding, Finding::FilesMove { .. }) {
                        return Err("The Data settings folder is reserved for the launcher.".into());
                    }
                    target.push_str("data");
                }
                if !targets.insert(target) {
                    return Err(format!(
                        "Conflicting reviewed moves into Data: {data_target}."
                    ));
                }
            }
            Finding::RegistryKey { key } => {
                let tail = key
                    .strip_prefix("HKEY_CURRENT_USER\\")
                    .ok_or("Reviewed registry handling requires a per-user key.")?;
                if !tail.split('\\').all(safe_launcher_component)
                    || !tail
                        .rsplit('\\')
                        .next()
                        .is_some_and(|name| !name.trim_start_matches('.').is_empty())
                {
                    return Err(format!("Unsupported reviewed registry key: {key}."));
                }
            }
            Finding::FileWriteRewrite { path, .. } => {
                if !path
                    .rsplit('\\')
                    .next()
                    .is_some_and(safe_launcher_component)
                {
                    return Err(format!("Unsupported reviewed rewrite path: {path}."));
                }
            }
            Finding::AssociationCandidate { .. } | Finding::RedFlag { .. } => {
                return Err(
                    "The launcher cannot apply the selected association or machine-wide finding."
                        .into(),
                );
            }
            Finding::AppFile { .. } | Finding::UninstallMetadata { .. } => {}
        }
    }
    Ok(())
}

/// Whether a fix can be applied by the verified wizard. Pass the executable
/// selected for capture when available to rule out moving its install folder.
pub fn supports_review_fix(finding: &Finding, installed_exe: Option<&Path>) -> bool {
    match finding {
        Finding::AssociationCandidate { .. } | Finding::RedFlag { .. } => false,
        Finding::RegistryKey { key } => key
            .to_ascii_lowercase()
            .strip_prefix("hkey_current_user\\software\\")
            .and_then(|tail| tail.split('\\').next())
            .is_some_and(|vendor| !matches!(vendor, "" | "classes" | "microsoft" | "policies")),
        Finding::DirectoriesMove {
            host_pattern,
            evidence_file,
            ..
        } => {
            let Some((prefix, tail)) = host_pattern.split_once('\\') else {
                return false;
            };
            let evidence = evidence_file.to_ascii_lowercase();
            let tail = tail.to_ascii_lowercase();
            let location = match prefix {
                "%APPDATA%" => evidence
                    .find("\\appdata\\roaming\\")
                    .map(|i| i + "\\appdata\\roaming\\".len()),
                "%LOCALAPPDATA%" => evidence
                    .find("\\appdata\\local\\")
                    .map(|i| i + "\\appdata\\local\\".len()),
                "%USERPROFILE%" => ["\\documents\\", "\\saved games\\"]
                    .iter()
                    .find_map(|marker| evidence.find(marker).map(|i| i + 1)),
                _ => None,
            };
            let Some(location) = location else {
                return false;
            };
            if tail.is_empty()
                || !evidence[location..]
                    .strip_prefix(&tail)
                    .is_some_and(|suffix| suffix.starts_with('\\'))
            {
                return false;
            }
            let target = format!("{}{tail}\\", &evidence[..location]);
            !installed_exe.is_some_and(|path| {
                path.to_string_lossy()
                    .replace('/', "\\")
                    .to_ascii_lowercase()
                    .starts_with(&target)
            })
        }
        _ => true,
    }
}

fn safe_launcher_component(component: &str) -> bool {
    safe_name(component)
        && component
            .chars()
            .all(|c| c.is_ascii() && !matches!(c, '=' | ';' | '[' | ']' | '$' | '\'' | '`' | '%'))
}
