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
    matches!(drive_type(path), 2 | 3 | 5 | 6)
}

fn drive_type(path: &Path) -> u32 {
    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return 0;
    };
    let drive = match prefix.kind() {
        Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => drive,
        _ => return 0,
    };
    let root = [u16::from(drive), u16::from(b':'), u16::from(b'\\'), 0];
    // SAFETY: this is a live, NUL-terminated DOS drive-root string.
    unsafe { GetDriveTypeW(root.as_ptr()) }
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

#[cfg(target_arch = "x86_64")]
mod build_paths {
    use std::ffi::{c_void, OsString};
    use std::fs::{self, File, OpenOptions};
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use std::path::{Path, PathBuf};
    use std::ptr;

    const FILE_PERSISTENT_ACLS: u32 = 8;

    #[repr(C)]
    struct SecurityAttributes {
        length: u32,
        descriptor: *mut c_void,
        inherit: i32,
    }

    #[repr(C)]
    struct FileIdentityInfo {
        attributes: u32,
        creation: [u32; 2],
        access: [u32; 2],
        written: [u32; 2],
        volume: u32,
        size: [u32; 2],
        links: u32,
        index: [u32; 2],
    }

    #[repr(C)]
    struct RenameInfo {
        replace: u32,
        root: *mut c_void,
        name_bytes: u32,
        first_name_unit: u16,
    }

