//! Read key metadata from the dedicated hive without querying the merged view.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::{self, Read};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::ptr::null_mut;

use super::{wide, Handle, RegCloseKey, RegOpenKeyExW};

const HKEY_USERS: Handle = 0x80000003_u32 as i32 as isize as Handle;
const REG_OPTION_OPEN_LINK: u32 = 8;
const REG_PROCESS_APPKEY: u32 = 1;
const KEY_READ: u32 = 0x0002_0019;
const QUERY_AND_ENUMERATE: u32 = 1 | 8;
const REG_LINK: u32 = 6;
const ERROR_FILE_NOT_FOUND: i32 = 2;
const ERROR_PATH_NOT_FOUND: i32 = 3;
const ERROR_NO_MORE_ITEMS: i32 = 259;
const MAX_KEYS: usize = 8192;
const MAX_DEPTH: usize = 32;
const MAX_PATH_BYTES: usize = 4096;
const MAX_HIVE_BYTES: usize = 32 * 1024 * 1024;
const HIVE_FILES: [&str; 4] = ["RegHive", "RegHive.LOG", "RegHive.LOG1", "RegHive.LOG2"];
const LOGICAL_SOFTWARE: &str = r"HKEY_CURRENT_USER\Software";

#[repr(C)]
#[derive(Default)]
struct FileTime {
    low: u32,
    high: u32,
}

impl FileTime {
    fn ticks(&self) -> u64 {
        u64::from(self.high) << 32 | u64::from(self.low)
    }
}

#[link(name = "advapi32")]
extern "system" {
    fn RegEnumKeyExW(
        key: Handle,
        index: u32,
        name: *mut u16,
        units: *mut u32,
        reserved: *mut u32,
        class: *mut u16,
        class_units: *mut u32,
        last_write: *mut FileTime,
    ) -> i32;
    fn RegQueryInfoKeyW(
        key: Handle,
        class: *mut u16,
        class_units: *mut u32,
        reserved: *mut u32,
        subkeys: *mut u32,
        max_subkey_units: *mut u32,
        max_class_units: *mut u32,
        values: *mut u32,
        max_value_name_units: *mut u32,
        max_value_bytes: *mut u32,
        security_bytes: *mut u32,
        last_write: *mut FileTime,
    ) -> i32;
    fn RegQueryValueExW(
        key: Handle,
        name: *const u16,
        reserved: *mut u32,
        kind: *mut u32,
        data: *mut u8,
        bytes: *mut u32,
    ) -> i32;
    fn RegLoadAppKeyW(
        file: *const u16,
        key: *mut Handle,
        access: u32,
        options: u32,
        reserved: u32,
    ) -> i32;
}

struct Key(Handle);

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: each wrapper owns one successfully opened key.
        unsafe { RegCloseKey(self.0) };
    }
}

fn registry_error(action: &str, status: i32) -> String {
    format!(
        "Could not {action} in the dedicated registry hive: {}",
        io::Error::from_raw_os_error(status)
    )
}

fn validate_component(name: &str) -> Result<(), String> {
    if name.is_empty() || name.contains(['\\', '\0']) {
        return Err("The dedicated registry hive has an unexpected key name.".into());
    }
    Ok(())
}

fn open_child(parent: Handle, component: &str) -> Result<Option<Key>, String> {
    validate_component(component)?;
    let path = wide(OsStr::new(component))?;
    let mut child = null_mut();
    // SAFETY: the parent is a live key, path is a single NUL-terminated
    // component, and OPEN_LINK prevents a final symbolic-link redirect.
    let status = unsafe {
        RegOpenKeyExW(
            parent,
            path.as_ptr(),
            REG_OPTION_OPEN_LINK,
            QUERY_AND_ENUMERATE,
            &mut child,
        )
    };
    let key = match status {
        0 => Key(child),
        ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => return Ok(None),
        _ => return Err(registry_error("open a key", status)),
    };
    check_link(&key)?;
    Ok(Some(key))
}

fn check_link(key: &Key) -> Result<(), String> {
    let name = wide(OsStr::new("SymbolicLinkValue"))?;
    let mut kind = 0;
    let mut size = 0;
    // SAFETY: all pointer arguments have the documented sizes. Null data
    // means this query obtains type and size metadata only.
    let status = unsafe {
        RegQueryValueExW(
            key.0,
            name.as_ptr(),
            null_mut(),
            &mut kind,
            null_mut(),
            &mut size,
        )
    };
    match status {
        0 if kind == REG_LINK => {
            Err("The dedicated registry snapshot contains a symbolic link.".into())
        }
        0 | ERROR_FILE_NOT_FOUND => Ok(()),
        _ => Err(registry_error("check a key for redirection", status)),
    }
}

