//! Verify the generated installer icon using read-only Windows resource mapping.

use std::ffi::c_void;
use std::fs::File;
use std::io::{self, Read};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr;
use std::slice;

const MAX_ICON_BYTES: usize = 16 * 1024 * 1024;
const MAX_IMAGES: usize = 256;
const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
// Win32 exposes the pinned NSIS installer's primary icon as group 103.
const INSTALLER_ICON_GROUP: u16 = 103;
const LOAD_LIBRARY_AS_DATAFILE_EXCLUSIVE: u32 = 0x40;
const LOAD_LIBRARY_AS_IMAGE_RESOURCE: u32 = 0x20;

type Handle = *mut c_void;

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryExW(path: *const u16, file: Handle, flags: u32) -> Handle;
    fn FreeLibrary(module: Handle) -> i32;
    fn FindResourceW(module: Handle, name: *const u16, kind: *const u16) -> Handle;
    fn SizeofResource(module: Handle, resource: Handle) -> u32;
    fn LoadResource(module: Handle, resource: Handle) -> Handle;
    fn LockResource(resource: Handle) -> *const u8;
}

struct ResourceModule(Handle);

impl ResourceModule {
    fn open(path: &Path) -> Result<Self, String> {
        let full_path = path
            .canonicalize()
            .map_err(|e| format!("Could not resolve the generated installer: {e}"))?;
        let mut wide: Vec<_> = full_path.as_os_str().encode_wide().collect();
        if wide.contains(&0) {
            return Err("The installer path contains an invalid character.".into());
        }
        wide.push(0);
        // SAFETY: The path is absolute and NUL-terminated. These flags request
        // a resource mapping, without loading imports or initializing the image.
        let handle = unsafe {
            LoadLibraryExW(
                wide.as_ptr(),
                ptr::null_mut(),
                LOAD_LIBRARY_AS_DATAFILE_EXCLUSIVE | LOAD_LIBRARY_AS_IMAGE_RESOURCE,
            )
        };
        if handle.is_null() {
            return Err(format!(
                "Could not open the generated installer resources: {}",
                io::Error::last_os_error()
            ));
        }
        let module = Self(handle);
        // The low handle bits identify a datafile or image resource mapping.
        if handle as usize & 3 == 0 {
            return Err("The generated installer did not open as a resource mapping.".into());
        }
        Ok(module)
    }

    fn bytes(&self, kind: u16, id: u16) -> Result<&[u8], String> {
        // SAFETY: The module mapping is live. Win32 accepts 16-bit numeric
        // resource names and types encoded as pointer values (MAKEINTRESOURCE).
        let resource = unsafe {
            FindResourceW(
                self.0,
                id as usize as *const u16,
                kind as usize as *const u16,
            )
        };
        if resource.is_null() {
            return Err(format!("Installer icon resource {kind}/{id} is missing."));
        }
        // SAFETY: FindResourceW returned a resource owned by this live mapping.
        let size = unsafe { SizeofResource(self.0, resource) } as usize;
        if size == 0 || size > MAX_ICON_BYTES {
            return Err(format!(
                "Installer icon resource {kind}/{id} has an invalid size."
            ));
        }
        // SAFETY: LoadResource is called with the live mapping and its resource.
        let data = unsafe { LoadResource(self.0, resource) };
        if data.is_null() {
            return Err(format!(
                "Could not read installer icon resource {kind}/{id}."
            ));
        }
        // SAFETY: data is the successful result of LoadResource.
        let bytes = unsafe { LockResource(data) };
        if bytes.is_null() {
            return Err(format!(
                "Could not access installer icon resource {kind}/{id}."
            ));
        }
        // SAFETY: Win32 exposes SizeofResource bytes at this address. The
        // returned borrow cannot outlive the mapping, which Drop releases.
        Ok(unsafe { slice::from_raw_parts(bytes, size) })
    }
}

impl Drop for ResourceModule {
    fn drop(&mut self) {
        // SAFETY: This instance owns one successful LoadLibraryExW mapping.
        unsafe { FreeLibrary(self.0) };
    }
}

