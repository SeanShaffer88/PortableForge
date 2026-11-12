use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::windows::fs::OpenOptionsExt;

use super::*;
use crate::profile::Fingerprint;

const SOURCE_DIR: &str = "setup-source";
const MAX_COMPANIONS: usize = 32;
const MAX_COMPANION_BYTES: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SetupCompanion {
    pub(crate) name: String,
    pub(crate) sha256: String,
    pub(crate) size_bytes: u64,
}

fn digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn source_name(name: &str) -> bool {
    name.encode_utf16().count() <= 128 && safe_name(name)
}

pub(super) fn setup_args_sha256(args: &[OsString]) -> Result<String, String> {
    let args = args
        .iter()
        .map(|arg| {
            arg.to_str()
                .filter(|value| !value.contains('\0'))
                .ok_or("Setup arguments must be Unicode strings without NULs.")
        })
        .collect::<Result<Vec<_>, _>>()?;
    if args.len() > 128
        || args
            .iter()
            .map(|arg| arg.encode_utf16().count())
            .sum::<usize>()
            > 16_384
    {
        return Err("Setup arguments exceed the supported command-line limit.".into());
    }
    serde_json::to_vec(&args)
        .map(|bytes| sha256_hex(&bytes))
        .map_err(|error| error.to_string())
}

fn companion_metadata_valid(setup_name: &str, companions: &[SetupCompanion]) -> bool {
    if companions.len() > MAX_COMPANIONS {
        return false;
    }
    let mut names = BTreeSet::from([setup_name.to_uppercase()]);
    let mut total = 0_u64;
    companions.iter().all(|file| {
        total = total.saturating_add(file.size_bytes);
        total <= MAX_COMPANION_BYTES
            && source_name(&file.name)
            && digest(&file.sha256)
            && names.insert(file.name.to_uppercase())
    })
}

pub(super) fn valid_receipt_sources(receipt: &Receipt) -> bool {
    match receipt.schema_version {
        1 => {
            receipt.setup_name.is_none()
                && receipt.setup_args_sha256.is_none()
                && receipt.installed_application.is_none()
                && receipt.companions.is_empty()
        }
        2 => {
            receipt.setup_name.as_deref().is_some_and(|name| {
                source_name(name)
                    && Path::new(name)
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
                    && companion_metadata_valid(name, &receipt.companions)
            }) && receipt.setup_args_sha256.as_deref().is_some_and(digest)
                && receipt
                    .installed_application
                    .as_deref()
                    .is_some_and(|path| logical_executable(path).is_ok())
        }
        _ => false,
    }
}

fn checked_filename(path: &Path) -> Result<String, String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| source_name(name))
        .map(str::to_owned)
        .ok_or_else(|| format!("Choose a file with a supported name: {}.", path.display()))
}

fn hold(path: &Path) -> Result<File, String> {
    OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(path)
        .map_err(|e| io_error(path, e))
}

pub(super) fn copy_locked(
    source: &Path,
    destination: &Path,
    expected: &Fingerprint,
) -> Result<File, String> {
    let mut source = hold(source)?;
    let mut copy = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)
        .map_err(|e| io_error(destination, e))?;
    let size = io::copy(
        &mut (&mut source).take(expected.size_bytes.saturating_add(1)),
        &mut copy,
    )
    .map_err(|e| io_error(destination, e))?;
    drop(copy);
    let lock = hold(destination)?;
    if size != expected.size_bytes || fingerprint(destination)? != *expected {
        return Err(format!(
            "File changed while staging {}.",
            destination.display()
        ));
    }
    Ok(lock)
}

pub(super) struct PreparedSetup {
    pub(super) name: String,
    pub(super) image: PathBuf,
    pub(super) cwd: PathBuf,
    pub(super) companions: Vec<SetupCompanion>,
    _locks: Vec<File>,
}

impl PreparedSetup {
    pub(super) fn stage(
        setup: &Path,
        expected: &Fingerprint,
        inputs: &[PathBuf],
        layout: &SessionLayout,
    ) -> Result<Self, String> {
        let name = checked_filename(setup)?;
        if inputs.len() > MAX_COMPANIONS {
            return Err("Too many setup companion files.".into());
        }
        let mut companions = Vec::with_capacity(inputs.len());
        let mut paths = Vec::with_capacity(inputs.len());
        let mut total = 0_u64;
        for path in inputs {
            let path = local_file(path)?;
            if within(&path, &layout.root) {
                return Err("Setup companion files must be outside the session folder.".into());
            }
            total = total.saturating_add(metadata_at(&path)?.len());
            if total > MAX_COMPANION_BYTES {
                return Err("Setup companion files exceed the size limit.".into());
            }
            let fp = fingerprint(&path)?;
            companions.push(SetupCompanion {
                name: checked_filename(&path)?,
                sha256: fp.sha256,
                size_bytes: fp.size_bytes,
            });
            paths.push(path);
        }
        if !companion_metadata_valid(&name, &companions) {
            return Err(
                "Setup companion filenames must be distinct and within the staging limits.".into(),
            );
        }
        let input = layout.input.join(&name);
        let mut locks = vec![copy_locked(setup, &input, expected)?];
        let cwd = if companions.is_empty() {
            layout.input.clone()
        } else {
            let cwd = layout.root.join(SOURCE_DIR);
            mkdir(&cwd)?;
            locks.push(copy_locked(&input, &cwd.join(&name), expected)?);
            for (path, file) in paths.iter().zip(&companions) {
                let fp = Fingerprint {
                    sha256: file.sha256.clone(),
                    size_bytes: file.size_bytes,
                };
                locks.push(copy_locked(path, &cwd.join(&file.name), &fp)?);
            }
            cwd
        };
        Ok(Self {
            image: cwd.join(&name),
            name,
            cwd,
            companions,
            _locks: locks,
        })
    }
}

pub(crate) fn verify_capture_sources(session: &Path, receipt: &Receipt) -> Result<(), String> {
    if receipt.schema_version == 1 {
        return Ok(());
    }
    let name = receipt
        .setup_name
        .as_deref()
        .ok_or("The receipt is missing the setup filename.")?;
    let input = session.join("input");
    let setup = input.join(name);
    require_regular_path(&setup, session)?;
    let fp = fingerprint(&setup)?;
    if fp.sha256 != receipt.setup_sha256 {
        return Err("The captured setup bytes do not match the receipt.".into());
    }
    if receipt.companions.is_empty() {
        return expect_absent(&session.join(SOURCE_DIR));
    }
    let source = session.join(SOURCE_DIR);
    let mut expected = BTreeMap::from([(PathBuf::from(name), fp)]);
    for file in &receipt.companions {
        expected.insert(
            PathBuf::from(&file.name),
            Fingerprint {
                sha256: file.sha256.clone(),
                size_bytes: file.size_bytes,
            },
        );
    }
    for row in inventory(&source)? {
        if expected.remove(Path::new(&row.path)).as_ref() != row.file.as_ref() || row.file.is_none()
        {
            return Err("Captured setup source files changed since capture.".into());
        }
    }
    if !expected.is_empty() {
        return Err("Captured setup source files are missing.".into());
    }
    Ok(())
}
