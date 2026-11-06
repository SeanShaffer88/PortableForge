//! Extract and verify icons using read-only Windows resource mapping.

use std::ffi::c_void;
use std::fs::{File, OpenOptions};
use std::io::{self, Cursor, Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr;
use std::slice;

use image::{DynamicImage, ImageDecoder, ImageEncoder, ImageFormat, ImageReader, RgbaImage};

const MAX_ICON_BYTES: usize = 16 * 1024 * 1024;
const MAX_IMAGES: usize = 256;
const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
// Win32 exposes the pinned NSIS installer's primary icon as group 103.
const INSTALLER_ICON_GROUP: u16 = 103;
const LOAD_LIBRARY_AS_DATAFILE_EXCLUSIVE: u32 = 0x40;
const LOAD_LIBRARY_AS_IMAGE_RESOURCE: u32 = 0x20;
const RESOURCE_ENUM_LN: u32 = 0x0001;
const RESOURCE_ENUM_VALIDATE: u32 = 0x0008;

type Handle = *mut c_void;
type ResourceCallback = unsafe extern "system" fn(Handle, *const u16, *mut u16, isize) -> i32;
type Preview = ((bool, bool, u32, u16), Vec<u8>);

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryExW(path: *const u16, file: Handle, flags: u32) -> Handle;
    fn FreeLibrary(module: Handle) -> i32;
    fn EnumResourceNamesExW(
        module: Handle,
        kind: *const u16,
        callback: ResourceCallback,
        state: isize,
        flags: u32,
        language: u16,
    ) -> i32;
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
            .map_err(|e| format!("Could not resolve the file for icon inspection: {e}"))?;
        let mut wide: Vec<_> = full_path.as_os_str().encode_wide().collect();
        if wide.contains(&0) {
            return Err("The icon source path contains an invalid character.".into());
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
                "Could not open the file's icon resources: {}",
                io::Error::last_os_error()
            ));
        }
        let module = Self(handle);
        // The low handle bits identify a datafile or image resource mapping.
        if handle as usize & 3 == 0 {
            return Err("The file did not open as a resource mapping.".into());
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
            return Err(format!("Icon resource {kind}/{id} is missing."));
        }
        self.resource_bytes(resource)
    }

    fn first_icon_group(&self) -> Result<&[u8], String> {
        unsafe extern "system" fn find_first(
            module: Handle,
            kind: *const u16,
            name: *mut u16,
            state: isize,
        ) -> i32 {
            // SAFETY: Win32 provides the live module and resource identifier
            // for this callback. The state pointer references its caller's
            // Handle slot for the duration of this synchronous enumeration.
            unsafe { *(state as *mut Handle) = FindResourceW(module, name, kind) };
            0 // Stop after the first group, whether or not it is readable.
        }

        let mut resource: Handle = ptr::null_mut();
        // SAFETY: The resource mapping and state pointer remain live until
        // the synchronous callback finishes. Search the image itself.
        unsafe {
            EnumResourceNamesExW(
                self.0,
                RT_GROUP_ICON as usize as *const u16,
                find_first,
                (&mut resource as *mut Handle) as isize,
                RESOURCE_ENUM_LN | RESOURCE_ENUM_VALIDATE,
                0,
            )
        };
        if resource.is_null() {
            return Err("The captured app has no readable embedded icon.".into());
        }
        self.resource_bytes(resource)
    }

    fn resource_bytes(&self, resource: Handle) -> Result<&[u8], String> {
        // SAFETY: FindResourceW returned a resource owned by this live mapping.
        let size = unsafe { SizeofResource(self.0, resource) } as usize;
        if size == 0 || size > MAX_ICON_BYTES {
            return Err("An icon resource has an invalid size.".into());
        }
        // SAFETY: LoadResource is called with the live mapping and its resource.
        let data = unsafe { LoadResource(self.0, resource) };
        if data.is_null() {
            return Err("Could not read an icon resource.".into());
        }
        // SAFETY: data is the successful result of LoadResource.
        let bytes = unsafe { LockResource(data) };
        if bytes.is_null() {
            return Err("Could not access an icon resource.".into());
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

struct IconImage<'a> {
    header: &'a [u8],
    bytes: &'a [u8],
}

fn encode_ico(images: &[IconImage<'_>]) -> Result<Vec<u8>, String> {
    if images.is_empty() || images.len() > MAX_IMAGES {
        return Err("The app icon has an invalid image count.".into());
    }
    let directory_end = 6 + images.len() * 16;
    let size = images
        .iter()
        .try_fold(directory_end, |size, image| {
            size.checked_add(image.bytes.len())
        })
        .filter(|&size| size <= MAX_ICON_BYTES)
        .ok_or("The app icon exceeds the 16 MiB limit.")?;
    let mut ico = Vec::with_capacity(size);
    ico.extend_from_slice(&[0, 0, 1, 0]);
    ico.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = directory_end;
    for image in images {
        let header = image.header;
        if header.len() != 12
            || header[3] != 0
            || image.bytes.is_empty()
            || image_size(header) != image.bytes.len()
        {
            return Err("The app icon contains an invalid directory entry.".into());
        }
        ico.extend_from_slice(header);
        ico.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += image.bytes.len();
    }
    for image in images {
        ico.extend_from_slice(image.bytes);
    }
    Ok(ico)
}

fn decode_icon(image: &IconImage<'_>) -> Result<DynamicImage, String> {
    // Decode each entry separately so the decoder cannot select a different
    // source image from the one used for a specific PNG size.
    let ico = encode_ico(std::slice::from_ref(image))?;
    let mut reader = ImageReader::with_format(Cursor::new(ico), ImageFormat::Ico);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(256);
    limits.max_image_height = Some(256);
    limits.max_alloc = Some(MAX_ICON_BYTES as u64);
    reader.limits(limits);
    let decoder = reader
        .into_decoder()
        .map_err(|e| format!("Could not read the embedded app icon: {e}"))?;
    let dimension = |byte| if byte == 0 { 256 } else { u32::from(byte) };
    let expected = (dimension(image.header[0]), dimension(image.header[1]));
    if decoder.dimensions() != expected {
        return Err("The app icon image dimensions do not match its directory.".into());
    }
    DynamicImage::from_decoder(decoder)
        .map_err(|e| format!("Could not decode the embedded app icon: {e}"))
}

fn encode_png(image: &RgbaImage) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| format!("Could not write the app icon preview: {e}"))?;
    Ok(bytes)
}