fn last_write(key: &Key) -> Result<u64, String> {
    let mut time = FileTime::default();
    // SAFETY: optional outputs are null and time is a writable FILETIME.
    let status = unsafe {
        RegQueryInfoKeyW(
            key.0,
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            &mut time,
        )
    };
    if status != 0 {
        return Err(registry_error("read a key's last write time", status));
    }
    Ok(time.ticks())
}

fn walk(
    key: &Key,
    path: &str,
    depth: usize,
    keys: &mut BTreeMap<String, u64>,
) -> Result<(), String> {
    let mut buffer = [0u16; 256];
    for index in 0..=MAX_KEYS as u32 {
        let mut units = buffer.len() as u32;
        let mut time = FileTime::default();
        // SAFETY: buffer, units, and time are writable outputs. The other
        // optional outputs are null and the key stays alive for this call.
        let status = unsafe {
            RegEnumKeyExW(
                key.0,
                index,
                buffer.as_mut_ptr(),
                &mut units,
                null_mut(),
                null_mut(),
                null_mut(),
                &mut time,
            )
        };
        match status {
            0 => {}
            ERROR_NO_MORE_ITEMS => return Ok(()),
            _ => return Err(registry_error("enumerate a key", status)),
        }
        let name = String::from_utf16(
            buffer
                .get(..units as usize)
                .ok_or("A dedicated registry key name exceeds the snapshot limit.")?,
        )
        .map_err(|_| "The dedicated registry hive has an invalid UTF-16 key name.")?;
        validate_component(&name)?;
        // Sandboxie can alias Software\Classes to current_classes. It is
        // outside this report's application-settings scope.
        if depth == 0 && name.eq_ignore_ascii_case("Classes") {
            continue;
        }
        let child_path = format!(r"{path}\{name}");
        if depth == MAX_DEPTH || keys.len() == MAX_KEYS || child_path.len() > MAX_PATH_BYTES {
            return Err("The dedicated registry hive exceeds the snapshot limits.".into());
        }
        let child = open_child(key.0, &name)?
            .ok_or("The dedicated registry hive changed during the snapshot.")?;
        if keys.insert(child_path.clone(), time.ticks()).is_some() {
            return Err("The dedicated registry hive returned a duplicate key.".into());
        }
        walk(&child, &child_path, depth + 1, keys)?;
    }
    Err("The dedicated registry hive exceeds the snapshot limits.".into())
}

fn user_software(root: &Key) -> Result<BTreeMap<String, u64>, String> {
    let mut parent = root.0;
    let mut opened = Vec::with_capacity(3);
    for name in ["user", "current", "Software"] {
        let Some(child) = open_child(parent, name)? else {
            return Ok(BTreeMap::new());
        };
        parent = child.0;
        opened.push(child);
    }
    let software = &opened[2];
    let mut keys = BTreeMap::from([(LOGICAL_SOFTWARE.into(), last_write(software)?)]);
    walk(software, LOGICAL_SOFTWARE, 0, &mut keys)?;
    Ok(keys)
}

pub(super) fn snapshot(key: &str, root: &Path) -> Result<BTreeMap<String, u64>, String> {
    if let Some(hive) = open_child(HKEY_USERS, key)? {
        return user_software(&hive);
    }
    let copy = HiveCopy::new(root)?;
    let hive = load_app_key(&copy.0.join(HIVE_FILES[0]))?;
    user_software(&hive)
}

struct HiveCopy(PathBuf);

impl HiveCopy {
    fn new(root: &Path) -> Result<Self, String> {
        if super::local_directory(root)? != root {
            return Err("The dedicated box registry folder changed.".into());
        }
        let temporary = root.join(format!("hive-read-{}", super::super::new_box_name()?));
        fs::create_dir(&temporary)
            .map_err(|err| format!("Could not create the private registry snapshot: {err}"))?;
        let owned = Self(temporary);
        let mut remaining = MAX_HIVE_BYTES;
        for (index, name) in HIVE_FILES.into_iter().enumerate() {
            match copy_hive_file(&root.join(name), &owned.0.join(name), remaining)? {
                Some(bytes) => remaining -= bytes,
                None if index == 0 => {
                    return Err("The dedicated registry hive is unavailable.".into())
                }
                None => {}
            }
        }
        Ok(owned)
    }
}

impl Drop for HiveCopy {
    fn drop(&mut self) {
        for file in HIVE_FILES {
            let _ = fs::remove_file(self.0.join(file));
        }
        // The entire dedicated box is also removed and verified by the caller.
        let _ = fs::remove_dir(&self.0);
    }
}

