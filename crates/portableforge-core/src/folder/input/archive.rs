//! Bound ZIP metadata and verify raw paths before passing data to the decoder.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use zip::{CompressionMethod, ZipArchive};

use super::{
    bounded_total, check_cancel, error, io_error, write_bounded, ScanError, MAX_INPUT_BYTES,
};
use crate::folder::{MAX_DEPTH, MAX_ENTRIES};

const MAX_DIRECTORY_BYTES: u64 = 32 * 1024 * 1024;
const MAX_NAME_BYTES: usize = 1024;
const FOOTER_BYTES: usize = 22;
const CENTRAL_BYTES: usize = 46;
const LOCAL_BYTES: usize = 30;
const DATA_DESCRIPTOR: u16 = 1 << 3;

struct Member {
    name: String,
    relative: PathBuf,
    directory: bool,
    flags: u16,
    method: u16,
    crc: u32,
    packed: u64,
    size: u64,
    local_start: u64,
    data_start: u64,
}

struct Node {
    relative: PathBuf,
    directory: bool,
    declared: bool,
}

struct Layout {
    members: Vec<Member>,
    nodes: BTreeMap<String, Node>,
    central_start: u64,
}

pub(super) fn unpack(
    file: &mut File,
    root: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<PathBuf, ScanError> {
    let layout = inspect(file, cancelled)?;
    check_cancel(cancelled)?;
    let mut archive = ZipArchive::new(file).map_err(zip_error)?;
    if archive.len() != layout.members.len()
        || archive.offset() != 0
        || archive.central_directory_start() != layout.central_start
    {
        return Err(invalid_zip());
    }
    // Each implicit and explicit directory is created exactly once. Using
    // create_dir also rejects filesystem name aliases that differ from our keys.
    for node in layout.nodes.values().filter(|node| node.directory) {
        check_cancel(cancelled)?;
        fs::create_dir(root.join(&node.relative)).map_err(io_error)?;
    }
    for (index, member) in layout.members.iter().enumerate() {
        check_cancel(cancelled)?;
        let mut entry = archive.by_index(index).map_err(zip_error)?;
        if entry.name_raw() != member.name.as_bytes()
            || entry.is_dir() != member.directory
            || entry.encrypted()
            || entry.compression() != method(member.method)?
            || entry.size() != member.size
            || entry.compressed_size() != member.packed
            || entry.crc32() != member.crc
            || entry.header_start() != member.local_start
            || entry.data_start() != Some(member.data_start)
        {
            return Err(invalid_zip());
        }
        if member.directory {
            if entry.read(&mut [0]).map_err(io_error)? != 0 {
                return Err(invalid_zip());
            }
        } else {
            write_bounded(entry, &root.join(&member.relative), member.size, cancelled)?;
        }
    }
    let mut top = layout
        .nodes
        .values()
        .filter(|node| node.relative.components().count() == 1);
    if let Some(node) = top.next().filter(|node| node.directory) {
        if top.next().is_none() {
            return Ok(root.join(&node.relative));
        }
    }
    Ok(root.to_path_buf())
}

fn inspect(file: &mut File, cancelled: &dyn Fn() -> bool) -> Result<Layout, ScanError> {
    let length = file.metadata().map_err(io_error)?.len();
    if length < FOOTER_BYTES as u64 || length >= MAX_INPUT_BYTES {
        return Err(error("Choose a ZIP archive smaller than 4 GiB."));
    }
    let tail_length = length.min((FOOTER_BYTES + u16::MAX as usize) as u64);
    let tail_start = length - tail_length;
    file.seek(SeekFrom::Start(tail_start)).map_err(io_error)?;
    let mut tail = vec![0; tail_length as usize];
    file.read_exact(&mut tail).map_err(io_error)?;
    let footer_offset = (0..=tail.len() - FOOTER_BYTES)
        .rev()
        .find(|&offset| {
            let bytes = &tail[offset..];
            bytes.starts_with(b"PK\x05\x06")
                && bytes.len() == FOOTER_BYTES + usize::from(u16_at(bytes, 20))
        })
        .ok_or_else(invalid_zip)?;
    let footer = &tail[footer_offset..];
    // Keep the decoder and the preflight on the same unambiguous end record.
    if footer[FOOTER_BYTES..]
        .windows(4)
        .any(|bytes| bytes == b"PK\x05\x06")
    {
        return Err(invalid_zip());
    }
    let count = usize::from(u16_at(footer, 10));
    let central_size = u64::from(u32_at(footer, 12));
    let central_start = u64::from(u32_at(footer, 16));
    let central_end = tail_start + footer_offset as u64;
    if u16_at(footer, 4) != 0 || u16_at(footer, 6) != 0 || usize::from(u16_at(footer, 8)) != count {
        return Err(error("Choose a ZIP stored in a single file."));
    }
    if count == 0 || count > MAX_ENTRIES || central_size > MAX_DIRECTORY_BYTES {
        return Err(error(
            "The ZIP is empty or exceeds the archive entry or metadata limit.",
        ));
    }
    if central_start == 0 || central_start + central_size != central_end {
        return Err(error(
            "Choose a standard ZIP archive. ZIP64 and split archives are unsupported.",
        ));
    }

    file.seek(SeekFrom::Start(central_start))
        .map_err(io_error)?;
    let mut central = vec![0; central_size as usize];
    file.read_exact(&mut central).map_err(io_error)?;
    let mut remaining = central.as_slice();
    let mut members = Vec::with_capacity(count);
    let mut nodes = BTreeMap::new();
    let mut total = 0;
    let mut local_metadata_bytes = 0;
    let mut ranges = Vec::with_capacity(count);
    for _ in 0..count {
        check_cancel(cancelled)?;
        let (member, rest) = parse_member(remaining)?;
        remaining = rest;
        total = bounded_total(total, member.size)?;
        insert_path(&mut nodes, &member)?;
        let (member, end) = check_local(file, member, central_start)?;
        local_metadata_bytes += end - member.local_start - member.packed;
        if local_metadata_bytes > MAX_DIRECTORY_BYTES {
            return Err(error("The ZIP exceeds the archive metadata limit."));
        }
        ranges.push((member.local_start, end));
        members.push(member);
    }
    ranges.sort_unstable();
    if !remaining.is_empty()
        || ranges[0].0 != 0
        || ranges.windows(2).any(|pair| pair[0].1 > pair[1].0)
    {
        return Err(invalid_zip());
    }
    Ok(Layout {
        members,
        nodes,
        central_start,
    })
}

fn parse_member(bytes: &[u8]) -> Result<(Member, &[u8]), ScanError> {
    let header = bytes.get(..CENTRAL_BYTES).ok_or_else(invalid_zip)?;
    if !header.starts_with(b"PK\x01\x02") || u16_at(header, 34) != 0 {
        return Err(invalid_zip());
    }
    let flags = u16_at(header, 8);
    if flags & !0x080e != 0 {
        return Err(error("The ZIP is encrypted or uses unsupported entry options. Create a ZIP without encryption."));
    }
    let compression = u16_at(header, 10);
    method(compression)?;
    let name_len = usize::from(u16_at(header, 28));
    let extra_len = usize::from(u16_at(header, 30));
    let comment_len = usize::from(u16_at(header, 32));
    let end_name = CENTRAL_BYTES + name_len;
    let end_extra = end_name + extra_len;
    let end_member = end_extra + comment_len;
    if name_len == 0 || name_len > MAX_NAME_BYTES || end_member > bytes.len() {
        return Err(invalid_zip());
    }
    validate_extra(&bytes[end_name..end_extra])?;
    let name = std::str::from_utf8(&bytes[CENTRAL_BYTES..end_name])
        .map_err(|_| error("The ZIP contains a file name that is not valid UTF-8."))?;
    let directory = name.ends_with('/');
    let relative = relative_path(name.strip_suffix('/').unwrap_or(name))?;
    let attributes = u32_at(header, 38);
    let kind = attributes >> 16 & 0xf000;
    let expected_kind = if directory { 0x4000 } else { 0x8000 };
    if (kind != 0 && kind != expected_kind)
        || attributes & 0x400 != 0
        || (!directory && attributes & 0x10 != 0)
    {
        return Err(error("The ZIP contains a link, special entry, or inconsistent directory. Create a ZIP with ordinary app files."));
    }
    let packed = u64::from(u32_at(header, 20));
    let size = u64::from(u32_at(header, 24));
    let crc = u32_at(header, 16);
    if (compression == 0 && packed != size) || (directory && (size != 0 || crc != 0)) {
        return Err(invalid_zip());
    }
    let member = Member {
        name: name.to_owned(),
        relative,
        directory,
        flags,
        method: compression,
        crc,
        packed,
        size,
        local_start: u64::from(u32_at(header, 42)),
        data_start: 0,
    };
    Ok((member, &bytes[end_member..]))
}

fn relative_path(name: &str) -> Result<PathBuf, ScanError> {
    let mut relative = PathBuf::new();
    for (depth, part) in name.split('/').enumerate() {
        if depth >= MAX_DEPTH
            || part.encode_utf16().count() > 255
            || !crate::path_safety::safe_name(part)
        {
            return Err(error("The ZIP contains an unsafe or unsupported file path. Use plain, relative Windows file names."));
        }
        relative.push(part);
    }
    Ok(relative)
}

fn insert_path(nodes: &mut BTreeMap<String, Node>, member: &Member) -> Result<(), ScanError> {
    let mut key = String::new();
    let mut relative = PathBuf::new();
    let mut parts = member.relative.iter().peekable();
    while let Some(part) = parts.next() {
        if !key.is_empty() {
            key.push('/');
        }
        key.push_str(&part.to_str().expect("validated UTF-8").to_uppercase());
        relative.push(part);
        let final_part = parts.peek().is_none();
        let directory = !final_part || member.directory;
        if let Some(node) = nodes.get_mut(&key) {
            if node.relative != relative
                || node.directory != directory
                || (final_part && node.declared)
            {
                return Err(error(
                    "The ZIP contains duplicate paths or conflicting file and folder names.",
                ));
            }
            node.declared |= final_part;
        } else {
            if nodes.len() >= MAX_ENTRIES {
                return Err(error("The ZIP exceeds the 20,000 file and folder limit."));
            }
            nodes.insert(
                key.clone(),
                Node {
                    relative: relative.clone(),
                    directory,
                    declared: final_part,
                },
            );
        }
    }
    Ok(())
}

fn check_local(
    file: &mut File,
    mut member: Member,
    central_start: u64,
) -> Result<(Member, u64), ScanError> {
    if member.local_start + LOCAL_BYTES as u64 > central_start {
        return Err(invalid_zip());
    }
    file.seek(SeekFrom::Start(member.local_start))
        .map_err(io_error)?;
    let mut header = [0; LOCAL_BYTES];
    file.read_exact(&mut header).map_err(io_error)?;
    let name_len = usize::from(u16_at(&header, 26));
    let extra_len = usize::from(u16_at(&header, 28));
    member.data_start = member.local_start + (LOCAL_BYTES + name_len + extra_len) as u64;
    let mut end = member.data_start + member.packed;
    if !header.starts_with(b"PK\x03\x04")
        || member.flags != u16_at(&header, 6)
        || member.method != u16_at(&header, 8)
        || name_len != member.name.len()
        || end > central_start
    {
        return Err(invalid_zip());
    }
    let mut name_and_extra = vec![0; name_len + extra_len];
    file.read_exact(&mut name_and_extra).map_err(io_error)?;
    if name_and_extra[..name_len] != *member.name.as_bytes() {
        return Err(invalid_zip());
    }
    validate_extra(&name_and_extra[name_len..])?;
    let local_values = [
        u32_at(&header, 14),
        u32_at(&header, 18),
        u32_at(&header, 22),
    ];
    let central_values = [member.crc, member.packed as u32, member.size as u32];
    if member.flags & DATA_DESCRIPTOR == 0 {
        if local_values != central_values {
            return Err(invalid_zip());
        }
    } else {
        if local_values
            .iter()
            .zip(central_values)
            .any(|(&value, expected)| value != 0 && value != expected)
            || end + 12 > central_start
        {
            return Err(invalid_zip());
        }
        file.seek(SeekFrom::Start(end)).map_err(io_error)?;
        let mut descriptor = [0; 16];
        file.read_exact(&mut descriptor[..12]).map_err(io_error)?;
        let offset = if descriptor.starts_with(b"PK\x07\x08") {
            if end + 16 > central_start {
                return Err(invalid_zip());
            }
            file.read_exact(&mut descriptor[12..]).map_err(io_error)?;
            4
        } else {
            0
        };
        let descriptor_values = [
            u32_at(&descriptor, offset),
            u32_at(&descriptor, offset + 4),
            u32_at(&descriptor, offset + 8),
        ];
        if descriptor_values != central_values {
            return Err(invalid_zip());
        }
        end += (offset + 12) as u64;
    }
    Ok((member, end))
}

fn validate_extra(mut bytes: &[u8]) -> Result<(), ScanError> {
    while !bytes.is_empty() {
        if bytes.len() < 4 {
            return Err(invalid_zip());
        }
        if u16_at(bytes, 0) == 1 {
            return Err(error(
                "ZIP64 archives are unsupported. Create a standard ZIP under 4 GiB.",
            ));
        }
        bytes = bytes
            .get(4 + usize::from(u16_at(bytes, 2))..)
            .ok_or_else(invalid_zip)?;
    }
    Ok(())
}

fn method(value: u16) -> Result<CompressionMethod, ScanError> {
    match value {
        0 => Ok(CompressionMethod::Stored),
        8 => Ok(CompressionMethod::Deflated),
        _ => Err(error(
            "The ZIP uses unsupported compression. Create a Stored or Deflate ZIP.",
        )),
    }
}

fn u16_at(bytes: &[u8], start: usize) -> u16 {
    u16::from_le_bytes([bytes[start], bytes[start + 1]])
}

fn u32_at(bytes: &[u8], start: usize) -> u32 {
    u32::from_le_bytes(
        bytes[start..start + 4]
            .try_into()
            .expect("fixed record field"),
    )
}

fn invalid_zip() -> ScanError {
    error("The ZIP is damaged or has inconsistent records. Recreate the archive and try again.")
}

fn zip_error(cause: zip::result::ZipError) -> ScanError {
    match cause {
        zip::result::ZipError::Io(cause) if cause.kind() != io::ErrorKind::InvalidData => {
            io_error(cause)
        }
        _ => invalid_zip(),
    }
}
