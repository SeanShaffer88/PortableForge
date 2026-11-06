//! Seal an approved Windows Sandbox capture and replay its prepared package.

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::compile::safe_name;
use crate::compile::windows::{
    compile_installer_checked, copy_tree, file_digest, local_directory, metadata_at,
    new_output_path, preflight_profile_package, require_regular_path, within, CompileApproval,
    CompileReport, ProfileBuildEvidence, VerifyToolSupport,
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
        };
        if evidence.mode == SandboxMode::Sandboxie {
            evidence.sandboxie_receipt_sha256 = Some(sandboxie_receipt_digest(
                session,
                &session_record.id,
                &setup.sha256,
                &evidence,
                &harvest_before,
            )?);
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
    verify_tool_support: Option<VerifyToolSupport>,
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
    let mut expected_names = BTreeSet::from([MANIFEST, REVIEW, PACKAGE]);
    for entry in
        fs::read_dir(&profile).map_err(|e| format!("Could not list {}: {e}", profile.display()))?
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
    let review = read_small(&profile.join(REVIEW), &profile, MAX_MANIFEST)?;
    if sha256_hex(&review) != manifest.review_sha256 {
        return Err("The saved review has changed since approval.".into());
    }
    let package = profile.join(PACKAGE);
    verify_inventory(&package, &manifest.package)?;
    let approval = CompileApproval {
        evidence: ProfileBuildEvidence {
            manifest_sha256: sha256_hex(&bytes),
            setup_sha256: actual_setup.sha256,
            session_id: manifest.session_id,
        },
        verify_staged: &|stage| verify_inventory(stage, &manifest.package),
        verify_tool_support,
    };
    compile_installer_checked(
        &package,
        tool_root,
        &output,
        manifest.redistribution_asserted,
        Some(approval),
    )
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

fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
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

#[cfg(target_arch = "x86_64")]
pub(crate) fn tree_sha256(root: &Path) -> Result<String, String> {
    inventory_sha256(&inventory(root)?)
}

#[cfg(target_arch = "x86_64")]
fn inventory_sha256(entries: &[TreeEntry]) -> Result<String, String> {
    serde_json::to_vec(entries)
        .map(|bytes| sha256_hex(&bytes))
        .map_err(|e| e.to_string())
}

#[cfg(target_arch = "x86_64")]
fn sandboxie_receipt_digest(
    session: &Path,
    session_id: &str,
    setup_sha256: &str,
    evidence: &CaptureEvidence,
    harvest: &[TreeEntry],
) -> Result<String, String> {
    use crate::sbie_verified::{verify_clean_storage, verify_receipt, RECEIPT_FILE};
    verify_clean_storage(session)?;
    let bytes = read_small(&session.join(RECEIPT_FILE), session, 16_384).map_err(|error| {
        format!("This capture needs a verified Sandboxie cleanup receipt. {error}")
    })?;
    verify_receipt(
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
    Ok(sha256_hex(&bytes))
}

#[cfg(not(target_arch = "x86_64"))]
fn sandboxie_receipt_digest(
    _session: &Path,
    _session_id: &str,
    _setup_sha256: &str,
    _evidence: &CaptureEvidence,
    _harvest: &[TreeEntry],
) -> Result<String, String> {
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
