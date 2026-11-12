//! Bounded artwork import and deterministic PortableApps image generation.

use image::{imageops, ImageEncoder, ImageFormat, ImageReader, Limits, Rgba, RgbaImage};
use std::fs::File;
use std::io::{Cursor, Read};
use std::path::Path;
use std::sync::Arc;

const MAX_IMPORT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_DIMENSION: u32 = 4096;
const PNG_SIZES: [u32; 5] = [16, 32, 75, 128, 256];
pub const SIDEBAR_WIDTH: u32 = 328;
pub const SIDEBAR_HEIGHT: u32 = 628;
pub(crate) const SIDEBAR_PATH: &str = "Other/Source/PortableForgeInstallerArtwork.bmp";
pub(crate) const BMP_BYTES: usize = 54 + (SIDEBAR_WIDTH * SIDEBAR_HEIGHT * 3) as usize;

#[derive(Clone, Debug)]
pub struct Raster(Arc<RgbaImage>);

impl Raster {
    pub fn load(path: &Path) -> Result<Self, String> {
        let file =
            File::open(path).map_err(|error| format!("Could not open the artwork: {error}"))?;
        let metadata = file.metadata().map_err(|error| error.to_string())?;
        if !metadata.is_file() || metadata.len() > MAX_IMPORT_BYTES {
            return Err("Choose a PNG or ICO file no larger than 16 MiB.".into());
        }
        let mut bytes = Vec::new();
        file.take(MAX_IMPORT_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("Could not read the artwork: {error}"))?;
        if bytes.len() as u64 > MAX_IMPORT_BYTES {
            return Err("Choose a PNG or ICO file no larger than 16 MiB.".into());
        }
        Self::decode(&bytes)
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, String> {
        let format = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            ImageFormat::Png
        } else if bytes.starts_with(&[0, 0, 1, 0]) {
            ImageFormat::Ico
        } else {
            return Err("Choose a PNG or ICO image.".into());
        };
        let mut limits = Limits::default();
        limits.max_image_width = Some(MAX_DIMENSION);
        limits.max_image_height = Some(MAX_DIMENSION);
        limits.max_alloc = Some(96 * 1024 * 1024);
        let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
        reader.limits(limits);
        let image = reader
            .decode()
            .map_err(|error| format!("Could not read the PNG or ICO image: {error}"))?
            .into_rgba8();
        Ok(Self(Arc::new(image)))
    }

    pub fn width(&self) -> u32 {
        self.0.width()
    }

    pub fn height(&self) -> u32 {
        self.0.height()
    }

