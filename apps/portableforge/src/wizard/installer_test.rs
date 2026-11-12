//! Run an installer test from a previously approved build identity.

use std::path::{Path, PathBuf};
use std::time::Duration;

use portableforge_core::compile::{verify_build_result, VerifiedBuild};
use portableforge_core::sbie_verified::{
    preflight, test_built_paf, CancellationToken, RuntimeDataStatus, RuntimeReport, RuntimeRequest,
};
use portableforge_core::settings::SavedBuild;

use super::{sessions_base, unique_folder};

pub(super) struct BuildIdentity<'a> {
    pub(super) directory: &'a Path,
    pub(super) artifact: &'a Path,
    pub(super) manifest_sha256: &'a str,
    pub(super) artifact_sha256: &'a str,
    pub(super) app_id: &'a str,
}

impl<'a> From<&'a SavedBuild> for BuildIdentity<'a> {
    fn from(build: &'a SavedBuild) -> Self {
        Self {
            directory: &build.directory,
            artifact: &build.artifact,
            manifest_sha256: &build.manifest_sha256,
            artifact_sha256: &build.artifact_sha256,
            app_id: &build.app_id,
        }
    }
}

impl BuildIdentity<'_> {
    pub(super) fn check(&self, verified: &VerifiedBuild) -> Result<(), String> {
        if verified.manifest_sha256 != self.manifest_sha256
            || verified.sha256 != self.artifact_sha256
            || verified.app_id.as_deref() != Some(self.app_id)
            || verified.artifact != self.artifact
        {
            return Err("This installer or its build receipt changed since the build.".into());
        }
        Ok(())
    }

    pub(super) fn verify(&self) -> Result<VerifiedBuild, String> {
        let verified = verify_build_result(self.directory)?;
        self.check(&verified)?;
        Ok(verified)
    }
}

#[derive(Debug)]
pub(super) struct InstallerTest {
    report: RuntimeReport,
    output: PathBuf,
}

#[derive(Debug)]
pub(super) enum TestFailure {
    Identity(String),
    Startup(String),
    Failed(String),
    Cleanup(String),
    Cancelled(String),
}

impl TestFailure {
    fn cancelled() -> Self {
        Self::Cancelled("The operation was cancelled.".into())
    }

    fn from_runtime(detail: String) -> Self {
        if detail.contains("Cleanup failed for")
            || detail.contains("Could not roll back the owned box")
        {
            Self::Cleanup(detail)
        } else if detail == "The operation was cancelled."
            || (detail.starts_with("The operation was cancelled.")
                && detail.ends_with("The dedicated box was cleaned up."))
        {
            Self::Cancelled(detail)
        } else {
            Self::Failed(detail)
        }
    }

    pub(super) fn detail(&self) -> &str {
        match self {
            Self::Identity(detail)
            | Self::Startup(detail)
            | Self::Failed(detail)
            | Self::Cleanup(detail)
            | Self::Cancelled(detail) => detail,
        }
    }
}

impl InstallerTest {
    pub(super) fn checked(
        report: RuntimeReport,
        output: PathBuf,
        expected_sha256: &str,
    ) -> Result<Self, TestFailure> {
        if !report.cleanup_verified {
            return Err(TestFailure::Cleanup(
                "Cleanup of the dedicated test box could not be confirmed.".into(),
            ));
        }
        if report.install_exit != 0
            || report.launch_exits != [0, 0]
            || report.installer_sha256 != expected_sha256
        {
            return Err(TestFailure::Failed(
                "The installer test did not finish with the approved build.".into(),
            ));
        }
        Ok(Self { report, output })
    }

    pub(super) fn data_message(&self) -> &'static str {
        match self.report.data_status {
            RuntimeDataStatus::NoDataFilesChanged => {
                "No portable data files changed during the first launch."
            }
            RuntimeDataStatus::ObservedChangesRetained => {
                "The observed portable data changes were still present after reopening."
            }
            RuntimeDataStatus::ObservedChangesDifferAfterReopen => {
                "Some observed portable data changes differed after reopening."
            }
        }
    }

    pub(super) fn details(&self) -> String {
        let report = &self.report;
        let state = self.data_message();
        format!(
            "Installer finished with exit code {}.\nApp launches finished with exit codes {} and {}.\nDedicated box removed · Yes\n\n{state}\nNew or changed data files · {}\nDeleted data files · {}\nUnchanged on reopen · {}\n\nTest folder\n{}",
            report.install_exit,
            report.launch_exits[0],
            report.launch_exits[1],
            report.data_files_created_or_changed,
            report.data_files_deleted,
            report.data_files_unchanged_on_reopen,
            self.output.display(),
        )
    }
}

pub(super) fn run(
    build: BuildIdentity<'_>,
    cancellation: &CancellationToken,
    ready: impl FnOnce(),
) -> Result<InstallerTest, TestFailure> {
    if cancellation.is_cancelled() {
        return Err(TestFailure::cancelled());
    }
    let verified = build.verify().map_err(TestFailure::Identity)?;
    preflight().map_err(TestFailure::Startup)?;
    if cancellation.is_cancelled() {
        return Err(TestFailure::cancelled());
    }
    let parent = sessions_base();
    std::fs::create_dir_all(&parent).map_err(|error| TestFailure::Startup(error.to_string()))?;
    let output = unique_folder(&parent, "Installer-Test").map_err(TestFailure::Startup)?;
    ready();
    let report = test_built_paf(
        RuntimeRequest {
            installer: &verified.artifact,
            installer_sha256: build.artifact_sha256,
            app_id: build.app_id,
            output: &output,
            timeout: Duration::from_secs(600),
        },
        cancellation,
    )
    .map_err(TestFailure::from_runtime)?;
    InstallerTest::checked(report, output, build.artifact_sha256)
}

