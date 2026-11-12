use super::*;
use crate::profile::Fingerprint;

pub struct RuntimeRequest<'a> {
    pub installer: &'a Path,
    pub installer_sha256: &'a str,
    pub app_id: &'a str,
    pub output: &'a Path,
    pub timeout: Duration,
}

#[derive(Debug, Serialize)]
pub struct RuntimeReport {
    pub installer_sha256: String,
    pub install_exit: u32,
    pub launch_exits: [u32; 2],
    pub data_files_created_or_changed: usize,
    pub data_files_deleted: usize,
    pub data_files_unchanged_on_reopen: usize,
    pub data_status: RuntimeDataStatus,
    pub cleanup_verified: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeDataStatus {
    NoDataFilesChanged,
    ObservedChangesRetained,
    ObservedChangesDifferAfterReopen,
}

fn data_files(path: &Path) -> Result<BTreeMap<String, Fingerprint>, String> {
    if !optional_directory(path)? {
        return Ok(BTreeMap::new());
    }
    Ok(inventory(path)?
        .into_iter()
        .filter_map(|row| row.file.map(|file| (row.path, file)))
        .collect())
}

fn check_data(
    before: &BTreeMap<String, Fingerprint>,
    first: &BTreeMap<String, Fingerprint>,
    reopen: &BTreeMap<String, Fingerprint>,
) -> (usize, usize, usize, RuntimeDataStatus) {
    let changed = first
        .iter()
        .filter(|(path, file)| before.get(*path) != Some(*file));
    let mut removed = before.keys().filter(|path| !first.contains_key(*path));
    let created_or_changed = changed.clone().count();
    let deleted = removed.clone().count();
    let retained = changed
        .filter(|(path, file)| reopen.get(*path) == Some(*file))
        .count();
    let deletions_retained = removed.all(|path| !reopen.contains_key(path));
    let status = if created_or_changed + deleted == 0 {
        RuntimeDataStatus::NoDataFilesChanged
    } else if retained == created_or_changed && deletions_retained {
        RuntimeDataStatus::ObservedChangesRetained
    } else {
        RuntimeDataStatus::ObservedChangesDifferAfterReopen
    };
    (created_or_changed, deleted, retained, status)
}

/// Test a manifest-verified offline PAF in a fresh box. Finish the installer
/// and close each of the two launcher runs when prompted by their windows.
pub fn test_built_paf(
    request: RuntimeRequest<'_>,
    cancellation: &CancellationToken,
) -> Result<RuntimeReport, String> {
    let RuntimeRequest {
        installer,
        installer_sha256,
        app_id,
        output,
        timeout,
    } = request;
    cancellation.check()?;
    if !safe_name(app_id) || !crate::paf::valid_app_id(app_id) {
        return Err("The build manifest has an invalid application ID.".into());
    }
    let installer = local_file(installer)?;
    let filename = installer
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Missing PAF installer filename.")?;
    let lower = filename.to_ascii_lowercase();
    if !lower.ends_with(".paf.exe") || lower.ends_with("_online.paf.exe") {
        return Err("Choose the verified offline .paf.exe installer.".into());
    }
    let fp = fingerprint(&installer)?;
    if fp.size_bytes == 0 || fp.sha256 != installer_sha256 {
        return Err("The PAF installer differs from the verified build.".into());
    }
    let output = new_output_path(output)?;
    if within(&installer, &output) {
        return Err("Choose a new runtime test folder separate from the build.".into());
    }
    let sbie = Sandboxie::load()?;
    let roots = BoxRoots::new(&system_reg()?)?;
    let host = local_file(&std::env::current_exe().map_err(|e| e.to_string())?)?;
    let name = new_box_name()?;
    let destination = roots.system_drive.join(&name);
    expect_absent(&destination)?;
    let id = output
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Invalid runtime test folder name.")?;
    let layout = session_layout(output.parent().ok_or("Choose a runtime test folder.")?, id);
    mkdir(&layout.root)?;
    mkdir(&layout.input)?;
    let staged = layout.input.join(filename);
    let _installer_lock = sources::copy_locked(&installer, &staged, &fp)?;
    let root = layout.root.join(ROOT_NAME);
    expect_absent(&root)?;
    let owned = serde_json::to_vec_pretty(&serde_json::json!({ "box_name": name, "root": root }))
        .map_err(|error| error.to_string())?;
    write_new(&layout.root.join("sandboxie-owned.json"), &owned)?;
    cancellation.check()?;
    sbie.provision(&name, &root)?;
    let tested = (|| {
        let redirected = root
            .join(BoxRoots::drive_mirror(&roots.system_drive)?)
            .join(&name);
        fs::create_dir_all(&redirected).map_err(|error| io_error(&redirected, error))?;
        if local_directory(&redirected)? != redirected {
            return Err("The redirected runtime install folder changed.".into());
        }
        let installed = destination.join(app_id);
        let launcher = installed.join(format!("{app_id}.exe"));
        let info = installed.join("App").join("AppInfo").join("appinfo.ini");
        let install = checked_exit(
            sbie.run(
                &name,
                &root,
                &staged,
                LaunchArguments::PafDestination(&destination),
                &layout.input,
                RunLimits {
                    timeout,
                    cancellation,
                },
            )?,
            "PAF installation",
        )?;
        sbie.quiescent(&name, QUIET_TIMEOUT)?;
        expect_absent(&destination)?;
        let launcher_physical = roots.logical_file(&root, &launcher)?;
        roots.logical_file(&root, &info)?;
        let data = launcher_physical
            .parent()
            .ok_or("Missing installed app directory.")?
            .join("Data");
        let before = data_files(&data)?;
        let mut launch_exits = [0; 2];
        let mut after = [BTreeMap::new(), BTreeMap::new()];
        for (exit, files) in launch_exits.iter_mut().zip(&mut after) {
            expect_absent(&destination)?;
            roots.logical_file(&root, &launcher)?;
            let args = [
                OsString::from("__sandboxie-exercise"),
                OsString::from(&name),
                root.clone().into_os_string(),
                launcher.clone().into_os_string(),
            ];
            *exit = checked_exit(
                sbie.run(
                    &name,
                    &root,
                    &host,
                    LaunchArguments::Tokens(&args),
                    &layout.input,
                    RunLimits {
                        timeout,
                        cancellation,
                    },
                )?,
                "PAF launcher",
            )?
            .exit_code;
            sbie.quiescent(&name, QUIET_TIMEOUT)?;
            expect_absent(&destination)?;
            cancellation.check()?;
            *files = data_files(&data)?;
        }
        let (created, deleted, retained, data_status) = check_data(&before, &after[0], &after[1]);
        Ok::<_, String>(RuntimeReport {
            installer_sha256: fp.sha256,
            install_exit: install.exit_code,
            launch_exits,
            data_files_created_or_changed: created,
            data_files_deleted: deleted,
            data_files_unchanged_on_reopen: retained,
            data_status,
            cleanup_verified: true,
        })
    })();
    let cleaned = cleanup(&sbie, &name, &root, &layout);
    let report = match (tested, cleaned) {
        (Ok(report), Ok(())) => report,
        (Err(error), Ok(())) => return Err(format!("{error} The dedicated box was cleaned up.")),
        (result, Err(error)) => {
            let cause = result
                .err()
                .unwrap_or_else(|| "Runtime test finished.".into());
            return Err(format!(
                "{cause} Cleanup failed for {name} at {}. {error}",
                root.display()
            ));
        }
    };
    expect_absent(&destination)?;
    cancellation.check()?;
    let evidence = serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?;
    write_new(&layout.root.join("sandboxie-runtime.json"), &evidence)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_observation_distinguishes_retention_from_an_unexercised_setting() {
        let baseline = BTreeMap::new();
        let first = BTreeMap::from([(
            "settings.ini".into(),
            Fingerprint {
                sha256: "ab".into(),
                size_bytes: 1,
            },
        )]);
        assert_eq!(
            check_data(&baseline, &baseline, &baseline).3,
            RuntimeDataStatus::NoDataFilesChanged
        );
        assert_eq!(
            check_data(&baseline, &first, &first),
            (1, 0, 1, RuntimeDataStatus::ObservedChangesRetained)
        );
        assert_eq!(
            check_data(&baseline, &first, &baseline),
            (1, 0, 0, RuntimeDataStatus::ObservedChangesDifferAfterReopen)
        );
    }
}
