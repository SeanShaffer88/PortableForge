//! Seal an approved Windows Sandbox capture and replay its prepared package.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::compile::safe_name;
use crate::compile::windows::{
    compile_installer_checked, copy_tree, file_digest, local_directory, metadata_at,
    new_output_path, preflight_profile_package, require_regular_path, within, BuildInputEvidence,
    CompileApproval, CompileReport, ProfileBuildEvidence,
};
use crate::orchestrate::{read_capture, session_layout};
use crate::paf::Distribution;
use crate::review::{default_decision, Decision, Review};
use crate::snapshot::sha256_hex;
use crate::{SandboxMode, Session};

const MANIFEST: &str = "profile.json";
const REVIEW: &str = "review.json";
const PACKAGE: &str = "package";
const SCHEMA_VERSION: u32 = 1;
const MAX_MANIFEST: usize = 8 * 1024 * 1024;
const MAX_CAPTURE_NOTE: usize = 32 * 1024 * 1024;
const MAX_ENTRIES: usize = 20_000;
const MAX_DEPTH: usize = 32;
const MAX_BYTES: u64 = 32 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Fingerprint {
    pub(crate) sha256: String,
    pub(crate) size_bytes: u64,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TreeEntry {
    pub(crate) path: String,
    pub(crate) file: Option<Fingerprint>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureEvidence {
    mode: SandboxMode,
    before_sha256: String,
    after_install_sha256: String,
    after_runtime_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sandboxie_receipt_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sandboxie_setup: Option<CapturedSetupInputs>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CapturedSetupInputs {
    arguments_sha256: String,
    companions: Vec<SetupCompanion>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetupCompanion {
    pub name: String,
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    session_id: String,
    capture: CaptureEvidence,
    setup: Fingerprint,
    review_sha256: String,
    redistribution_asserted: bool,
    reviewed_package_approved: bool,
    package: Vec<TreeEntry>,
}

#[derive(Debug)]
pub struct SavedProfile {
    pub manifest: PathBuf,
    pub manifest_sha256: String,
    pub setup_sha256: String,
}

/// Content and provenance of a profile verified against the caller's
/// independent approval fingerprint.
#[derive(Debug, Serialize)]
pub struct ProfileSummary {
    pub manifest_sha256: String,
    pub session_id: String,
    pub capture_mode: SandboxMode,
    pub setup_sha256: String,
    pub setup_size_bytes: u64,
    pub review_sha256: String,
    pub package_sha256: String,
    pub package_file_count: usize,
    pub setup_arguments_sha256: Option<String>,
    pub setup_companions: Vec<SetupCompanion>,
}

#[derive(Debug, Serialize)]
pub struct CaptureComparison {
    pub previous_setup_sha256: String,
    pub current_setup_sha256: String,
    pub setup_companions_changed: bool,
    pub setup_arguments_changed: bool,
    pub payload_changes: Vec<PayloadChange>,
    pub previous_review: Review,
    pub current_review: Review,
}

#[derive(Debug, Serialize)]
pub struct PayloadChange {
    pub relative_path: String,
    pub previous_sha256: Option<String>,
    pub current_sha256: Option<String>,
}

/// Seal the reviewed package in a new directory outside the session.
/// Close the Sandbox and keep the local inputs stable before saving.
pub fn save(
    session: &Path,
    review: &Path,
    package: &Path,
    output: &Path,
    redistribution_asserted: bool,
    reviewed_package_approved: bool,
) -> Result<SavedProfile, String> {
    Distribution::Bundled {
        redistribution_asserted,
    }
    .check_guardrail()?;
    if !reviewed_package_approved {
        return Err(
            "Confirm that the prepared package implements the choices you reviewed.".into(),
        );
    }
    let session = local_directory(session)?;
    let package = local_directory(package)?;
    let review = local_file(review)?;
    let output = new_output_path(output)?;
    if [&session, &package]
        .into_iter()
        .any(|root| within(&output, root) || within(root, &output))
        || within(&review, &output)
    {
        return Err(
            "Keep the new profile folder separate from the capture, review and package.".into(),
        );
    }
    let layout = capture_layout(&session)?;
    if [&layout.input, &layout.exchange, &layout.tools]
        .into_iter()
        .any(|root| within(&review, root) || within(&package, root) || within(root, &package))
    {
        return Err(
            "Keep the approved review and prepared package outside the box's shared folders."
                .into(),
        );
    }
    let review_bytes = read_small(
        &review,
        review.parent().ok_or("Choose a review file.")?,
        MAX_MANIFEST,
    )?;
    let approved =
        Review::from_json(std::str::from_utf8(&review_bytes).map_err(|e| e.to_string())?)?;
    let capture = verify_capture(&session, &approved)?;

    let package_before = inventory(&package)?;
    let base_app_name = preflight_profile_package(&package)?;
    if capture.harvest_inventory != inventory(&package.join("App").join(base_app_name))? {
        return Err("The prepared package's program files differ from the captured files.".into());
    }
    let manifest = Manifest {
        schema_version: SCHEMA_VERSION,
        session_id: capture.session_id,
        capture: capture.evidence,
        setup: capture.setup,
        review_sha256: sha256_hex(&review_bytes),
        redistribution_asserted,
        reviewed_package_approved,
        package: package_before,
    };
    let bytes = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_MANIFEST {
        return Err("The profile inventory exceeds the size limit.".into());
    }

    fs::create_dir(&output).map_err(|e| {
        format!(
            "Could not create new profile folder {}: {e}",
            output.display()
        )
    })?;
    let staged = output.join(PACKAGE);
    copy_tree(&package, &staged)?;
    verify_inventory(&staged, &manifest.package)?;
    write_new(&output.join(REVIEW), &review_bytes)?;
    // Write the approval manifest last. Failed attempts leave their fresh
    // folder for inspection and never return an approval fingerprint.
    let manifest_path = output.join(MANIFEST);
    write_new(&manifest_path, &bytes)?;
    Ok(SavedProfile {
        manifest: manifest_path,
        manifest_sha256: sha256_hex(&bytes),
        setup_sha256: manifest.setup.sha256,
    })
}

fn capture_layout(session: &Path) -> Result<crate::orchestrate::SessionLayout, String> {
    let parent = session.parent().ok_or("Choose a capture session folder.")?;
    let folder = session
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Choose a capture session folder.")?;
    Ok(session_layout(parent, folder))
}

pub(crate) struct VerifiedCapture {
    session_id: String,
    setup: Fingerprint,
    evidence: CaptureEvidence,
    pub(crate) harvest: PathBuf,
    pub(crate) harvest_inventory: Vec<TreeEntry>,
    #[cfg(target_arch = "x86_64")]
    pub(crate) install_diff: crate::diff::Diff,
}

pub(crate) fn verify_capture(session: &Path, approved: &Review) -> Result<VerifiedCapture, String> {
    let layout = capture_layout(session)?;
    let session_file = read_small(&layout.session_path, session, MAX_MANIFEST)?;
    let session_record =
        Session::from_json(std::str::from_utf8(&session_file).map_err(|e| e.to_string())?)?;
    if !matches!(
        session_record.sandbox_mode,
        SandboxMode::WindowsSandbox | SandboxMode::Sandboxie
    ) {
        return Err(
            "Save a profile from Windows Sandbox or a verified Sandboxie-Plus capture.".into(),
        );
    }
    if session_record.id.trim().is_empty() {
        return Err("The capture session has no identifier.".into());
    }
    let input = local_directory(&layout.input)?;
    let setup = only_setup(&input)?;
    let harvest = local_directory(&layout.exchange.join("harvest"))?;
    let harvest_before = inventory(&harvest)?;
    if !harvest_before.iter().any(|entry| entry.file.is_some()) {
        return Err("The capture has no harvested program files.".into());
    }

    let note_hash = |name| {
        read_small(&layout.exchange.join(name), session, MAX_CAPTURE_NOTE)
            .map(|bytes| sha256_hex(&bytes))
    };
    let capture_evidence = || -> Result<CaptureEvidence, String> {
        let mut evidence = CaptureEvidence {
            mode: session_record.sandbox_mode,
            before_sha256: note_hash("before.json")?,
            after_install_sha256: note_hash("after-install.json")?,
            after_runtime_sha256: note_hash("after-runtime.json")?,
            sandboxie_receipt_sha256: None,
            sandboxie_setup: None,
        };
        if evidence.mode == SandboxMode::Sandboxie {
            let (receipt_digest, inputs) = sandboxie_receipt_evidence(
                session,
                &session_record.id,
                &setup.sha256,
                &evidence,
                &harvest_before,
            )?;
            evidence.sandboxie_receipt_sha256 = Some(receipt_digest);
            evidence.sandboxie_setup = inputs;
        }
        Ok(evidence)
    };
    let notes_before = capture_evidence()?;
    let observed = read_capture(&layout)?;
    verify_review(approved, &observed.review)?;
    if notes_before != capture_evidence()? || harvest_before != inventory(&harvest)? {
        return Err(
            "The capture changed while it was being reviewed. Close the test box and try again."
                .into(),
        );
    }

    Ok(VerifiedCapture {
        session_id: session_record.id,
        setup,
        evidence: notes_before,
        harvest,
        harvest_inventory: harvest_before,
        #[cfg(target_arch = "x86_64")]
        install_diff: observed.install_diff,
    })
}

/// Build from a saved profile. The expected manifest hash comes from the
/// operator's record outside the profile. The setup file is only read.
pub fn build(
    setup: &Path,
    profile: &Path,
    manifest_sha256: &str,
    tool_root: &Path,
    output: &Path,
) -> Result<CompileReport, String> {
    build_with_tool_check(setup, profile, manifest_sha256, tool_root, output, None)
}

pub(crate) fn build_with_tool_check(
    setup: &Path,
    profile: &Path,
    manifest_sha256: &str,
    tool_root: &Path,
    output: &Path,
    installer_support_pin: Option<&str>,
) -> Result<CompileReport, String> {
    let profile = local_directory(profile)?;
    let setup = local_file(setup)?;
    let output = new_output_path(output)?;
    if within(&output, &profile) || within(&profile, &output) || within(&setup, &output) {
        return Err("Keep the new build folder separate from the profile and setup file.".into());
    }
    let bytes = read_small(&profile.join(MANIFEST), &profile, MAX_MANIFEST)?;
    let manifest = approved_manifest(&bytes, manifest_sha256)?;
    let actual_setup = fingerprint(&setup)?;
    if actual_setup != manifest.setup {
        return Err("This setup file does not match the approved SHA-256. Review this version before building.".into());
    }
    if let Some(inputs) = &manifest.capture.sandboxie_setup {
        verify_companions(&setup, &inputs.companions)?;
    }
    verify_profile_contents(&profile, &manifest)?;
    let tool_root = local_directory(tool_root)?;
    let support_sha256 = tree_sha256(&tool_root.join("App"))?;
    if installer_support_pin.is_some_and(|expected| expected != support_sha256) {
        return Err("Select the official PortableApps.com Installer 3.9.19. Its App support files differ from the verified bundle.".into());
    }
    let inputs = profile_build_inputs(&manifest, support_sha256.clone())?;
    let package = profile.join(PACKAGE);
    let verify_support = |root: &Path| {
        if tree_sha256(&root.join("App"))? != support_sha256 {
            return Err("The Installer support files changed while preparing the build.".into());
        }
        Ok(())
    };
    let approval = CompileApproval {
        evidence: ProfileBuildEvidence {
            manifest_sha256: sha256_hex(&bytes),
            setup_sha256: actual_setup.sha256,
            session_id: manifest.session_id,
            inputs: Some(inputs),
        },
        verify_staged: &|stage| {
            verify_inventory(stage, &manifest.package)?;
            staged_file_times(stage, &manifest.package)
        },
        verify_tool_support: &verify_support,
        #[cfg(target_arch = "x86_64")]
        wizard_checks: installer_support_pin.is_some(),
    };
    compile_installer_checked(
        &package,
        &tool_root,
        &output,
        manifest.redistribution_asserted,
        Some(approval),
    )
}

fn profile_build_inputs(
    manifest: &Manifest,
    installer_support_sha256: String,
) -> Result<BuildInputEvidence, String> {
    let digest = |entries: Vec<&TreeEntry>| {
        serde_json::to_vec(&entries)
            .map(|bytes| sha256_hex(&bytes))
            .map_err(|error| error.to_string())
    };
    let in_appinfo = |path: &str| {
        let mut parts = path.split('/');
        parts
            .next()
            .is_some_and(|part| part.eq_ignore_ascii_case("App"))
            && parts
                .next()
                .is_some_and(|part| part.eq_ignore_ascii_case("AppInfo"))
    };
    let icon = |path: &str| {
        if !in_appinfo(path) {
            return false;
        }
        let name = path
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        name == "appicon.ico" || (name.starts_with("appicon_") && name.ends_with(".png"))
    };
    let entries = &manifest.package;
    let setup = serde_json::to_vec(&(&manifest.setup, &manifest.capture.sandboxie_setup))
        .map_err(|error| error.to_string())?;
    Ok(BuildInputEvidence {
        setup_identity_sha256: sha256_hex(&setup),
        package_sha256: inventory_sha256(entries)?,
        payload_sha256: digest(
            entries
                .iter()
                .filter(|entry| {
                    entry
                        .path
                        .split('/')
                        .next()
                        .is_some_and(|part| part.eq_ignore_ascii_case("App"))
                        && !in_appinfo(&entry.path)
                })
                .collect(),
        )?,
        configuration_sha256: digest(
            entries
                .iter()
                .filter(|entry| in_appinfo(&entry.path) && !icon(&entry.path))
                .collect(),
        )?,
        icons_sha256: digest(entries.iter().filter(|entry| icon(&entry.path)).collect())?,
        installer_support_sha256,
        staged_file_times_sha256: None,
    })
}

fn staged_file_times(root: &Path, entries: &[TreeEntry]) -> Result<String, String> {
    use std::time::UNIX_EPOCH;

    let mut times = Vec::new();
    for entry in entries.iter().filter(|entry| entry.file.is_some()) {
        let path = root.join(&entry.path);
        let time = metadata_at(&path)?.modified().map_err(|error| {
            format!(
                "Could not read the staged file time at {}: {error}",
                path.display()
            )
        })?;
        let nanos = time.duration_since(UNIX_EPOCH).map_or_else(
            |error| -(error.duration().as_nanos() as i128),
            |duration| duration.as_nanos() as i128,
        );
        times.push((&entry.path, nanos));
    }
    serde_json::to_vec(&times)
        .map(|bytes| sha256_hex(&bytes))
        .map_err(|error| error.to_string())
}

/// Inspect a verified profile without changing its contents.
pub fn inspect(profile: &Path, manifest_sha256: &str) -> Result<ProfileSummary, String> {
    let root = local_directory(profile)?;
    let bytes = read_small(&root.join(MANIFEST), &root, MAX_MANIFEST)?;
    let manifest = approved_manifest(&bytes, manifest_sha256)?;
    verify_profile_contents(&root, &manifest)?;
    let inputs = manifest.capture.sandboxie_setup;
    Ok(ProfileSummary {
        manifest_sha256: sha256_hex(&bytes),
        session_id: manifest.session_id,
        capture_mode: manifest.capture.mode,
        setup_sha256: manifest.setup.sha256,
        setup_size_bytes: manifest.setup.size_bytes,
        review_sha256: manifest.review_sha256,
        package_sha256: inventory_sha256(&manifest.package)?,
        package_file_count: manifest
            .package
            .iter()
            .filter(|entry| entry.file.is_some())
            .count(),
        setup_arguments_sha256: inputs
            .as_ref()
            .map(|inputs| inputs.arguments_sha256.clone()),
        setup_companions: inputs.map_or_else(Vec::new, |inputs| inputs.companions),
    })
}

/// Compare a reviewed new capture with a verified saved profile. This
/// reports changes without writing or carrying over any approval.
pub fn compare_capture(
    old_profile: &Path,
    old_approval_sha: &str,
    new_session: &Path,
    current_review: &Review,
) -> Result<CaptureComparison, String> {
    let root = local_directory(old_profile)?;
    let bytes = read_small(&root.join(MANIFEST), &root, MAX_MANIFEST)?;
    let old = approved_manifest(&bytes, old_approval_sha)?;
    let review = verify_profile_contents(&root, &old)?;
    let previous_review =
        Review::from_json(std::str::from_utf8(&review).map_err(|error| error.to_string())?)?;
    let session = local_directory(new_session)?;
    let current = verify_capture(&session, current_review)?;
    let base = preflight_profile_package(&root.join(PACKAGE))?;
    let previous = old.package.iter().filter_map(|entry| {
        let mut parts = entry.path.splitn(3, '/');
        let app = parts.next()?;
        let app_name = parts.next()?;
        let path = parts.next()?;
        (app.eq_ignore_ascii_case("App") && app_name.eq_ignore_ascii_case(&base))
            .then_some((path, entry.file.as_ref()?))
    });
    let next = current
        .harvest_inventory
        .iter()
        .filter_map(|entry| Some((entry.path.as_str(), entry.file.as_ref()?)));
    let previous: BTreeMap<_, _> = previous.collect();
    let next: BTreeMap<_, _> = next.collect();
    let paths: BTreeSet<_> = previous.keys().chain(next.keys()).copied().collect();
    let payload_changes = paths
        .into_iter()
        .filter_map(|path| {
            let before = previous.get(path);
            let after = next.get(path);
            (before != after).then(|| PayloadChange {
                relative_path: path.to_owned(),
                previous_sha256: before.map(|item| item.sha256.clone()),
                current_sha256: after.map(|item| item.sha256.clone()),
            })
        })
        .collect();
    let old_inputs = old.capture.sandboxie_setup.as_ref();
    let new_inputs = current.evidence.sandboxie_setup.as_ref();
    Ok(CaptureComparison {
        previous_setup_sha256: old.setup.sha256,
        current_setup_sha256: current.setup.sha256,
        setup_companions_changed: old_inputs
            .map(|inputs| inputs.companions.as_slice())
            .unwrap_or_default()
            != new_inputs
                .map(|inputs| inputs.companions.as_slice())
                .unwrap_or_default(),
        setup_arguments_changed: old_inputs.map(|inputs| &inputs.arguments_sha256)
            != new_inputs.map(|inputs| &inputs.arguments_sha256),
        payload_changes,
        previous_review,
        current_review: current_review.clone(),
    })
}

/// Copy a verified profile into a fresh location while preserving its
/// original approval. The source and copied bytes use the same pin.
pub fn relocate(
    profile: &Path,
    manifest_sha256: &str,
    output: &Path,
) -> Result<SavedProfile, String> {
    let root = local_directory(profile)?;
    let output = new_output_path(output)?;
    if within(&output, &root) || within(&root, &output) {
        return Err("Choose a new profile folder outside the source profile.".into());
    }
    let bytes = read_small(&root.join(MANIFEST), &root, MAX_MANIFEST)?;
    let manifest = approved_manifest(&bytes, manifest_sha256)?;
    let review = verify_profile_contents(&root, &manifest)?;
    fs::create_dir(&output).map_err(|error| {
        format!(
            "Could not create new profile folder {}: {error}",
            output.display()
        )
    })?;
    copy_tree(&root.join(PACKAGE), &output.join(PACKAGE))?;
    verify_inventory(&output.join(PACKAGE), &manifest.package)?;
    write_new(&output.join(REVIEW), &review)?;
    let manifest_path = output.join(MANIFEST);
    write_new(&manifest_path, &bytes)?;
    Ok(SavedProfile {
        manifest: manifest_path,
        manifest_sha256: sha256_hex(&bytes),
        setup_sha256: manifest.setup.sha256,
    })
}

fn verify_profile_contents(profile: &Path, manifest: &Manifest) -> Result<Vec<u8>, String> {
    let mut expected_names = BTreeSet::from([MANIFEST, REVIEW, PACKAGE]);
    for entry in
        fs::read_dir(profile).map_err(|e| format!("Could not list {}: {e}", profile.display()))?
    {
        let name = entry
            .map_err(|e| format!("Could not read a profile entry: {e}"))?
            .file_name();
        if !name
            .to_str()
            .is_some_and(|name| expected_names.remove(name))
        {
            return Err("The profile folder has unexpected files.".into());
        }
    }
    if !expected_names.is_empty() {
        return Err("The profile folder is incomplete.".into());
    }
    let review = read_small(&profile.join(REVIEW), profile, MAX_MANIFEST)?;
    if sha256_hex(&review) != manifest.review_sha256 {
        return Err("The saved review has changed since approval.".into());
    }
    verify_inventory(&profile.join(PACKAGE), &manifest.package)?;
    Ok(review)
}

fn approved_manifest(bytes: &[u8], expected: &str) -> Result<Manifest, String> {
    if !valid_digest(expected) || !sha256_hex(bytes).eq_ignore_ascii_case(expected) {
        return Err("The profile manifest does not match its saved SHA-256.".into());
    }
    let manifest: Manifest =
        serde_json::from_slice(bytes).map_err(|e| format!("Invalid profile manifest: {e}"))?;
    if manifest.schema_version != SCHEMA_VERSION
        || !matches!(
            (
                manifest.capture.mode,
                &manifest.capture.sandboxie_receipt_sha256
            ),
            (SandboxMode::WindowsSandbox, None) | (SandboxMode::Sandboxie, Some(_))
        )
        || manifest
            .capture
            .sandboxie_receipt_sha256
            .as_deref()
            .is_some_and(|hash| !valid_digest(hash))
        || manifest
            .capture
            .sandboxie_setup
            .as_ref()
            .is_some_and(|setup| {
                manifest.capture.mode != SandboxMode::Sandboxie || !valid_setup_inputs(setup)
            })
        || [
            &manifest.capture.before_sha256,
            &manifest.capture.after_install_sha256,
            &manifest.capture.after_runtime_sha256,
        ]
        .into_iter()
        .any(|digest| !valid_digest(digest))
        || !manifest.redistribution_asserted
        || !manifest.reviewed_package_approved
        || !valid_digest(&manifest.setup.sha256)
        || !valid_digest(&manifest.review_sha256)
        || manifest.setup.size_bytes == 0
        || manifest.setup.size_bytes > MAX_BYTES
        || manifest.session_id.trim().is_empty()
        || manifest.package.is_empty()
        || manifest.package.len() > MAX_ENTRIES
    {
        return Err("The profile has an unsupported format or is missing its approval.".into());
    }
    Ok(manifest)
}

pub(crate) fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_setup_inputs(inputs: &CapturedSetupInputs) -> bool {
    let mut names = BTreeSet::new();
    valid_digest(&inputs.arguments_sha256)
        && inputs.companions.len() <= 64
        && inputs.companions.iter().all(|companion| {
            safe_name(&companion.name)
                && names.insert(companion.name.to_uppercase())
                && valid_digest(&companion.sha256)
                && companion.size_bytes <= MAX_BYTES
        })
}

fn verify_companions(setup: &Path, companions: &[SetupCompanion]) -> Result<(), String> {
    let parent = setup
        .parent()
        .ok_or("Choose a setup file with a parent folder.")?;
    let name = setup
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Choose a setup file with a name.")?;
    for companion in companions {
        if name.to_uppercase() == companion.name.to_uppercase() {
            return Err("The setup file and an approved companion need separate names.".into());
        }
        let file = local_file(&parent.join(&companion.name))?;
        let actual = fingerprint(&file)?;
        if actual.sha256 != companion.sha256 || actual.size_bytes != companion.size_bytes {
            return Err(format!(
                "Setup companion {} differs from its approved bytes.",
                companion.name
            ));
        }
    }
    Ok(())
}

fn verify_review(approved: &Review, observed: &Review) -> Result<(), String> {
    let cards = |review: &Review| {
        let mut rows = review
            .items
            .iter()
            .map(|item| serde_json::to_vec(&item.classification))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        rows.sort_unstable();
        Ok::<_, String>(rows)
    };
    if cards(approved)? != cards(observed)? {
        return Err("The exported review does not match this capture.".into());
    }
    for item in &approved.items {
        match (default_decision(&item.classification), item.decision) {
            (Decision::Fix { .. }, Decision::Fix { .. }) | (Decision::Info, Decision::Info) => {}
            (Decision::Acknowledge { .. }, Decision::Acknowledge { .. }) => {
                return Err("A capture with warnings needs a separate packaging review.".into());
            }
            _ => return Err("A decision in the exported review has the wrong type.".into()),
        }
    }
    Ok(())
}

pub(crate) fn local_file(path: &Path) -> Result<PathBuf, String> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| safe_name(name))
        .ok_or("Choose a regular file with a plain file name.")?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = local_directory(parent)?;
    let file = parent.join(name);
    require_regular_path(&file, &parent)?;
    Ok(file)
}

fn only_setup(input: &Path) -> Result<Fingerprint, String> {
    let mut files =
        fs::read_dir(input).map_err(|e| format!("Could not list {}: {e}", input.display()))?;
    let first = files
        .next()
        .transpose()
        .map_err(|e| format!("Could not read captured setup: {e}"))?;
    let Some(first) = first.filter(|_| files.next().is_none()) else {
        return Err("The capture's input folder must contain exactly one setup file.".into());
    };
    let setup = local_file(&first.path())?;
    let fingerprint = fingerprint(&setup)?;
    if fingerprint.size_bytes == 0 {
        return Err("The capture's setup file is empty.".into());
    }
    Ok(fingerprint)
}

pub(crate) fn fingerprint(path: &Path) -> Result<Fingerprint, String> {
    let before = metadata_at(path)?;
    if !before.is_file() || before.len() > MAX_BYTES {
        return Err(format!(
            "Expected a regular file of at most 32 GiB at {}.",
            path.display()
        ));
    }
    let (sha256, size_bytes) = file_digest(path)?;
    if size_bytes != before.len() {
        return Err(format!(
            "The file changed while it was being read: {}.",
            path.display()
        ));
    }
    Ok(Fingerprint { sha256, size_bytes })
}

pub(crate) fn inventory(root: &Path) -> Result<Vec<TreeEntry>, String> {
    let root = local_directory(root)?;
    let mut folders = vec![(root, String::new(), 0usize)];
    let mut seen = BTreeSet::new();
    let mut entries = Vec::new();
    let mut bytes = 0;
    while let Some((folder, prefix, depth)) = folders.pop() {
        let listing = fs::read_dir(&folder)
            .map_err(|e| format!("Could not list {}: {e}", folder.display()))?;
        for item in listing {
            let item = item.map_err(|e| format!("Could not list {}: {e}", folder.display()))?;
            let name = item.file_name();
            let name = name
                .to_str()
                .filter(|name| safe_name(name))
                .ok_or_else(|| {
                    format!(
                        "Unsupported path in the approved tree: {}.",
                        item.path().display()
                    )
                })?;
            let path = if prefix.is_empty() {
                name.into()
            } else {
                format!("{prefix}/{name}")
            };
            if entries.len() >= MAX_ENTRIES || depth >= MAX_DEPTH {
                return Err("The profile tree exceeds the entry or folder depth limit.".into());
            }
            if !seen.insert(path.to_uppercase()) {
                return Err(format!("Conflicting paths in the profile tree: {path}."));
            }
            let absolute = item.path();
            let metadata = metadata_at(&absolute)?;
            let file = if metadata.is_dir() {
                folders.push((absolute, path.clone(), depth + 1));
                None
            } else {
                if metadata.len() > MAX_BYTES - bytes {
                    return Err("The profile tree exceeds the 32 GiB limit.".into());
                }
                let file = fingerprint(&absolute)?;
                if file.size_bytes > MAX_BYTES - bytes {
                    return Err("The profile tree exceeds the 32 GiB limit.".into());
                }
                bytes += file.size_bytes;
                Some(file)
            };
            entries.push(TreeEntry { path, file });
        }
    }
    entries.sort_unstable_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

fn verify_inventory(root: &Path, expected: &[TreeEntry]) -> Result<(), String> {
    if inventory(root)? != expected {
        return Err(
            "The approved package tree has changed. Save a new reviewed profile before building."
                .into(),
        );
    }
    Ok(())
}

pub(crate) fn read_small(path: &Path, root: &Path, max: usize) -> Result<Vec<u8>, String> {
    require_regular_path(path, root)?;
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(max as u64 + 1).read_to_end(&mut bytes))
        .map_err(|e| format!("Could not read {}: {e}", path.display()))?;
    if bytes.len() > max {
        return Err(format!(
            "The file exceeds its size limit: {}.",
            path.display()
        ));
    }
    Ok(bytes)
}

pub(crate) fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .and_then(|mut file| file.write_all(bytes))
        .map_err(|e| format!("Could not write {}: {e}", path.display()))
}

pub(crate) fn tree_sha256(root: &Path) -> Result<String, String> {
    inventory_sha256(&inventory(root)?)
}

fn inventory_sha256(entries: &[TreeEntry]) -> Result<String, String> {
    serde_json::to_vec(entries)
        .map(|bytes| sha256_hex(&bytes))
        .map_err(|e| e.to_string())
}

#[cfg(target_arch = "x86_64")]
fn sandboxie_receipt_evidence(
    session: &Path,
    session_id: &str,
    setup_sha256: &str,
    evidence: &CaptureEvidence,
    harvest: &[TreeEntry],
) -> Result<(String, Option<CapturedSetupInputs>), String> {
    use crate::sbie_verified::{
        verify_capture_sources, verify_clean_storage, verify_receipt, MAX_RECEIPT_BYTES,
        RECEIPT_FILE,
    };
    verify_clean_storage(session)?;
    let bytes =
        read_small(&session.join(RECEIPT_FILE), session, MAX_RECEIPT_BYTES).map_err(|error| {
            format!("This capture needs a verified Sandboxie cleanup receipt. {error}")
        })?;
    let receipt = verify_receipt(
        &bytes,
        session_id,
        setup_sha256,
        [
            &evidence.before_sha256,
            &evidence.after_install_sha256,
            &evidence.after_runtime_sha256,
        ],
        &inventory_sha256(harvest)?,
    )?;
    verify_capture_sources(session, &receipt)?;
    let inputs = receipt.setup_args_sha256.map(|arguments_sha256| {
        let mut companions: Vec<_> = receipt
            .companions
            .into_iter()
            .map(|item| SetupCompanion {
                name: item.name,
                sha256: item.sha256,
                size_bytes: item.size_bytes,
            })
            .collect();
        companions.sort_unstable_by(|left, right| left.name.cmp(&right.name));
        CapturedSetupInputs {
            arguments_sha256,
            companions,
        }
    });
    if inputs
        .as_ref()
        .is_some_and(|inputs| !valid_setup_inputs(inputs))
    {
        return Err("The capture's setup companion evidence is invalid.".into());
    }
    Ok((sha256_hex(&bytes), inputs))
}

#[cfg(not(target_arch = "x86_64"))]
fn sandboxie_receipt_evidence(
    _session: &Path,
    _session_id: &str,
    _setup_sha256: &str,
    _evidence: &CaptureEvidence,
    _harvest: &[TreeEntry],
) -> Result<(String, Option<CapturedSetupInputs>), String> {
    Err("Saving a Sandboxie-Plus capture profile requires Windows x64.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Classification, Finding, Severity};

    #[test]
    fn manifest_requires_the_external_hash_and_both_approvals() {
        let manifest = Manifest {
            schema_version: 1,
            session_id: "widget".into(),
            capture: CaptureEvidence {
                mode: SandboxMode::WindowsSandbox,
                before_sha256: sha256_hex(b"before"),
                after_install_sha256: sha256_hex(b"install"),
                after_runtime_sha256: sha256_hex(b"runtime"),
                sandboxie_receipt_sha256: None,
                sandboxie_setup: None,
            },
            setup: Fingerprint {
                sha256: sha256_hex(b"setup"),
                size_bytes: 5,
            },
            review_sha256: sha256_hex(b"review"),
            redistribution_asserted: true,
            reviewed_package_approved: true,
            package: vec![TreeEntry {
                path: "App".into(),
                file: None,
            }],
        };
        let bytes = serde_json::to_vec(&manifest).unwrap();
        let digest = sha256_hex(&bytes);
        assert!(approved_manifest(&bytes, &digest.to_ascii_uppercase()).is_ok());
        assert!(approved_manifest(&bytes, &sha256_hex(b"changed")).is_err());
        let mut edited = serde_json::to_value(manifest).unwrap();
        edited["reviewed_package_approved"] = false.into();
        let edited = serde_json::to_vec(&edited).unwrap();
        assert!(approved_manifest(&edited, &digest).is_err());
        assert!(approved_manifest(&edited, &sha256_hex(&edited)).is_err());
    }

    #[test]
    fn relocation_preserves_the_pin_and_rejects_modified_payload() {
        let root = std::env::temp_dir().join(format!("pf-relocate-{}", std::process::id()));
        let original = root.join("approved");
        let copied = root.join("relocated");
        let package = original.join(PACKAGE);
        fs::create_dir_all(&package).unwrap();
        let source_file = package.join("Widget.exe");
        fs::write(&source_file, b"payload").unwrap();
        let review = Review::default().to_json();
        let manifest = serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "session_id": "widget",
            "capture": {
                "mode": "WindowsSandbox",
                "before_sha256": sha256_hex(b"before"),
                "after_install_sha256": sha256_hex(b"install"),
                "after_runtime_sha256": sha256_hex(b"runtime")
            },
            "setup": { "sha256": sha256_hex(b"setup"), "size_bytes": 5 },
            "review_sha256": sha256_hex(review.as_bytes()),
            "redistribution_asserted": true,
            "reviewed_package_approved": true,
            "package": inventory(&package).unwrap()
        });
        let bytes = serde_json::to_vec(&manifest).unwrap();
        let digest = sha256_hex(&bytes);
        fs::write(original.join(MANIFEST), bytes).unwrap();
        fs::write(original.join(REVIEW), review).unwrap();
        assert!(relocate(&original, &sha256_hex(b"wrong"), &copied).is_err());
        assert!(!copied.exists());
        let relocated = relocate(&original, &digest, &copied).unwrap();
        assert_eq!(relocated.manifest_sha256, digest);
        assert_eq!(inspect(&copied, &digest).unwrap().package_file_count, 1);
        assert!(relocate(&original, &digest, &copied).is_err());
        fs::write(copied.join(PACKAGE).join("Widget.exe"), b"changed").unwrap();
        assert!(inspect(&copied, &digest).is_err());
        assert!(inspect(&original, &digest).is_ok());
        for path in [&original, &copied] {
            fs::remove_file(path.join(PACKAGE).join("Widget.exe")).unwrap();
            fs::remove_file(path.join(MANIFEST)).unwrap();
            fs::remove_file(path.join(REVIEW)).unwrap();
            fs::remove_dir(path.join(PACKAGE)).unwrap();
            fs::remove_dir(path).unwrap();
        }
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn replay_companions_must_match_their_named_bytes() {
        let root = std::env::temp_dir().join(format!("pf-companions-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let setup = root.join("Setup.exe");
        let file = root.join("Layout.dat");
        let companion = SetupCompanion {
            name: "Layout.dat".into(),
            sha256: sha256_hex(b"data"),
            size_bytes: 4,
        };
        let companions = [companion];
        assert!(verify_companions(&setup, &companions).is_err());
        fs::write(&file, b"data").unwrap();
        assert!(verify_companions(&setup, &companions).is_ok());
        assert!(verify_companions(&file, &companions).is_err());
        fs::write(&file, b"edit").unwrap();
        assert!(verify_companions(&setup, &companions).is_err());
        fs::remove_file(file).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn sandboxie_manifest_requires_a_receipt_fingerprint() {
        let input = serde_json::json!({
            "schema_version": 1,
            "session_id": "widget",
            "capture": {
                "mode": "Sandboxie",
                "before_sha256": sha256_hex(b"before"),
                "after_install_sha256": sha256_hex(b"install"),
                "after_runtime_sha256": sha256_hex(b"runtime")
            },
            "setup": { "sha256": sha256_hex(b"setup"), "size_bytes": 5 },
            "review_sha256": sha256_hex(b"review"),
            "redistribution_asserted": true,
            "reviewed_package_approved": true,
            "package": [{ "path": "App", "file": null }]
        });
        let bytes = serde_json::to_vec(&input).unwrap();
        assert!(approved_manifest(&bytes, &sha256_hex(&bytes)).is_err());
        let mut sealed = input;
        sealed["capture"]["sandboxie_receipt_sha256"] = sha256_hex(b"receipt").into();
        let bytes = serde_json::to_vec(&sealed).unwrap();
        assert!(approved_manifest(&bytes, &sha256_hex(&bytes)).is_ok());
    }

    #[test]
    fn review_allows_reordered_choices_but_rejects_missing_or_forged_cards() {
        let finding = |key: &str| Classification {
            finding: Finding::RegistryKey { key: key.into() },
            severity: Severity::Review,
            confidence: 0.75,
            why: "observed".into(),
        };
        let observed = Review::from_classifications(vec![finding("key A"), finding("key B")]);
        let mut approved = observed.clone();
        approved.items.reverse();
        approved.items[0].decision = Decision::Fix { on: false };
        assert!(verify_review(&approved, &observed).is_ok());
        approved.items[0].decision = Decision::Info;
        assert!(verify_review(&approved, &observed).is_err());
        approved.items.pop();
        assert!(verify_review(&approved, &observed).is_err());
    }

    #[test]
    fn exact_tree_includes_empty_folders_and_file_contents() {
        let root = std::env::temp_dir().join(format!("pf-profile-tree-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let file = root.join("Widget.exe");
        let folder = root.join("empty");
        fs::write(&file, b"same").unwrap();
        fs::create_dir(&folder).unwrap();
        let expected = inventory(&root).unwrap();
        assert_eq!(expected.len(), 2);
        fs::write(&file, b"edit").unwrap();
        assert!(verify_inventory(&root, &expected).is_err());
        fs::write(&file, b"same").unwrap();
        fs::remove_dir(&folder).unwrap();
        assert!(verify_inventory(&root, &expected).is_err());
        fs::create_dir(&folder).unwrap();
        assert!(verify_inventory(&root, &expected).is_ok());
        fs::remove_file(&file).unwrap();
        fs::remove_dir(&folder).unwrap();
        fs::remove_dir(&root).unwrap();
    }
}
