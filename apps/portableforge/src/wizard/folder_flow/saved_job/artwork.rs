//! A saved job with custom artwork is one atomic file containing JSON and
//! up to two lossless RGBA images. Jobs without artwork keep the JSON format.

use std::io::{self, Read, Write};

use portableforge_core::artwork::{
    Artwork, IconArtwork, Raster, SidebarArtwork, Transform, MAX_DIMENSION,
};
use serde::{Deserialize, Serialize};

use super::{SavedJob, MAX_METADATA_BYTES};

pub(super) const MAGIC: [u8; 8] = *b"PFART1\r\n";
const LENGTHS_BYTES: u64 = 12;
const RASTER_HEADER_BYTES: u32 = 8;
const MAX_RASTER_BYTES: u32 = RASTER_HEADER_BYTES + MAX_DIMENSION * MAX_DIMENSION * 4;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Metadata<Job> {
    job: Job,
    icon: Option<Transform>,
    sidebar: Option<Sidebar>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Sidebar {
    transform: Transform,
    background: [u8; 3],
    imported: bool,
}

pub(super) struct Contents<'a> {
    metadata: Vec<u8>,
    icon: Option<&'a Raster>,
    pane: Option<&'a Raster>,
    envelope: bool,
}

impl<'a> Contents<'a> {
    pub(super) fn new(job: &'a SavedJob) -> io::Result<Self> {
        let (metadata, icon, pane) = if let Some(artwork) = &job.artwork {
            let icon = artwork.icon.as_ref();
            let sidebar = artwork.sidebar.as_ref();
            if (icon.is_none() && sidebar.is_none())
                || icon.is_some_and(|icon| icon.transform.quarter_turns > 3)
                || sidebar.is_some_and(|pane| pane.transform.quarter_turns > 3)
            {
                return Err(invalid_artwork());
            }
            let metadata = Metadata {
                job,
                icon: icon.map(|icon| icon.transform),
                sidebar: sidebar.map(|pane| Sidebar {
                    transform: pane.transform,
                    background: pane.background,
                    imported: pane.source.is_some(),
                }),
            };
            (
                serde_json::to_vec(&metadata)?,
                icon.map(|icon| &icon.source),
                sidebar.and_then(|pane| pane.source.as_ref()),
            )
        } else {
            (serde_json::to_vec(job)?, None, None)
        };
        if metadata.len() as u64 > MAX_METADATA_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "The saved job is too large.",
            ));
        }
        Ok(Self {
            metadata,
            icon,
            pane,
            envelope: job.artwork.is_some(),
        })
    }

    pub(super) fn write(self, mut writer: impl Write) -> io::Result<()> {
        if !self.envelope {
            return writer.write_all(&self.metadata);
        }
        writer.write_all(&MAGIC)?;
        for len in [
            self.metadata.len() as u32,
            raster_len(self.icon),
            raster_len(self.pane),
        ] {
            writer.write_all(&len.to_le_bytes())?;
        }
        writer.write_all(&self.metadata)?;
        for raster in [self.icon, self.pane].into_iter().flatten() {
            writer.write_all(&raster.width().to_le_bytes())?;
            writer.write_all(&raster.height().to_le_bytes())?;
            writer.write_all(raster.rgba())?;
        }
        Ok(())
    }
}

