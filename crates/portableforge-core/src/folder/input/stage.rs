use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
#[cfg(any(unix, all(windows, target_arch = "x86_64")))]
use std::{fs, io};

use super::{error, io_error, ScanError};

#[derive(Debug)]
pub(super) struct Workspace {
    path: PathBuf,
    #[cfg(all(windows, target_arch = "x86_64"))]
    guard: Option<fs::File>,
    #[cfg(all(windows, target_arch = "x86_64"))]
    identity: [u32; 3],
    #[cfg(all(windows, target_arch = "x86_64"))]
    _ancestors: Vec<fs::File>,
    #[cfg(unix)]
    identity: (u64, u64),
}

impl Workspace {
    pub(super) fn new() -> Result<Self, ScanError> {
        let parent = std::path::absolute(std::env::temp_dir()).map_err(io_error)?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|cause| error(cause.to_string()))?;
        let tag = crate::snapshot::sha256_hex(
            format!("{}:{}", std::process::id(), now.as_nanos()).as_bytes(),
        );
        let path = parent.join(format!("pf-input-{}", &tag[..12]));
        Self::create(path)
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    #[cfg(all(windows, target_arch = "x86_64"))]
    fn create(path: PathBuf) -> Result<Self, ScanError> {
        use super::super::{hold_directory, hold_directory_chain_with, windows};
        let parent = path.parent().expect("workspace has a parent");
        let ancestors = hold_directory_chain_with(parent, windows::open_writable_ancestor)?;
        windows::create_private_directory(&path, true).map_err(io_error)?;
        let guard = hold_directory(&path).map_err(|cause| {
            error(format!(
                "The staging folder could not be protected at {}. {cause}",
                path.display()
            ))
        })?;
        let identity = windows::file_identity(&guard).map_err(|cause| {
            error(format!(
                "The staging folder identity could not be verified at {}. {cause}",
                path.display()
            ))
        })?;
        Ok(Self {
            path,
            guard: Some(guard),
            identity,
            _ancestors: ancestors,
        })
    }

    #[cfg(unix)]
    fn create(path: PathBuf) -> Result<Self, ScanError> {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt};
        let _parents = super::hold_directory_chain(path.parent().expect("workspace has a parent"))?;
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(io_error)?;
        let metadata = fs::symlink_metadata(&path).map_err(io_error)?;
        if !metadata.is_dir() || super::is_link(&metadata) {
            return Err(error("The staging folder changed before use."));
        }
        Ok(Self {
            path,
            identity: (metadata.dev(), metadata.ino()),
        })
    }

    #[cfg(not(any(unix, all(windows, target_arch = "x86_64"))))]
    fn create(_path: PathBuf) -> Result<Self, ScanError> {
        Err(error("Staging ZIPs and app files requires a supported platform. Choose an unpacked app folder."))
    }

    pub(super) fn cleanup(&mut self) -> Result<(), ScanError> {
        #[cfg(any(unix, all(windows, target_arch = "x86_64")))]
        self.remove().map_err(|cause| {
            error(format!(
                "Could not clean the staging folder {}. {cause}",
                self.path.display()
            ))
        })?;
        Ok(())
    }

    #[cfg(all(windows, target_arch = "x86_64"))]
    fn remove(&mut self) -> io::Result<()> {
        use super::super::windows;
        use std::os::windows::fs::MetadataExt;

        // Bind the deletion handle to the original identity before removing
        // any contents, and keep the ancestor and root guards alive throughout.
        drop(self.guard.take());
        let owner = match windows::own_delivery(&self.path) {
            Ok(owner) => owner,
            Err(cause) if cause.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(cause) => return Err(cause),
        };
        if windows::file_identity(&owner)? != self.identity {
            return Err(io::Error::other("Staging directory ownership changed."));
        }
        for entry in fs::read_dir(&self.path)? {
            let path = entry?.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_attributes() & 0x10 != 0 {
                fs::remove_dir_all(&path)?;
            } else {
                fs::remove_file(&path)?;
            }
        }
        windows::discard_owned(&owner)
    }

    #[cfg(unix)]
    fn remove(&mut self) -> io::Result<()> {
        use std::os::unix::fs::MetadataExt;
        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(cause) if cause.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(cause) => return Err(cause),
        };
        if !metadata.is_dir() || (metadata.dev(), metadata.ino()) != self.identity {
            return Err(io::Error::other("Staging directory ownership changed."));
        }
        fs::remove_dir_all(&self.path)
    }
}

#[cfg(any(unix, all(windows, target_arch = "x86_64")))]
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}
