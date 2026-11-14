//! One local app-file draft. This data never grants permission to build.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use portableforge_core::{artwork::Artwork, folder::input::DropPlan};
use serde::{Deserialize, Serialize};

use super::draft::Draft;

mod artwork;

pub(super) const SAVE: &str = "Save for Later";
pub(super) const SAVING: &str = "Saving the app job…";
pub(super) const RESUME: &str = "Resume App Package";
pub(super) const LOADING: &str = "Opening saved job…";
pub(super) const REMOVE: &str = "Remove Saved Job";
const MAX_METADATA_BYTES: u64 = 16 * 1024 * 1024;
const MAX_INPUTS: usize = 20_000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum SourceKind {
    Folder,
    Zip,
    Files,
}

impl SourceKind {
    pub(super) fn from_plan(plan: &DropPlan) -> Option<Self> {
        match plan {
            DropPlan::Folder(_) => Some(Self::Folder),
            DropPlan::Zip(_) => Some(Self::Zip),
            DropPlan::Files(_) => Some(Self::Files),
            DropPlan::Setup(_) => None,
        }
    }
}

#[derive(Deserialize, Serialize)]
pub(super) struct SavedJob {
    version: u32,
    pub(super) kind: SourceKind,
    pub(super) inputs: Vec<PathBuf>,
    pub(super) selected: Option<PathBuf>,
    pub(super) draft: Draft,
    pub(super) launcher_tools: PathBuf,
    pub(super) installer_tools: PathBuf,
    pub(super) output_parent: PathBuf,
    #[serde(skip)]
    pub(super) artwork: Option<Artwork>,
}

impl SavedJob {
    pub(super) fn from_flow(flow: &super::Flow, kind: SourceKind) -> Self {
        Self {
            version: 1,
            kind,
            inputs: flow.original_inputs.clone(),
            selected: flow.selected.clone(),
            draft: flow.draft.clone(),
            launcher_tools: flow.launcher_tools.clone(),
            installer_tools: flow.installer_tools.clone(),
            output_parent: flow.output_parent.clone(),
            artwork: flow.artwork.artwork().cloned(),
        }
    }

    fn validate(&self) -> io::Result<()> {
        use std::path::Component::Normal;

        let count_ok = match self.kind {
            SourceKind::Files => (2..=MAX_INPUTS).contains(&self.inputs.len()),
            _ => self.inputs.len() == 1,
        };
        let optional_absolute = |path: &Path| path.as_os_str().is_empty() || path.is_absolute();
        if self.version != 1
            || !count_ok
            || !self.inputs.iter().all(|path| path.is_absolute())
            || !self.selected.as_ref().is_none_or(|path| {
                !path.as_os_str().is_empty()
                    && path
                        .components()
                        .all(|component| matches!(component, Normal(_)))
            })
            || !optional_absolute(&self.launcher_tools)
            || !optional_absolute(&self.installer_tools)
            || !optional_absolute(&self.output_parent)
            || (self.artwork.is_some() && self.selected.is_none())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "The saved job contains unsupported app files or choices.",
            ));
        }
        Ok(())
    }
}

pub(super) fn path(settings_path: &Path) -> PathBuf {
    settings_path.with_file_name("autonomous-job.json")
}

pub(super) fn load(path: &Path) -> io::Result<SavedJob> {
    if !fs::symlink_metadata(path)?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "The saved job is not a file.",
        ));
    }
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let mut magic = [0; artwork::MAGIC.len()];
    let has_artwork = if len >= magic.len() as u64 {
        file.read_exact(&mut magic)?;
        magic == artwork::MAGIC
    } else {
        false
    };
    let job = if has_artwork {
        artwork::read(file, len)?
    } else {
        file.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        file.take(MAX_METADATA_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_METADATA_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "The saved job is too large.",
            ));
        }
        serde_json::from_slice(&bytes)?
    };
    job.validate()?;
    Ok(job)
}

/// Use the slot's file metadata as a precondition on explicit replacement and removal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Revision {
    created: Option<SystemTime>,
    modified: Option<SystemTime>,
    len: u64,
}

pub(super) fn revision(path: &Path) -> io::Result<Option<Revision>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(Revision {
            created: metadata.created().ok(),
            modified: metadata.modified().ok(),
            len: metadata.len(),
        })),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub(super) fn save(path: &Path, expected: Option<Revision>, job: &SavedJob) -> io::Result<()> {
    job.validate()?;
    let contents = artwork::Contents::new(job)?;
    with_slot(path, expected, || {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let clock = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let temporary = path.with_extension(format!(
            "{}-{}-{}.tmp",
            std::process::id(),
            clock.as_nanos(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let result = contents.write(&mut file).and_then(|()| file.sync_all());
        drop(file);
        let result = result.and_then(|()| fs::rename(&temporary, path));
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    })
}

pub(super) fn remove(path: &Path, expected: Revision) -> io::Result<()> {
    with_slot(path, Some(expected), || fs::remove_file(path))
}

fn with_slot(
    path: &Path,
    expected: Option<Revision>,
    action: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    #[cfg(windows)]
    let _lock = {
        use std::os::windows::fs::OpenOptionsExt;
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(path.with_extension("lock"))?
    };
    #[cfg(not(windows))]
    let _lock = {
        static SLOT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        SLOT_LOCK.lock().map_err(io::Error::other)?
    };
    if revision(path)? != expected {
        return Err(io::Error::other(
            "The saved job changed. Review the current saved job and try again.",
        ));
    }
    action()
}