#[cfg(test)]
mod tests {
    use super::*;
    use portableforge_core::snapshot::sha256_hex;
    use std::fs;

    const TOOLS: &[(&str, &str)] = &[
        (
            "PortableApps.comInstaller.exe",
            "e32cb548713566a2a1f64d51f34b0b9827a4eb8b96256b56a8ecc3ba237c9491",
        ),
        (
            "App/nsis/makensis.exe",
            "b043e554afefbfc56315669d0b4779793aeae67f0f2a7a790e2ea91f05298eff",
        ),
        (
            "App/nsis/Bin/makensis.exe",
            "7144fb5e10113752a92c743b55d0d7bea9c842d4d9cdbfe3d63b092f3c9debf3",
        ),
        (
            "App/bin/MakeHeader.exe",
            "c4d270ddcebd77df693f5cb9112cc5b5372cd1d19a3aec601320d9e982747b6d",
        ),
    ];

    struct InertBuild {
        directory: PathBuf,
        artifact: PathBuf,
        receipt: PathBuf,
    }

    impl Drop for InertBuild {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.artifact);
            let _ = fs::remove_file(&self.receipt);
            let _ = fs::remove_dir(&self.directory);
        }
    }

    #[test]
    fn changed_build_identity_is_rejected_before_sandboxie_preflight() {
        let directory = unique_folder(&std::env::temp_dir(), "pf-test-identity").unwrap();
        fs::create_dir(&directory).unwrap();
        let directory = std::path::absolute(directory).unwrap();
        let build = InertBuild {
            artifact: directory.join("WidgetPortable.paf.exe"),
            receipt: directory.join("build-manifest.json"),
            directory,
        };
        let mut pe = [0; 80];
        pe[..2].copy_from_slice(b"MZ");
        pe[0x3c] = 64;
        pe[64..70].copy_from_slice(b"PE\0\0\x4c\x01");
        fs::write(&build.artifact, pe).unwrap();
        let original_artifact_hash = sha256_hex(&pe);
        let mut record = serde_json::json!({
            "artifact": build.artifact,
            "app_id": "WidgetPortable",
            "size_bytes": pe.len(),
            "sha256": original_artifact_hash,
            "application_icon_sha256": sha256_hex(b"test icon"),
            "embedded_icons_verified": true,
            "installer_version": "3.9.19",
            "verified_executables": TOOLS,
            "redistribution_asserted": true,
            "steps": [
                { "executable": TOOLS[0].0, "exit_code": 0 },
                { "executable": TOOLS[2].0, "exit_code": 0 }
            ],
        });
        let approved_bytes = serde_json::to_vec(&record).unwrap();
        fs::write(&build.receipt, &approved_bytes).unwrap();
        let expected_artifact = verify_build_result(&build.directory).unwrap().artifact;
        let original_manifest_hash = sha256_hex(&approved_bytes);
        let approval = || BuildIdentity {
            directory: &build.directory,
            artifact: &expected_artifact,
            manifest_sha256: &original_manifest_hash,
            artifact_sha256: &original_artifact_hash,
            app_id: "WidgetPortable",
        };
        approval().verify().unwrap();

        let cancellation = CancellationToken::new().unwrap();
        let reject = || {
            let error = run(approval(), &cancellation, || {
                panic!("unexpected test launch")
            })
            .unwrap_err();
            assert!(matches!(error, TestFailure::Identity(_)), "{error:?}");
        };
        let mut changed_receipt = approved_bytes.clone();
        changed_receipt.push(b'\n');
        fs::write(&build.receipt, changed_receipt).unwrap();
        verify_build_result(&build.directory).unwrap();
        reject();

        fs::write(&build.receipt, approved_bytes).unwrap();
        pe[79] = 1;
        fs::write(&build.artifact, pe).unwrap();
        reject();

        record["sha256"] = serde_json::json!(sha256_hex(&pe));
        fs::write(&build.receipt, serde_json::to_vec(&record).unwrap()).unwrap();
        verify_build_result(&build.directory).unwrap();
        reject();
    }

    #[test]
    fn completion_requires_approved_bytes_successful_launches_and_cleanup() {
        let report = |cleanup_verified, launch_exits, installer_sha256: &str| RuntimeReport {
            installer_sha256: installer_sha256.to_owned(),
            install_exit: 0,
            launch_exits,
            data_files_created_or_changed: 0,
            data_files_deleted: 0,
            data_files_unchanged_on_reopen: 0,
            data_status: RuntimeDataStatus::NoDataFilesChanged,
            cleanup_verified,
        };
        for unsuccessful in [
            report(false, [0, 0], "approved"),
            report(true, [1, 0], "approved"),
            report(true, [0, 0], "changed"),
        ] {
            assert!(InstallerTest::checked(unsuccessful, PathBuf::new(), "approved").is_err());
        }
        assert!(InstallerTest::checked(
            report(true, [0, 0], "approved"),
            PathBuf::new(),
            "approved"
        )
        .is_ok());
        assert!(matches!(
            TestFailure::from_runtime(
                "The operation was cancelled. Cleanup failed for the dedicated box.".into()
            ),
            TestFailure::Cleanup(_)
        ));
    }
}
