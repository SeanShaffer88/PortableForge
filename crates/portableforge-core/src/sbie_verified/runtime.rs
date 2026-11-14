use super::*;
use crate::profile::Fingerprint;

const MAX_DATA_FILE_CHANGES: usize = 100;

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
    pub data_file_changes: Vec<RuntimeDataFileChange>,
    pub data_file_changes_omitted: usize,
    pub cleanup_verified: bool,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct RuntimeDataFileChange {
    /// File path relative to the installed package's Data directory.
    pub path: String,
    /// None for files that changed only after reopening.
    pub first_launch: Option<RuntimeDataChangeKind>,
    pub reopen: RuntimeDataReopenStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeDataChangeKind {
    Added,
    Updated,
    Removed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeDataReopenStatus {
    Retained,
    ChangedAgain,
    Missing,
    StillRemoved,
    Returned,
    CreatedOnReopen,
    UpdatedOnReopen,
    RemovedOnReopen,
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

#[derive(Default)]
struct DataObservation {
    created_or_changed: usize,
    deleted: usize,
    retained: usize,
    deletion_returned: bool,
    changes: Vec<RuntimeDataFileChange>,
    omitted: usize,
}

impl DataObservation {
    fn first_launch_status(&self) -> RuntimeDataStatus {
        if self.created_or_changed + self.deleted == 0 {
            RuntimeDataStatus::NoDataFilesChanged
        } else if self.retained == self.created_or_changed && !self.deletion_returned {
            RuntimeDataStatus::ObservedChangesRetained
        } else {
            RuntimeDataStatus::ObservedChangesDifferAfterReopen
        }
    }
}

fn check_data(
    before: &BTreeMap<String, Fingerprint>,
    first: &BTreeMap<String, Fingerprint>,
    reopen: &BTreeMap<String, Fingerprint>,
) -> DataObservation {
    use RuntimeDataChangeKind::{Added, Removed, Updated};
    use RuntimeDataReopenStatus::{
        ChangedAgain, CreatedOnReopen, Missing, RemovedOnReopen, Retained, Returned, StillRemoved,
        UpdatedOnReopen,
    };

    let mut observation = DataObservation::default();
    let paths: BTreeSet<_> = before
        .keys()
        .chain(first.keys())
        .chain(reopen.keys())
        .collect();
    for path in paths {
        let baseline = before.get(path);
        let initial = first.get(path);
        let reopened = reopen.get(path);
        let first_launch = match (baseline, initial) {
            (None, Some(_)) => Some(Added),
            (Some(_), None) => Some(Removed),
            (baseline, initial) if baseline != initial => Some(Updated),
            _ => None,
        };
        let reopen = match first_launch {
            Some(Added | Updated) => {
                observation.created_or_changed += 1;
                match reopened {
                    None => Missing,
                    Some(_) if initial == reopened => {
                        observation.retained += 1;
                        Retained
                    }
                    Some(_) => ChangedAgain,
                }
            }
            Some(Removed) => {
                observation.deleted += 1;
                if reopened.is_some() {
                    observation.deletion_returned = true;
                    Returned
                } else {
                    StillRemoved
                }
            }
            None => match (initial, reopened) {
                (None, Some(_)) => CreatedOnReopen,
                (Some(_), None) => RemovedOnReopen,
                (initial, reopened) if initial != reopened => UpdatedOnReopen,
                _ => continue,
            },
        };
        if observation.changes.len() < MAX_DATA_FILE_CHANGES {
            observation.changes.push(RuntimeDataFileChange {
                path: path.clone(),
                first_launch,
                reopen,
            });
        } else {
            observation.omitted += 1;
        }
    }
    observation
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
                sbie.runtime_root().as_os_str().to_os_string(),
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
        let observation = check_data(&before, &after[0], &after[1]);
        Ok::<_, String>(RuntimeReport {
            installer_sha256: fp.sha256,
            install_exit: install.exit_code,
            launch_exits,
            data_files_created_or_changed: observation.created_or_changed,
            data_files_deleted: observation.deleted,
            data_files_unchanged_on_reopen: observation.retained,
            data_status: observation.first_launch_status(),
            data_file_changes: observation.changes,
            data_file_changes_omitted: observation.omitted,
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

    fn files(rows: &[(&str, &str)]) -> BTreeMap<String, Fingerprint> {
        rows.iter()
            .map(|(path, content)| {
                (
                    (*path).to_owned(),
                    Fingerprint {
                        sha256: sha256_hex(content.as_bytes()),
                        size_bytes: content.len() as u64,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn data_observation_distinguishes_retention_from_an_unexercised_setting() {
        let baseline = BTreeMap::new();
        let first = files(&[("settings.ini", "saved")]);
        let untouched = check_data(&baseline, &baseline, &baseline);
        assert!(untouched.changes.is_empty());
        assert_eq!(
            untouched.first_launch_status(),
            RuntimeDataStatus::NoDataFilesChanged
        );
        let retained = check_data(&baseline, &first, &first);
        assert_eq!(
            (
                retained.created_or_changed,
                retained.deleted,
                retained.retained
            ),
            (1, 0, 1)
        );
        assert_eq!(
            retained.first_launch_status(),
            RuntimeDataStatus::ObservedChangesRetained
        );
        let missing = check_data(&baseline, &first, &baseline);
        assert_eq!(
            (
                missing.created_or_changed,
                missing.deleted,
                missing.retained
            ),
            (1, 0, 0)
        );
        assert_eq!(
            missing.first_launch_status(),
            RuntimeDataStatus::ObservedChangesDifferAfterReopen
        );
    }

    #[test]
    fn file_evidence_reports_changes_from_either_launch() {
        use RuntimeDataChangeKind::{Added, Removed, Updated};
        use RuntimeDataReopenStatus::{
            ChangedAgain, CreatedOnReopen, Missing, RemovedOnReopen, Retained, Returned,
            StillRemoved, UpdatedOnReopen,
        };
        let baseline = files(&[
            ("b-changed", "before"),
            ("d-removed", "before"),
            ("e-returned", "before"),
            ("g-reopen-updated", "before"),
            ("h-reopen-removed", "before"),
            ("i-unchanged", "stable"),
        ]);
        let first = files(&[
            ("a-saved", "saved"),
            ("b-changed", "first"),
            ("c-missing", "temporary"),
            ("g-reopen-updated", "before"),
            ("h-reopen-removed", "before"),
            ("i-unchanged", "stable"),
        ]);
        let reopen = files(&[
            ("a-saved", "saved"),
            ("b-changed", "other"),
            ("e-returned", "restored"),
            ("f-reopen-created", "new"),
            ("g-reopen-updated", "after"),
            ("i-unchanged", "stable"),
        ]);
        let result = check_data(&baseline, &first, &reopen);
        let actual: Vec<_> = result
            .changes
            .iter()
            .map(|row| (row.path.as_str(), row.first_launch, row.reopen))
            .collect();
        assert_eq!(
            actual,
            [
                ("a-saved", Some(Added), Retained),
                ("b-changed", Some(Updated), ChangedAgain),
                ("c-missing", Some(Added), Missing),
                ("d-removed", Some(Removed), StillRemoved),
                ("e-returned", Some(Removed), Returned),
                ("f-reopen-created", None, CreatedOnReopen),
                ("g-reopen-updated", None, UpdatedOnReopen),
                ("h-reopen-removed", None, RemovedOnReopen),
            ]
        );
        assert_eq!(
            (result.created_or_changed, result.deleted, result.retained),
            (3, 2, 1)
        );
        assert_eq!(result.omitted, 0);
    }

    #[test]
    fn file_evidence_limit_keeps_exact_totals_and_reopen_only_status() {
        let baseline = BTreeMap::new();
        let first = (0..103)
            .rev()
            .map(|index| {
                (
                    format!("file-{index:03}"),
                    Fingerprint {
                        sha256: "fixture".into(),
                        size_bytes: 1,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let result = check_data(&baseline, &first, &first);
        assert_eq!((result.created_or_changed, result.retained), (103, 103));
        assert_eq!((result.changes.len(), result.omitted), (100, 3));
        assert_eq!(result.changes.first().unwrap().path, "file-000");
        assert_eq!(result.changes.last().unwrap().path, "file-099");
        let reopen_only = check_data(&baseline, &baseline, &first);
        assert_eq!(
            (
                reopen_only.created_or_changed,
                reopen_only.deleted,
                reopen_only.retained
            ),
            (0, 0, 0)
        );
        assert_eq!(
            reopen_only.first_launch_status(),
            RuntimeDataStatus::NoDataFilesChanged
        );
        assert_eq!((reopen_only.changes.len(), reopen_only.omitted), (100, 3));
    }
}