    pub fn rgba(&self) -> &[u8] {
        self.0.as_raw()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FitMode {
    #[default]
    Fit,
    Crop,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Transform {
    /// Clockwise quarter turns, followed by flips in the displayed axes.
    pub quarter_turns: u8,
    pub flip_horizontal: bool,
    pub flip_vertical: bool,
    pub fit: FitMode,
}

#[derive(Clone, Debug)]
pub struct IconArtwork {
    pub source: Raster,
    pub transform: Transform,
}

#[derive(Clone, Debug)]
pub struct SidebarArtwork {
    /// A missing import uses the effective application icon.
    pub source: Option<Raster>,
    pub transform: Transform,
    pub background: [u8; 3],
}

#[derive(Clone, Debug, Default)]
pub struct Artwork {
    pub icon: Option<IconArtwork>,
    pub sidebar: Option<SidebarArtwork>,
}

pub fn render_icon(artwork: &IconArtwork, size: u32) -> Result<Raster, String> {
    if !(1..=512).contains(&size) {
        return Err("Choose an icon preview size between 1 and 512 pixels.".into());
    }
    Ok(Raster(Arc::new(render(
        &artwork.source,
        artwork.transform,
        size,
        size,
    ))))
}

pub fn render_sidebar(
    artwork: &SidebarArtwork,
    effective_icon: Option<&Raster>,
) -> Result<Raster, String> {
    let [r, g, b] = artwork.background;
    let mut canvas = RgbaImage::from_pixel(SIDEBAR_WIDTH, SIDEBAR_HEIGHT, Rgba([r, g, b, 255]));
    if let Some(source) = artwork.source.as_ref().or(effective_icon) {
        let foreground = render(source, artwork.transform, SIDEBAR_WIDTH, SIDEBAR_HEIGHT);
        for (pixel, over) in canvas.pixels_mut().zip(foreground.pixels()) {
            let alpha = u32::from(over[3]);
            for channel in 0..3 {
                pixel[channel] = ((u32::from(over[channel]) * alpha
                    + u32::from(pixel[channel]) * (255 - alpha)
                    + 127)
                    / 255) as u8;
            }
        }
    }
    Ok(Raster(Arc::new(canvas)))
}

fn render(source: &Raster, transform: Transform, width: u32, height: u32) -> RgbaImage {
    let mut image = match transform.quarter_turns % 4 {
        1 => imageops::rotate90(source.0.as_ref()),
        2 => imageops::rotate180(source.0.as_ref()),
        3 => imageops::rotate270(source.0.as_ref()),
        _ => source.0.as_ref().clone(),
    };
    if transform.flip_horizontal {
        imageops::flip_horizontal_in_place(&mut image);
    }
    if transform.flip_vertical {
        imageops::flip_vertical_in_place(&mut image);
    }
    let (w, h) = image.dimensions();
    let wider = w * height > h * width;
    if transform.fit == FitMode::Crop {
        let (crop_w, crop_h) = if wider {
            ((h * width / height).max(1), h)
        } else {
            (w, (w * height / width).max(1))
        };
        let cropped = imageops::crop(
            &mut image,
            (w - crop_w) / 2,
            (h - crop_h) / 2,
            crop_w,
            crop_h,
        )
        .to_image();
        return resize(cropped, width, height);
    }

    let (fit_w, fit_h) = if wider {
        (width, (h * width / w).max(1))
    } else {
        ((w * height / h).max(1), height)
    };
    let fit = resize(image, fit_w, fit_h);
    let mut canvas = RgbaImage::new(width, height);
    imageops::replace(
        &mut canvas,
        &fit,
        i64::from((width - fit_w) / 2),
        i64::from((height - fit_h) / 2),
    );
    canvas
}

// Resampling in premultiplied alpha prevents transparent pixels from bleeding
// hidden color into the visible edge. Identical dimensions preserve exact bytes.
fn resize(mut image: RgbaImage, width: u32, height: u32) -> RgbaImage {
    if image.dimensions() == (width, height) {
        return image;
    }
    for pixel in image.pixels_mut() {
        let alpha = u16::from(pixel[3]);
        for channel in &mut pixel.0[..3] {
            *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
        }
    }
    let mut image = imageops::resize(&image, width, height, imageops::FilterType::Lanczos3);
    for pixel in image.pixels_mut() {
        let alpha = u16::from(pixel[3]);
        for channel in &mut pixel.0[..3] {
            *channel = (u16::from(*channel) * 255 + alpha / 2)
                .checked_div(alpha)
                .unwrap_or(0)
                .min(255) as u8;
        }
    }
    image
}

fn png(raster: &Raster) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(
            raster.rgba(),
            raster.width(),
            raster.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|error| format!("Could not encode the application icon: {error}"))?;
    Ok(bytes)
}

pub(crate) struct PackageIcons {
    pub(crate) ico: Vec<u8>,
    pub(crate) png: Vec<(u32, Vec<u8>)>,
}

#[cfg(target_arch = "x86_64")]
pub(crate) fn default_application_icon() -> Raster {
    Raster(Arc::new(RgbaImage::from_fn(64, 64, |x, y| {
        if !(6..58).contains(&x) || !(6..58).contains(&y) {
            Rgba([0, 0, 0, 0])
        } else if (15..49).contains(&x) && (17..23).contains(&y) {
            Rgba([180, 205, 226, 255])
        } else if (15..49).contains(&x) && (28..48).contains(&y) {
            Rgba([40, 61, 77, 255])
        } else {
            Rgba([61, 102, 133, 255])
        }
    })))
}

pub(crate) fn package_icons(artwork: &IconArtwork) -> Result<PackageIcons, String> {
    let mut frames = Vec::new();
    let mut pngs = Vec::new();
    for size in PNG_SIZES {
        let icon = render_icon(artwork, size)?;
        let png = png(&icon)?;
        pngs.push((size, png));
    }
    for size in [16, 32, 48] {
        let icon = render_icon(artwork, size)?;
        for depth in [8, 32] {
            frames.push((size, depth, dib_icon(&icon, depth)));
        }
    }
    frames.push((
        256,
        32,
        pngs.last().expect("256 is a required PNG size").1.clone(),
    ));

    let mut bytes = vec![0, 0, 1, 0];
    put_u16(&mut bytes, frames.len() as u16);
    let mut offset = (6 + 16 * frames.len()) as u32;
    for (size, depth, frame) in &frames {
        // ICO directory value 0 represents both dimension 256 and 256 colors.
        bytes.extend([*size as u8, *size as u8, 0, 0]);
        put_u16(&mut bytes, 1);
        put_u16(&mut bytes, *depth);
        put_u32(&mut bytes, frame.len() as u32);
        put_u32(&mut bytes, offset);
        offset += frame.len() as u32;
    }
    for (_, _, frame) in frames {
        bytes.extend(frame);
    }
    Ok(PackageIcons {
        ico: bytes,
        png: pngs,
    })
}

fn dib_icon(icon: &Raster, depth: u16) -> Vec<u8> {
    let size = icon.width();
    let mut bytes = dib_header(size, 2 * size, depth, 0);
    if depth == 8 {
        for index in 0..=255u8 {
            bytes.extend([
                (index & 3) * 85,
                (((index >> 2) & 7) as u16 * 255 / 7) as u8,
                ((index >> 5) as u16 * 255 / 7) as u8,
                0,
            ]);
        }
    }
    // Supported small icon sizes are multiples of four, so XOR rows
    // are already DWORD-aligned at both depths.
    for y in (0..size).rev() {
        for x in 0..size {
            let [r, g, b, a] = icon.0.get_pixel(x, y).0;
            if depth == 8 {
                bytes.push((r & 0xe0) | ((g >> 3) & 0x1c) | (b >> 6));
            } else {
                bytes.extend([b, g, r, a]);
            }
        }
    }
    let mask_stride = size.div_ceil(32) as usize * 4;
    for y in (0..size).rev() {
        let offset = bytes.len();
        bytes.resize(offset + mask_stride, 0);
        for x in 0..size {
            let alpha = icon.0.get_pixel(x, y)[3];
            if (depth == 8 && alpha < 128) || alpha == 0 {
                bytes[offset + x as usize / 8] |= 0x80 >> (x % 8);
            }
        }
    }
    bytes
}

fn put_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend(value.to_le_bytes());
}

fn put_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend(value.to_le_bytes());
}

