//! Windows read-only folder guards and file properties.

use super::{error, io_error, is_link, AppProperties, ScanError};
use std::ffi::c_void;
use std::fs::{File, OpenOptions};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Component, Path, Prefix};
use std::ptr;

const FILE_SHARE_READ: u32 = 1;
const FILE_LIST_DIRECTORY: u32 = 1;
const FILE_READ_ATTRIBUTES: u32 = 0x80;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
const MAX_VERSION_BYTES: u32 = 1024 * 1024;
const MAX_PROPERTY_UNITS: usize = 256;

#[link(name = "kernel32")]
extern "system" {
    fn GetDriveTypeW(root: *const u16) -> u32;
}

#[link(name = "version")]
extern "system" {
    fn GetFileVersionInfoSizeW(filename: *const u16, handle: *mut u32) -> u32;
    fn GetFileVersionInfoW(
        filename: *const u16,
        handle: u32,
        length: u32,
        data: *mut c_void,
    ) -> i32;
    fn VerQueryValueW(
        block: *const c_void,
        subblock: *const u16,
        data: *mut *mut c_void,
        length: *mut u32,
    ) -> i32;
}

pub(super) fn local_drive(path: &Path) -> bool {
    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return false;
    };
    let drive = match prefix.kind() {
        Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => drive,
        _ => return false,
    };
    let root = [u16::from(drive), u16::from(b':'), u16::from(b'\\'), 0];
    // SAFETY: this is a live, NUL-terminated DOS drive-root string.
    let kind = unsafe { GetDriveTypeW(root.as_ptr()) };
    // DRIVE_REMOVABLE, DRIVE_FIXED, DRIVE_CDROM, or DRIVE_RAMDISK.
    matches!(kind, 2 | 3 | 5 | 6)
}

pub(super) fn open_directory(path: &Path) -> Result<File, ScanError> {
    // Holding each ancestor without FILE_SHARE_DELETE prevents a rename or
    // junction replacement while a descendant is opened by its full path.
    let directory = OpenOptions::new()
        .access_mode(FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(io_error)?;
    let metadata = directory.metadata().map_err(io_error)?;
    if !metadata.is_dir() || is_link(&metadata) {
        return Err(error("Links and redirected folders are excluded."));
    }
    Ok(directory)
}

pub(super) fn open_file(path: &Path) -> Result<File, ScanError> {
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(io_error)
}

pub(super) fn read_properties(path: &Path, properties: &mut AppProperties) {
    if let Some(version) = VersionResource::load(path) {
        if let Some(name) = version
            .property("ProductName")
            .or_else(|| version.property("FileDescription"))
        {
            properties.name = name;
            properties.name_inferred = false;
        }
        properties.publisher = version.property("CompanyName");
        properties.version = version
            .fixed_version()
            .or_else(|| version.property("ProductVersion"))
            .or_else(|| version.property("FileVersion"));
    }
    #[cfg(target_arch = "x86_64")]
    {
        properties.icon = crate::compile::read_application_icon(path).ok();
    }
}

struct VersionResource {
    words: Vec<u32>,
    length: usize,
}

impl VersionResource {
    fn load(path: &Path) -> Option<Self> {
        let path: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        let mut ignored = 0;
        // SAFETY: filename is a guarded absolute file path, and ignored is writable.
        let length = unsafe { GetFileVersionInfoSizeW(path.as_ptr(), &mut ignored) };
        if !(4..=MAX_VERSION_BYTES).contains(&length) {
            return None;
        }
        let mut words = vec![0u32; length.div_ceil(4) as usize];
        // SAFETY: the DWORD-aligned buffer holds at least length bytes.
        let result =
            unsafe { GetFileVersionInfoW(path.as_ptr(), 0, length, words.as_mut_ptr().cast()) };
        (result != 0).then_some(Self {
            words,
            length: length as usize,
        })
    }

    fn query(&self, key: &str, unit_bytes: usize) -> Option<&[u8]> {
        let key: Vec<u16> = key.encode_utf16().chain([0]).collect();
        let base = self.words.as_ptr();
        let mut result: *mut c_void = ptr::null_mut();
        let mut units = 0;
        // SAFETY: base is the initialized version buffer; key is NUL-terminated.
        // The returned pointer is treated only as an offset until bounded below.
        let ok = unsafe { VerQueryValueW(base.cast(), key.as_ptr(), &mut result, &mut units) };
        if ok == 0 || result.is_null() || units == 0 {
            return None;
        }
        let offset = (result as usize).checked_sub(base as usize)?;
        let size = (units as usize).checked_mul(unit_bytes)?;
        let end = offset.checked_add(size)?;
        if end > self.length {
            return None;
        }
        // SAFETY: all words were initialized and self.length is within the owned allocation.
        let bytes = unsafe { std::slice::from_raw_parts(base.cast::<u8>(), self.length) };
        bytes.get(offset..end)
    }

    fn property(&self, name: &str) -> Option<String> {
        let translations = self
            .query(r"\VarFileInfo\Translation", 1)
            .unwrap_or_default()
            .as_chunks::<4>()
            .0
            .iter()
            .take(32)
            .map(|bytes| {
                let language = u16::from_le_bytes([bytes[0], bytes[1]]);
                let page = u16::from_le_bytes([bytes[2], bytes[3]]);
                (language, page)
            })
            .chain([(0x0409, 0x04b0), (0x0409, 0x04e4)]);
        for (language, page) in translations {
            let key = format!(r"\StringFileInfo\{language:04x}{page:04x}\{name}");
            if let Some(text) = self.query(&key, 2).and_then(safe_property_text) {
                return Some(text);
            }
        }
        None
    }

    fn fixed_version(&self) -> Option<String> {
        let info = self.query(r"\", 1)?;
        let word = |offset| {
            Some(u32::from_le_bytes(
                info.get(offset..offset + 4)?.try_into().ok()?,
            ))
        };
        if word(0)? != 0xfeef04bd {
            return None;
        }
        // VS_FIXEDFILEINFO stores file version first and product version next.
        [(word(16)?, word(20)?), (word(8)?, word(12)?)]
            .into_iter()
            .find(|(ms, ls)| *ms != 0 || *ls != 0)
            .map(|(ms, ls)| format!("{}.{}.{}.{}", ms >> 16, ms & 0xffff, ls >> 16, ls & 0xffff))
    }
}

fn safe_property_text(bytes: &[u8]) -> Option<String> {
    let (units, remainder) = bytes.as_chunks::<2>();
    if !remainder.is_empty() || units.len() > MAX_PROPERTY_UNITS + 1 {
        return None;
    }
    let units: Vec<_> = units
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .take_while(|unit| *unit != 0)
        .collect();
    let text = String::from_utf16(&units).ok()?;
    let safe: String = text
        .chars()
        .map(|ch| {
            if ch.is_control() || matches!(ch, '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}') {
                ' '
            } else {
                ch
            }
        })
        .collect();
    let trimmed = safe.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn property_text_cannot_add_metadata_lines() {
        let bytes: Vec<_> = "Example\r\nCompany\u{202e}\0"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(
            safe_property_text(&bytes).as_deref(),
            Some("Example  Company")
        );
        assert!(safe_property_text(&vec![0; 2 * (MAX_PROPERTY_UNITS + 2)]).is_none());
    }
}
