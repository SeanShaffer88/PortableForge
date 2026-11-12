//! Classify a user-selected input without following redirected paths.

use std::path::Path;

use super::{error, hold_directory_chain, io_error, is_link, safe_name, ScanError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DroppedInput {
    Folder,
    Setup,
}

/// A drag may carry a directory with a file extension. Inspect its type only
/// after validating the local drive and holding its parent chain.
pub fn classify_drop(path: &Path) -> Result<DroppedInput, ScanError> {
    if !path.is_absolute() {
        return Err(error("Choose a local app folder or setup file."));
    }
    let Some(parent) = path.parent() else {
        hold_directory_chain(path)?;
        return Ok(DroppedInput::Folder);
    };
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| safe_name(name))
        .ok_or_else(|| error("Choose a folder or setup file with a plain name."))?;
    let _parents = hold_directory_chain(parent)?;
    let metadata = std::fs::symlink_metadata(path).map_err(io_error)?;
    if is_link(&metadata) {
        return Err(error(
            "Choose the actual folder or setup file instead of a link.",
        ));
    }
    if metadata.is_dir() {
        return Ok(DroppedInput::Folder);
    }
    if metadata.is_file() {
        if let Some((_, extension)) = name.rsplit_once('.') {
            if extension.eq_ignore_ascii_case("exe") || extension.eq_ignore_ascii_case("msi") {
                return Ok(DroppedInput::Setup);
            }
        }
    }
    Err(error(
        "Choose an unpacked app folder or a setup file ending in .exe or .msi.",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn classifies_a_folder_with_an_executable_suffix_and_regular_setup() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("pf-drop-{}-{stamp}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let folder = root.join("App.exe");
        std::fs::create_dir(&folder).unwrap();
        let setup = root.join("Setup.MSI");
        std::fs::write(&setup, b"inert").unwrap();
        let other = root.join("Other.txt");
        std::fs::write(&other, b"inert").unwrap();
        assert_eq!(classify_drop(&folder).unwrap(), DroppedInput::Folder);
        assert_eq!(classify_drop(&setup).unwrap(), DroppedInput::Setup);
        assert!(classify_drop(&other).is_err());
        std::fs::remove_file(other).unwrap();
        std::fs::remove_file(setup).unwrap();
        std::fs::remove_dir(folder).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn rejects_a_network_share_before_reading_it() {
        let error = classify_drop(Path::new(r"\\host\share\setup.exe")).unwrap_err();
        assert!(error.message.contains("Network and device paths"));
    }
}