fn dib_header(width: u32, height: u32, depth: u16, image_bytes: u32) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(40);
    for value in [40, width, height] {
        put_u32(&mut bytes, value);
    }
    put_u16(&mut bytes, 1);
    put_u16(&mut bytes, depth);
    for value in [0, image_bytes, 0, 0, if depth == 8 { 256 } else { 0 }, 0] {
        put_u32(&mut bytes, value);
    }
    bytes
}

fn bmp_header() -> Vec<u8> {
    let mut bytes = b"BM".to_vec();
    for value in [BMP_BYTES as u32, 0, 54] {
        put_u32(&mut bytes, value);
    }
    bytes.extend(dib_header(
        SIDEBAR_WIDTH,
        SIDEBAR_HEIGHT,
        24,
        (BMP_BYTES - 54) as u32,
    ));
    bytes
}

pub(crate) fn sidebar_bmp(
    artwork: &SidebarArtwork,
    effective_icon: Option<&Raster>,
) -> Result<Vec<u8>, String> {
    let image = render_sidebar(artwork, effective_icon)?;
    let mut bytes = bmp_header();
    bytes.reserve(BMP_BYTES - bytes.len());
    // 328 * 3 is already DWORD-aligned. BMP stores rows bottom-up.
    for row in image.0.rows().rev() {
        for pixel in row {
            bytes.extend([pixel[2], pixel[1], pixel[0]]);
        }
    }
    Ok(bytes)
}