    #[link(name = "advapi32")]
    extern "system" {
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text: *const u16,
            revision: u32,
            descriptor: *mut *mut c_void,
            length: *mut u32,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateDirectoryW(path: *const u16, attributes: *const SecurityAttributes) -> i32;
        fn GetVolumePathNameW(path: *const u16, volume: *mut u16, capacity: u32) -> i32;
        fn GetVolumeInformationW(
            root: *const u16,
            name: *mut u16,
            name_capacity: u32,
            serial: *mut u32,
            component_length: *mut u32,
            flags: *mut u32,
            filesystem: *mut u16,
            filesystem_capacity: u32,
        ) -> i32;
        fn LocalFree(allocation: *mut c_void) -> *mut c_void;
        fn GetFileInformationByHandle(handle: *mut c_void, info: *mut FileIdentityInfo) -> i32;
        fn SetFileInformationByHandle(
            handle: *mut c_void,
            class: u32,
            info: *const c_void,
            size: u32,
        ) -> i32;
        fn GetSystemWindowsDirectoryW(path: *mut u16, capacity: u32) -> u32;
    }

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain([0]).collect()
    }

    pub(in super::super) fn create_private_directory(path: &Path, scratch: bool) -> io::Result<()> {
        if scratch && super::drive_type(path) != 3 {
            return Err(io::Error::other(
                "Use a fixed local drive for temporary build work.",
            ));
        }
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("No work folder parent."))?;
        let parent = wide(parent);
        let mut volume = vec![0u16; 32_768];
        // SAFETY: parent is terminated and the writable volume buffer has capacity.
        if unsafe { GetVolumePathNameW(parent.as_ptr(), volume.as_mut_ptr(), volume.len() as u32) }
            == 0
        {
            return Err(io::Error::last_os_error());
        }
        let mut flags = 0;
        // SAFETY: the volume string was initialized by the preceding successful
        // call; flags is writable and omitted outputs permit null.
        if unsafe {
            GetVolumeInformationW(
                volume.as_ptr(),
                ptr::null_mut(),
                0,
                ptr::null_mut(),
                ptr::null_mut(),
                &mut flags,
                ptr::null_mut(),
                0,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if flags & FILE_PERSISTENT_ACLS == 0 {
            if !scratch {
                return fs::create_dir(path);
            }
            return Err(io::Error::other(
                "The local temporary drive cannot enforce a private work folder.",
            ));
        }
        let mut descriptor = ptr::null_mut();
        // Protected inherited DACL: only the owner, SYSTEM and Administrators
        // may modify files under this staging directory.
        let sddl: Vec<_> = "D:P(A;OICI;FA;;;OW)(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)"
            .encode_utf16()
            .chain([0])
            .collect();
        // SAFETY: sddl is terminated and descriptor is a writable output.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let attributes = SecurityAttributes {
            length: std::mem::size_of::<SecurityAttributes>() as u32,
            descriptor,
            inherit: 0,
        };
        let path = wide(path);
        // SAFETY: attributes references a valid security descriptor until after
        // CreateDirectoryW returns; path is terminated.
        let created = unsafe { CreateDirectoryW(path.as_ptr(), &attributes) };
        let result = if created == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        };
        // SAFETY: ConvertStringSecurityDescriptorToSecurityDescriptorW allocated it.
        unsafe { LocalFree(descriptor) };
        result
    }

    pub(in super::super) fn file_identity(file: &File) -> io::Result<[u32; 3]> {
        let mut info = std::mem::MaybeUninit::<FileIdentityInfo>::uninit();
        // SAFETY: file is a live handle and info is a writable native structure.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the successful Win32 call initialized the full structure.
        let info = unsafe { info.assume_init() };
        Ok([info.volume, info.index[0], info.index[1]])
    }

    pub(in super::super) fn open_writable_ancestor(
        path: &Path,
    ) -> Result<File, super::super::ScanError> {
        let file = OpenOptions::new()
            .access_mode(super::FILE_LIST_DIRECTORY | super::FILE_READ_ATTRIBUTES)
            .share_mode(super::FILE_SHARE_READ | 2)
            .custom_flags(super::FILE_FLAG_BACKUP_SEMANTICS | super::FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .map_err(super::super::io_error)?;
        let metadata = file.metadata().map_err(super::super::io_error)?;
        if !metadata.is_dir() || super::is_link(&metadata) {
            return Err(super::super::error(
                "Choose an ordinary local folder. Links and redirected paths are excluded.",
            ));
        }
        Ok(file)
    }

    pub(in super::super) fn own_delivery(path: &Path) -> io::Result<File> {
        const DELETE: u32 = 0x0001_0000;
        let file = OpenOptions::new()
            .access_mode(super::FILE_LIST_DIRECTORY | super::FILE_READ_ATTRIBUTES | DELETE)
            .share_mode(super::FILE_SHARE_READ)
            .custom_flags(super::FILE_FLAG_BACKUP_SEMANTICS | super::FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_dir() || super::is_link(&metadata) {
            return Err(io::Error::other(
                "The delivery folder is no longer an ordinary directory.",
            ));
        }
        Ok(file)
    }

    pub(in super::super) fn lock_delivery_file(
        path: &Path,
        identity: [u32; 3],
    ) -> io::Result<File> {
        let file = OpenOptions::new()
            .read(true)
            .share_mode(super::FILE_SHARE_READ)
            .custom_flags(super::FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || super::is_link(&metadata) || file_identity(&file)? != identity {
            return Err(io::Error::other("A delivery file changed ownership."));
        }
        Ok(file)
    }

    pub(in super::super) fn discard_owned_file(path: &Path, identity: [u32; 3]) -> io::Result<()> {
        const DELETE: u32 = 0x0001_0000;
        let file = match OpenOptions::new()
            .access_mode(super::FILE_READ_ATTRIBUTES | DELETE)
            .share_mode(super::FILE_SHARE_READ)
            .custom_flags(super::FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
        {
            Ok(file) => file,
            Err(cause) if cause.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(cause) => return Err(cause),
        };
        let metadata = file.metadata()?;
        if !metadata.is_file() || super::is_link(&metadata) || file_identity(&file)? != identity {
            return Err(io::Error::other("A delivery file changed ownership."));
        }
        discard_owned(&file)
    }

    pub(in super::super) fn publish_directory(
        directory: &File,
        destination: &Path,
    ) -> io::Result<()> {
        let name = wide(destination);
        let name_bytes = std::mem::size_of_val(&name[..name.len() - 1]);
        let offset = std::mem::offset_of!(RenameInfo, first_name_unit);
        let size = offset
            .checked_add(std::mem::size_of_val(name.as_slice()))
            .map(|size| size.max(std::mem::size_of::<RenameInfo>()))
            .and_then(|size| u32::try_from(size).ok())
            .ok_or_else(|| io::Error::other("The destination path is too long."))?;
        let name_bytes = u32::try_from(name_bytes)
            .map_err(|_| io::Error::other("The destination name is too long."))?;
        let mut buffer = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
        let header = buffer.as_mut_ptr().cast::<RenameInfo>();
        // SAFETY: usize alignment is adequate for RenameInfo; the allocation
        // covers the fixed fields and complete NUL-terminated UTF-16 path.
        unsafe {
            (*header).replace = 0;
            (*header).root = ptr::null_mut();
            (*header).name_bytes = name_bytes;
            ptr::copy_nonoverlapping(
                name.as_ptr(),
                buffer.as_mut_ptr().cast::<u8>().add(offset).cast::<u16>(),
                name.len(),
            );
        }
        // SAFETY: directory is the originally opened DELETE-access handle;
        // FileRenameInfo with ReplaceIfExists false cannot overwrite output.
        if unsafe {
            SetFileInformationByHandle(directory.as_raw_handle(), 3, buffer.as_ptr().cast(), size)
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub(in super::super) fn discard_owned(directory: &File) -> io::Result<()> {
        let delete: u8 = 1;
        // SAFETY: FileDispositionInfo takes a pointer to one Win32 BOOLEAN.
        if unsafe {
            SetFileInformationByHandle(
                directory.as_raw_handle(),
                4,
                (&delete as *const u8).cast(),
                1,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub(crate) fn windows_directory() -> io::Result<PathBuf> {
        let mut path = vec![0u16; 32_768];
        // SAFETY: the writable buffer has the specified capacity.
        let len =
            unsafe { GetSystemWindowsDirectoryW(path.as_mut_ptr(), path.len() as u32) } as usize;
        if len == 0 || len >= path.len() {
            return Err(io::Error::last_os_error());
        }
        Ok(OsString::from_wide(&path[..len]).into())
    }
}

#[cfg(target_arch = "x86_64")]
pub(crate) use build_paths::windows_directory;
#[cfg(target_arch = "x86_64")]
pub(super) use build_paths::{
    create_private_directory, discard_owned, discard_owned_file, file_identity, lock_delivery_file,
    open_writable_ancestor, own_delivery, publish_directory,
};