fn icon_entries(bytes: &[u8], entry_size: usize) -> Option<slice::ChunksExact<'_, u8>> {
    if !bytes.starts_with(&[0, 0, 1, 0]) {
        return None;
    }
    let count = u16::from_le_bytes(bytes.get(4..6)?.try_into().ok()?) as usize;
    if count == 0 || count > MAX_IMAGES {
        return None;
    }
    Some(
        bytes
            .get(6..6 + entry_size * count)?
            .chunks_exact(entry_size),
    )
}

fn image_size(entry: &[u8]) -> usize {
    u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]) as usize
}

fn ico_images(bytes: &[u8]) -> Result<Vec<&[u8]>, String> {
    let entries = icon_entries(bytes, 16).ok_or("The application ICO directory is invalid.")?;
    let directory_end = 6 + entries.len() * 16;
    let mut ranges = Vec::with_capacity(entries.len());
    for entry in entries {
        let offset = u32::from_le_bytes([entry[12], entry[13], entry[14], entry[15]]) as usize;
        let size = image_size(entry);
        if size == 0 || offset < directory_end {
            return Err("The application ICO contains an invalid image entry.".into());
        }
        let end = offset
            .checked_add(size)
            .filter(|&end| end <= bytes.len())
            .ok_or("An application ICO image extends past the end of the file.")?;
        ranges.push((offset, end));
    }
    ranges.sort_unstable();
    if ranges.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return Err("The application ICO contains overlapping images.".into());
    }
    let mut images: Vec<_> = ranges
        .into_iter()
        .map(|(start, end)| &bytes[start..end])
        .collect();
    images.sort_unstable();
    Ok(images)
}

pub(super) fn verify(ico: &Path, installer: &Path) -> Result<(), String> {
    let mut ico_bytes = Vec::new();
    File::open(ico)
        .and_then(|file| {
            file.take(MAX_ICON_BYTES as u64 + 1)
                .read_to_end(&mut ico_bytes)
        })
        .map_err(|e| format!("Could not read the application ICO: {e}"))?;
    if ico_bytes.len() > MAX_ICON_BYTES {
        return Err("The application ICO exceeds the 16 MiB verification limit.".into());
    }
    let expected = ico_images(&ico_bytes)?;
    let expected_bytes = expected.iter().map(|image| image.len()).sum::<usize>();
    let module = ResourceModule::open(installer)?;
    let group = module.bytes(RT_GROUP_ICON, INSTALLER_ICON_GROUP)?;
    let entries = icon_entries(group, 14)
        .filter(|entries| group.len() == 6 + entries.len() * 14)
        .ok_or("The installer icon group has an invalid directory.")?;
    if entries.len() != expected.len() {
        return Err("The installer icon has a different image count.".into());
    }
    let mut actual = Vec::with_capacity(entries.len());
    let mut actual_bytes = 0;
    for entry in entries {
        let image_id = u16::from_le_bytes([entry[12], entry[13]]);
        let image = module.bytes(RT_ICON, image_id)?;
        if image.len() != image_size(entry) {
            return Err(format!(
                "Installer icon image {image_id} has an unexpected size."
            ));
        }
        actual_bytes += image.len();
        if actual_bytes > expected_bytes {
            return Err("The installer icon exceeds the application ICO image size.".into());
        }
        actual.push(image);
    }
    actual.sort_unstable();
    if actual != expected {
        return Err("The installer icon does not match the application ICO.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::ico_images;

    fn example_ico() -> Vec<u8> {
        let mut bytes = vec![0, 0, 1, 0, 2, 0];
        for (offset, size) in [(38_u32, 2_u32), (40, 1)] {
            bytes.extend([16, 16, 0, 0, 1, 0, 32, 0]);
            bytes.extend(size.to_le_bytes());
            bytes.extend(offset.to_le_bytes());
        }
        bytes.extend([7, 8, 4]);
        bytes
    }

    #[test]
    fn reads_and_sorts_ico_image_payloads() {
        assert_eq!(ico_images(&example_ico()).unwrap(), [&[4][..], &[7, 8][..]]);
    }

    #[test]
    fn rejects_invalid_ico_directory_and_image_bounds() {
        let bytes = example_ico();
        for invalid in [&bytes[..5], &bytes[..20], &bytes[..40]] {
            assert!(ico_images(invalid).is_err());
        }
        for offset in [0_u32, 39, u32::MAX] {
            let mut invalid = bytes.clone();
            invalid[34..38].copy_from_slice(&offset.to_le_bytes());
            assert!(ico_images(&invalid).is_err());
        }
    }
}