pub(crate) fn validate_sidebar_bmp(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() != BMP_BYTES || !bytes.starts_with(&bmp_header()) {
        return Err("Installer artwork must be a canonical 328 × 628, 24-bit BMP.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Raster {
        Raster(Arc::new(RgbaImage::from_fn(4, 2, |x, y| {
            Rgba([
                (x * 75) as u8,
                (y * 240) as u8,
                17,
                if x == 0 { 0 } else { 255 },
            ])
        })))
    }

    #[test]
    fn icons_are_deterministic_and_contain_all_official_frames() {
        let icon = IconArtwork {
            source: sample(),
            transform: Transform::default(),
        };
        let first = package_icons(&icon).unwrap();
        let again = package_icons(&icon).unwrap();
        assert_eq!(first.ico, again.ico);
        assert_eq!(first.png, again.png);
        assert_eq!(
            first.png.iter().map(|(size, _)| *size).collect::<Vec<_>>(),
            PNG_SIZES
        );
        for (size, bytes) in &first.png {
            let decoded = Raster::decode(bytes).unwrap();
            let preview = render_icon(&icon, *size).unwrap();
            assert_eq!((decoded.width(), decoded.height()), (*size, *size));
            assert_eq!(decoded.rgba(), preview.rgba());
        }
        assert_eq!(&first.ico[..6], &[0, 0, 1, 0, 7, 0]);
        let expected = [
            (16, 8),
            (16, 32),
            (32, 8),
            (32, 32),
            (48, 8),
            (48, 32),
            (256, 32),
        ];
        let mut next_offset = 6 + 16 * expected.len();
        for (entry, (size, bits)) in first.ico[6..next_offset]
            .as_chunks::<16>()
            .0
            .iter()
            .zip(expected)
        {
            assert_eq!(&entry[..2], &[size as u8, size as u8]);
            assert_eq!(u16::from_le_bytes(entry[6..8].try_into().unwrap()), bits);
            let length = u32::from_le_bytes(entry[8..12].try_into().unwrap()) as usize;
            let offset = u32::from_le_bytes(entry[12..16].try_into().unwrap()) as usize;
            assert_eq!(offset, next_offset);
            let frame = &first.ico[offset..offset + length];
            if size == 256 {
                assert!(frame.starts_with(b"\x89PNG\r\n\x1a\n"));
            } else {
                let mask_stride = (size as usize).div_ceil(32) * 4;
                let palette = if bits == 8 { 1024 } else { 0 };
                assert_eq!(
                    length,
                    40 + palette
                        + size as usize * (size as usize * bits as usize / 8 + mask_stride)
                );
                assert_eq!(
                    u32::from_le_bytes(frame[8..12].try_into().unwrap()),
                    2 * size
                );
                let mask = &frame[length - size as usize * mask_stride..];
                assert_eq!(mask[0] & 0x80, 0x80);
            }
            let mut single = vec![0, 0, 1, 0, 1, 0];
            single.extend(&entry[..12]);
            put_u32(&mut single, 22);
            single.extend(frame);
            let decoded = Raster::decode(&single).unwrap();
            assert_eq!((decoded.width(), decoded.height()), (size, size));
            next_offset += length;
        }
        assert_eq!(next_offset, first.ico.len());
    }

    #[test]
    fn rotation_and_flips_use_displayed_axes() {
        let image = render(
            &sample(),
            Transform {
                quarter_turns: 1,
                flip_horizontal: true,
                ..Transform::default()
            },
            2,
            4,
        );
        assert_eq!(image.get_pixel(0, 1).0, [75, 0, 17, 255]);
        assert_eq!(image.get_pixel(1, 1).0, [75, 240, 17, 255]);
    }

    #[test]
    fn fit_preserves_padding_and_crop_fills() {
        let source = Raster(Arc::new(RgbaImage::from_pixel(
            4,
            2,
            Rgba([32, 16, 64, 255]),
        )));
        let fit = render(&source, Transform::default(), 4, 4);
        let crop = render(
            &source,
            Transform {
                fit: FitMode::Crop,
                ..Transform::default()
            },
            4,
            4,
        );
        assert_eq!(fit.get_pixel(0, 0)[3], 0);
        assert_eq!(fit.get_pixel(0, 1)[3], 255);
        assert!(crop.pixels().all(|pixel| pixel[3] == 255));
    }

    #[test]
    fn canonical_sidebar_matches_opaque_preview_and_rejects_other_headers() {
        let artwork = SidebarArtwork {
            source: Some(sample()),
            transform: Transform::default(),
            background: [9, 18, 27],
        };
        let preview = render_sidebar(&artwork, None).unwrap();
        let mut bytes = sidebar_bmp(&artwork, None).unwrap();
        validate_sidebar_bmp(&bytes).unwrap();
        assert_eq!(bytes.len(), BMP_BYTES);
        for (bytes, pixel) in bytes[54..]
            .as_chunks::<3>()
            .0
            .iter()
            .zip(preview.0.rows().rev().flatten())
        {
            assert_eq!(*bytes, [pixel[2], pixel[1], pixel[0]]);
            assert_eq!(pixel[3], 255);
        }
        bytes[28] = 32;
        assert!(validate_sidebar_bmp(&bytes).is_err());
        assert!(Raster::decode(&bytes).is_err());
    }
}