fn copy_hive_file(source: &Path, target: &Path, limit: usize) -> Result<Option<usize>, String> {
    const FILE_SHARE_READ: u32 = 1;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    let file = match OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(source)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Could not read the idle dedicated hive: {error}")),
    };
    let metadata = file
        .metadata()
        .map_err(|error| format!("Could not inspect the idle dedicated hive: {error}"))?;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err("The dedicated registry hive has an unexpected file type.".into());
    }
    if metadata.len() > limit as u64 {
        return Err("The dedicated registry hive exceeds the offline snapshot size limit.".into());
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not copy the idle dedicated hive: {error}"))?;
    if bytes.len() as u64 != metadata.len() {
        return Err("The dedicated registry hive changed during the copy.".into());
    }
    crate::profile::write_new(target, &bytes)?;
    Ok(Some(bytes.len()))
}

fn load_app_key(path: &Path) -> Result<Key, String> {
    // A missing path would cause RegLoadAppKeyW to create a new hive.
    if !fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_file()) {
        return Err("The private registry snapshot is missing.".into());
    }
    let path = wide(path.as_os_str())?;
    let mut handle = null_mut();
    // SAFETY: file is the existing private copy below the owned box, the
    // path is NUL-terminated, and handle is a writable PHKEY output.
    let status =
        unsafe { RegLoadAppKeyW(path.as_ptr(), &mut handle, KEY_READ, REG_PROCESS_APPKEY, 0) };
    if status != 0 {
        return Err(registry_error("load the private snapshot", status));
    }
    Ok(Key(handle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr::null;

    #[link(name = "advapi32")]
    extern "system" {
        fn RegCreateKeyExW(
            key: Handle,
            path: *const u16,
            reserved: u32,
            class: *const u16,
            options: u32,
            access: u32,
            security: Handle,
            result: *mut Handle,
            disposition: *mut u32,
        ) -> i32;
    }

    fn create_private_test_key(root: &Key, path: &str) {
        let path = wide(OsStr::new(path)).unwrap();
        let mut key = null_mut();
        // SAFETY: the parent is an owned, process-private application hive.
        // The child path is a NUL-terminated fixture path.
        let status = unsafe {
            RegCreateKeyExW(
                root.0,
                path.as_ptr(),
                0,
                null(),
                0,
                KEY_READ,
                null_mut(),
                &mut key,
                null_mut(),
            )
        };
        assert_eq!(status, 0, "{}", io::Error::from_raw_os_error(status));
        drop(Key(key));
    }

    #[test]
    fn reads_only_scoped_keys_from_a_private_copy_of_a_hive() {
        let temp = super::super::local_directory(&std::env::temp_dir()).unwrap();
        let fixture = temp.join(format!(
            "registry-fixture-{}",
            super::super::super::new_box_name().unwrap()
        ));
        fs::create_dir(&fixture).unwrap();
        let fixture = HiveCopy(fixture);
        let path = fixture.0.join(HIVE_FILES[0]);
        let wide_path = wide(path.as_os_str()).unwrap();
        let mut root = null_mut();
        // SAFETY: intentionally create only a disposable process-private
        // hive at a fresh test path. No host registry hive is opened.
        let status = unsafe {
            RegLoadAppKeyW(
                wide_path.as_ptr(),
                &mut root,
                KEY_READ | 4,
                REG_PROCESS_APPKEY,
                0,
            )
        };
        assert_eq!(status, 0, "{}", io::Error::from_raw_os_error(status));
        let root = Key(root);
        assert!(user_software(&root).unwrap().is_empty());
        create_private_test_key(&root, r"user\current\Software\Fixture\Settings");
        create_private_test_key(&root, r"user\current\Software\Classes\OutsideScope");
        create_private_test_key(&root, r"machine\Software\OutsideScope");
        drop(root);

        let copy = HiveCopy::new(&fixture.0).unwrap();
        let work = copy.0.clone();
        let snapshot = user_software(&load_app_key(&work.join(HIVE_FILES[0])).unwrap()).unwrap();
        assert_eq!(
            snapshot.keys().map(String::as_str).collect::<Vec<_>>(),
            [
                LOGICAL_SOFTWARE,
                r"HKEY_CURRENT_USER\Software\Fixture",
                r"HKEY_CURRENT_USER\Software\Fixture\Settings",
            ],
        );
        assert!(snapshot.values().all(|&value| value > 0));
        drop(copy);
        assert!(
            !work.exists(),
            "private copy remained after closing the hive"
        );
    }

    #[test]
    fn refuses_missing_hive_and_multicomponent_names() {
        assert!(validate_component(r"user\current").is_err());
        assert!(validate_component("").is_err());
        let path = std::env::temp_dir().join(super::super::super::new_box_name().unwrap());
        assert!(load_app_key(&path).is_err());
        assert!(!path.exists());
    }
}