pub(crate) fn extract_application_icons(executable: &Path, app_info: &Path) -> Result<(), String> {
    let module = ResourceModule::open(executable)?;
    let group = module.first_icon_group()?;
    let entries = icon_entries(group, 14)
        .filter(|entries| group.len() == 6 + entries.len() * 14)
        .ok_or("The app icon group has an invalid directory.")?;
    let images = entries
        .map(|entry| {
            let id = u16::from_le_bytes([entry[12], entry[13]]);
            Ok(IconImage {
                header: &entry[..12],
                bytes: module.bytes(RT_ICON, id)?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let ico = encode_ico(&images)?;
    // Keep only the best representation for each size. Validate every entry
    // before any resource bytes are handed to the external compiler.
    let mut previews: [Option<Preview>; 2] = [None, None];
    for entry in &images {
        let source = decode_icon(entry)?;
        for (size, preview) in [16_u32, 32].into_iter().zip(&mut previews) {
            let (width, height) = (source.width(), source.height());
            let exact = width == size && height == size;
            let larger = width.min(height) >= size;
            let distance = width.abs_diff(size) + height.abs_diff(size);
            let depth = u16::from_le_bytes([entry.header[6], entry.header[7]]);
            let score = (exact, larger, u32::MAX - distance, depth);
            if preview.as_ref().is_none_or(|(best, _)| score > *best) {
                let image = if exact {
                    source.to_rgba8()
                } else {
                    let resized = source
                        .resize(size, size, image::imageops::FilterType::Lanczos3)
                        .into_rgba8();
                    let mut canvas = RgbaImage::new(size, size);
                    image::imageops::overlay(
                        &mut canvas,
                        &resized,
                        i64::from((size - resized.width()) / 2),
                        i64::from((size - resized.height()) / 2),
                    );
                    canvas
                };
                *preview = Some((score, encode_png(&image)?));
            }
        }
    }
    let [Some((_, small)), Some((_, large))] = previews else {
        return Err("The app icon has no usable images.".into());
    };
    for (name, bytes) in [
        ("appicon.ico", ico),
        ("appicon_16.png", small),
        ("appicon_32.png", large),
    ] {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(app_info.join(name))
            .and_then(|mut file| file.write_all(&bytes))
            .map_err(|e| format!("Could not create {name} in the new package: {e}"))?;
    }
    Ok(())
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
    use super::{decode_icon, encode_ico, ico_images, IconImage};

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

    #[test]
    fn writes_original_image_bytes_into_the_ico() {
        let header = [16, 16, 0, 0, 1, 0, 32, 0, 3, 0, 0, 0];
        let image = IconImage {
            header: &header,
            bytes: &[1, 2, 3],
        };
        let ico = encode_ico(&[image]).unwrap();
        assert_eq!(&ico[6..18], &header);
        assert_eq!(ico_images(&ico).unwrap(), [&[1, 2, 3][..]]);
    }

    #[test]
    fn rejects_icon_images_with_unmatched_dimensions() {
        let png = super::encode_png(&image::RgbaImage::new(32, 32)).unwrap();
        let mut header = [16, 16, 0, 0, 1, 0, 32, 0, 0, 0, 0, 0];
        header[8..12].copy_from_slice(&(png.len() as u32).to_le_bytes());
        assert!(decode_icon(&IconImage {
            header: &header,
            bytes: &png,
        })
        .is_err());
    }
}