pub(super) fn read(mut reader: impl Read, file_len: u64) -> io::Result<SavedJob> {
    let metadata_len = read_u32(&mut reader)?;
    let icon_len = read_u32(&mut reader)?;
    let pane_len = read_u32(&mut reader)?;
    let expected_len = MAGIC.len() as u64
        + LENGTHS_BYTES
        + u64::from(metadata_len)
        + u64::from(icon_len)
        + u64::from(pane_len);
    if metadata_len == 0
        || u64::from(metadata_len) > MAX_METADATA_BYTES
        || icon_len > MAX_RASTER_BYTES
        || pane_len > MAX_RASTER_BYTES
        || expected_len != file_len
    {
        return Err(invalid_artwork());
    }

    let mut json = vec![0; metadata_len as usize];
    reader.read_exact(&mut json)?;
    let Metadata {
        mut job,
        icon,
        sidebar,
    } = serde_json::from_slice::<Metadata<SavedJob>>(&json)?;
    job.validate()?;

    if job.selected.is_none()
        || (icon.is_none() && sidebar.is_none())
        || icon.is_some() != (icon_len != 0)
        || sidebar.as_ref().is_some_and(|pane| pane.imported) != (pane_len != 0)
        || icon.is_some_and(|transform| transform.quarter_turns > 3)
        || sidebar
            .as_ref()
            .is_some_and(|pane| pane.transform.quarter_turns > 3)
    {
        return Err(invalid_artwork());
    }

    let icon = icon
        .map(|transform| {
            let source = read_raster(&mut reader, icon_len)?;
            Ok::<_, io::Error>(IconArtwork { source, transform })
        })
        .transpose()?;
    let sidebar = sidebar
        .map(|pane| {
            let source = pane
                .imported
                .then(|| read_raster(&mut reader, pane_len))
                .transpose()?;
            Ok::<_, io::Error>(SidebarArtwork {
                source,
                transform: pane.transform,
                background: pane.background,
            })
        })
        .transpose()?;
    if reader.read(&mut [0; 1])? != 0 {
        return Err(invalid_artwork());
    }
    job.artwork = Some(Artwork { icon, sidebar });
    Ok(job)
}

fn raster_len(raster: Option<&Raster>) -> u32 {
    raster.map_or(0, |raster| RASTER_HEADER_BYTES + raster.rgba().len() as u32)
}

fn read_raster(reader: &mut impl Read, len: u32) -> io::Result<Raster> {
    if len < RASTER_HEADER_BYTES {
        return Err(invalid_artwork());
    }
    let width = read_u32(reader)?;
    let height = read_u32(reader)?;
    if !(1..=MAX_DIMENSION).contains(&width)
        || !(1..=MAX_DIMENSION).contains(&height)
        || width * height * 4 != len - RASTER_HEADER_BYTES
    {
        return Err(invalid_artwork());
    }
    let mut pixels = vec![0; (len - RASTER_HEADER_BYTES) as usize];
    reader.read_exact(&mut pixels)?;
    Raster::from_rgba(width, height, pixels).map_err(io::Error::other)
}

fn read_u32(reader: &mut impl Read) -> io::Result<u32> {
    let mut bytes = [0; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn invalid_artwork() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "The saved artwork is incomplete or invalid.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wizard::folder_flow::{saved_job::SourceKind, Flow};

    #[test]
    fn artwork_envelope_rejects_invalid_sections_and_layout() {
        let mut job = SavedJob::from_flow(&Flow::default(), SourceKind::Zip);
        job.inputs = vec![std::env::temp_dir().join("Widget.zip")];
        job.selected = Some("Widget.exe".into());
        job.artwork = Some(Artwork {
            icon: Some(IconArtwork {
                source: Raster::from_rgba(1, 1, vec![11, 22, 33, 255]).unwrap(),
                transform: Transform::default(),
            }),
            sidebar: Some(SidebarArtwork {
                source: None,
                transform: Transform::default(),
                background: [1, 2, 3],
            }),
        });
        let mut encoded = Vec::new();
        Contents::new(&job).unwrap().write(&mut encoded).unwrap();
        let decode = |bytes: &[u8]| read(&bytes[MAGIC.len()..], bytes.len() as u64);
        assert!(decode(&encoded).is_ok());
        assert!(decode(&encoded[..encoded.len() - 1]).is_err());
        let mut trailing = encoded.clone();
        trailing.push(0);
        assert!(decode(&trailing).is_err());

        let mut oversize = encoded.clone();
        oversize[12..16].copy_from_slice(&(MAX_RASTER_BYTES + 1).to_le_bytes());
        assert!(decode(&oversize).is_err());
        let mut invalid_dimensions = encoded.clone();
        let icon_at = encoded.len() - 12;
        invalid_dimensions[icon_at..icon_at + 4]
            .copy_from_slice(&(MAX_DIMENSION + 1).to_le_bytes());
        assert!(decode(&invalid_dimensions).is_err());
        let mut mismatched_pixels = encoded.clone();
        mismatched_pixels[icon_at..icon_at + 4].copy_from_slice(&2u32.to_le_bytes());
        assert!(decode(&mismatched_pixels).is_err());

        let mut missing_icon = encoded[..icon_at].to_vec();
        missing_icon[12..16].copy_from_slice(&0u32.to_le_bytes());
        assert!(decode(&missing_icon).is_err());
    }
}
