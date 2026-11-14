//! Bounded static analysis of PE import descriptors in inventoried files.

use std::collections::BTreeMap;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use super::{
    error, io_error, open_inventory_file, safe_name, EntryKind, FolderEntry, FolderScan, ScanError,
    MAX_PE_OFFSET,
};
use crate::snapshot::{pe_arch, PeArch};

const MAX_SECTIONS: usize = 96;
const MAX_IMPORTS: usize = 512;
const MAX_NAME_BYTES: usize = 256;
const INVALID_PE: &str = "The Windows file has an incomplete or invalid import table.";

/// Evidence from the selected executable's direct import tables.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DependencyReport {
    pub executable: PathBuf,
    pub architecture: PeArch,
    pub imports: Vec<Dependency>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Dependency {
    pub name: String,
    pub delayed: bool,
    pub status: DependencyStatus,
}

/// Package inventory evidence. These states do not predict Windows loader resolution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DependencyStatus {
    /// One same-named PE DLL beside the selected program has the same machine type.
    Bundled(PathBuf),
    /// A same-named file appears elsewhere in the package.
    OtherFolder,
    /// A DLL beside the selected program has a different PE machine type.
    WrongArchitecture,
    /// More than one file beside the program matches the name case-insensitively.
    Ambiguous,
    /// The name has a Windows API-set contract prefix. OS availability was not checked.
    ApiSet,
    /// No valid matching local DLL was identified. It could be supplied by Windows or a runtime.
    Unresolved,
}

impl FolderScan {
    /// Read direct and delay imports from a scanned executable without loading any image.
    ///
    /// Resolution is limited to the scanned files. Transitive and dynamic imports,
    /// the system environment, and Windows loader behavior require other evidence.
    pub fn dependencies_for(&self, relative: &Path) -> Result<DependencyReport, ScanError> {
        let candidate = self
            .candidates
            .iter()
            .find(|candidate| candidate.relative_path == relative)
            .ok_or_else(|| error("Choose an executable identified by the folder scan."))?;
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.kind == EntryKind::File && entry.relative_path == relative)
            .ok_or_else(|| error("The executable is missing from the scanned file list."))?;
        let (_guard, mut file) = open_inventory_file(&self.root, relative)?;
        let size = file.metadata().map_err(io_error)?.len();
        if size != entry.size_bytes {
            return Err(error("The executable changed after the scan."));
        }
        let image = Image::read(&mut file, size)?;
        if image.header.dll || image.header.architecture != candidate.architecture {
            return Err(error("The executable changed after the scan."));
        }
        let names = image.imports(&mut file)?;
        let mut locations = BTreeMap::<String, Vec<&FolderEntry>>::new();
        for entry in self
            .entries
            .iter()
            .filter(|entry| entry.kind == EntryKind::File)
        {
            if let Some(name) = entry
                .relative_path
                .file_name()
                .and_then(|name| name.to_str())
            {
                let key = name.to_ascii_lowercase();
                if names.contains_key(&key) {
                    locations.entry(key).or_default().push(entry);
                }
            }
        }
        let parent = relative.parent().unwrap_or(Path::new(""));
        let imports = names
            .into_iter()
            .map(|(key, (name, delayed))| Dependency {
                status: resolve(
                    &self.root,
                    parent,
                    &key,
                    image.header.architecture,
                    locations.get(&key).map_or(&[], Vec::as_slice),
                ),
                name,
                delayed,
            })
            .collect();
        Ok(DependencyReport {
            executable: relative.to_path_buf(),
            architecture: image.header.architecture,
            imports,
        })
    }
}

fn resolve(
    root: &Path,
    parent: &Path,
    name: &str,
    architecture: PeArch,
    entries: &[&FolderEntry],
) -> DependencyStatus {
    if name.starts_with("api-ms-win-") || name.starts_with("ext-ms-win-") {
        return DependencyStatus::ApiSet;
    }
    let mut adjacent = entries.iter().filter(|entry| {
        entry
            .relative_path
            .parent()
            .is_some_and(|path| super::validation::same_path(path, parent))
    });
    let Some(entry) = adjacent.next() else {
        return if entries.is_empty() {
            DependencyStatus::Unresolved
        } else {
            DependencyStatus::OtherFolder
        };
    };
    if adjacent.next().is_some() {
        return DependencyStatus::Ambiguous;
    }
    let header = (|| {
        let (_guard, mut file) = open_inventory_file(root, &entry.relative_path).ok()?;
        let size = file.metadata().ok()?.len();
        if size != entry.size_bytes {
            return None;
        }
        Header::read(&mut file, size).ok()
    })();
    match header {
        Some(header) if header.dll && header.architecture == architecture => {
            DependencyStatus::Bundled(entry.relative_path.clone())
        }
        Some(header) if header.dll => DependencyStatus::WrongArchitecture,
        _ => DependencyStatus::Unresolved,
    }
}

struct Header {
    architecture: PeArch,
    dll: bool,
    optional_offset: u64,
    optional_size: usize,
    section_count: usize,
}

impl Header {
    fn read(reader: &mut (impl Read + Seek), file_size: u64) -> Result<Self, ScanError> {
        let mut prefix = [0; 88];
        read_at(reader, file_size, 0, &mut prefix[..64])?;
        // Copy the COFF header into a small contiguous window for the shared machine mapping.
        let pe_offset = u64::from(dword(&prefix, 60));
        if !(64..=MAX_PE_OFFSET).contains(&pe_offset) {
            return Err(error(INVALID_PE));
        }
        read_at(reader, file_size, pe_offset, &mut prefix[64..])?;
        prefix[60..64].copy_from_slice(&64u32.to_le_bytes());
        let architecture = pe_arch(&prefix).ok_or_else(|| error(INVALID_PE))?;
        let flags = word(&prefix, 86);
        let section_count = usize::from(word(&prefix, 70));
        let optional_size = usize::from(word(&prefix, 84));
        let optional_offset = pe_offset + 24;
        if flags & 0x0002 == 0
            || !(1..=MAX_SECTIONS).contains(&section_count)
            || !(96..=4096).contains(&optional_size)
            || optional_offset + optional_size as u64 + 40 * section_count as u64 > file_size
        {
            return Err(error(INVALID_PE));
        }
        let mut magic = [0; 2];
        read_at(reader, file_size, optional_offset, &mut magic)?;
        match (architecture, word(&magic, 0)) {
            (PeArch::X86 | PeArch::Other(_), 0x10b) => {}
            (PeArch::X64 | PeArch::Arm64 | PeArch::Other(_), 0x20b) if optional_size >= 112 => {}
            _ => return Err(error(INVALID_PE)),
        }
        Ok(Self {
            architecture,
            dll: flags & 0x2000 != 0,
            optional_offset,
            optional_size,
            section_count,
        })
    }
}

struct Section {
    rva: u32,
    raw_size: u32,
    raw_offset: u32,
}

struct Image {
    header: Header,
    file_size: u64,
    headers_size: u32,
    image_base: u64,
    sections: Vec<Section>,
    directories: [(u32, u32); 2],
}

impl Image {
    fn read(reader: &mut (impl Read + Seek), file_size: u64) -> Result<Self, ScanError> {
        let header = Header::read(reader, file_size)?;
        let mut optional = vec![0; header.optional_size];
        read_at(reader, file_size, header.optional_offset, &mut optional)?;
        let (image_base, table) = if word(&optional, 0) == 0x10b {
            (u64::from(dword(&optional, 28)), 96)
        } else {
            (
                u64::from_le_bytes(optional[24..32].try_into().expect("PE32+ image base")),
                112,
            )
        };
        let count = dword(&optional, table - 4) as usize;
        if count > (optional.len() - table) / 8 {
            return Err(error(INVALID_PE));
        }
        let directories = [1, 13].map(|index| {
            if index >= count {
                (0, 0)
            } else {
                let offset = table + 8 * index;
                (dword(&optional, offset), dword(&optional, offset + 4))
            }
        });
        let headers_size = dword(&optional, 60);
        let table_offset = header.optional_offset + header.optional_size as u64;
        let mut sections = Vec::with_capacity(header.section_count);
        for index in 0..header.section_count {
            let mut section = [0; 40];
            read_at(
                reader,
                file_size,
                table_offset + 40 * index as u64,
                &mut section,
            )?;
            sections.push(Section {
                rva: dword(&section, 12),
                raw_size: dword(&section, 16),
                raw_offset: dword(&section, 20),
            });
        }
        Ok(Self {
            header,
            file_size,
            headers_size,
            image_base,
            sections,
            directories,
        })
    }

    fn raw_span(&self, rva: u32) -> Result<(u64, u64), ScanError> {
        let headers =
            (rva < self.headers_size).then(|| (u64::from(rva), u64::from(self.headers_size - rva)));
        let sections = self.sections.iter().filter_map(|section| {
            let delta = rva.checked_sub(section.rva)?;
            (delta < section.raw_size).then(|| {
                (
                    u64::from(section.raw_offset) + u64::from(delta),
                    u64::from(section.raw_size - delta),
                )
            })
        });
        let mut matches = headers.into_iter().chain(sections);
        let (offset, remaining) = matches.next().ok_or_else(|| error(INVALID_PE))?;
        if matches.next().is_some() || offset >= self.file_size {
            return Err(error(INVALID_PE));
        }
        Ok((offset, remaining.min(self.file_size - offset)))
    }

    fn imports(
        &self,
        reader: &mut (impl Read + Seek),
    ) -> Result<BTreeMap<String, (String, bool)>, ScanError> {
        let mut imports = BTreeMap::new();
        for ((rva, size), delayed) in self.directories.into_iter().zip([false, true]) {
            if (rva, size) == (0, 0) {
                continue;
            }
            let stride = if delayed { 32 } else { 20 };
            let (offset, available) = self.raw_span(rva)?;
            if rva == 0 || u64::from(size) > available {
                return Err(error(INVALID_PE));
            }
            let count = size as usize / stride;
            let mut terminated = false;
            for index in 0..count.min(MAX_IMPORTS + 1) {
                let mut descriptor = [0; 32];
                let bytes = &mut descriptor[..stride];
                read_at(
                    reader,
                    self.file_size,
                    offset + (index * stride) as u64,
                    bytes,
                )?;
                if bytes.iter().all(|byte| *byte == 0) {
                    terminated = true;
                    break;
                }
                let name_rva = if delayed {
                    self.delay_name_rva(bytes)?
                } else {
                    dword(bytes, 12)
                };
                let name = self.import_name(reader, name_rva)?;
                // A regular import wins when the same name appears in both tables.
                imports
                    .entry(name.to_ascii_lowercase())
                    .or_insert((name, delayed));
            }
            if !terminated {
                return Err(error(if count > MAX_IMPORTS {
                    "The import table exceeds the dependency scan limit or is incomplete."
                } else {
                    INVALID_PE
                }));
            }
        }
        if imports.len() > MAX_IMPORTS {
            return Err(error("The executable exceeds the dependency scan limit."));
        }
        Ok(imports)
    }

    fn delay_name_rva(&self, bytes: &[u8]) -> Result<u32, ScanError> {
        let address = dword(bytes, 4);
        match dword(bytes, 0) {
            1 => Ok(address),
            0 => u64::from(address)
                .checked_sub(self.image_base)
                .and_then(|rva| u32::try_from(rva).ok())
                .ok_or_else(|| error(INVALID_PE)),
            _ => Err(error(INVALID_PE)),
        }
    }

    fn import_name(&self, reader: &mut (impl Read + Seek), rva: u32) -> Result<String, ScanError> {
        if rva == 0 {
            return Err(error(INVALID_PE));
        }
        let (offset, available) = self.raw_span(rva)?;
        let mut bytes = [0; MAX_NAME_BYTES];
        let length = (available as usize).min(MAX_NAME_BYTES);
        read_at(reader, self.file_size, offset, &mut bytes[..length])?;
        let length = bytes[..length]
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(|| error(INVALID_PE))?;
        let name = std::str::from_utf8(&bytes[..length]).map_err(|_| error(INVALID_PE))?;
        if !name.is_ascii() || !safe_name(name) {
            return Err(error(INVALID_PE));
        }
        Ok(name.to_owned())
    }
}

fn read_at(
    reader: &mut (impl Read + Seek),
    file_size: u64,
    offset: u64,
    bytes: &mut [u8],
) -> Result<(), ScanError> {
    if offset
        .checked_add(bytes.len() as u64)
        .is_none_or(|end| end > file_size)
    {
        return Err(error(INVALID_PE));
    }
    reader
        .seek(SeekFrom::Start(offset))
        .and_then(|_| reader.read_exact(bytes))
        .map_err(|cause| {
            if cause.kind() == io::ErrorKind::UnexpectedEof {
                error(INVALID_PE)
            } else {
                io_error(cause)
            }
        })
}

fn word(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(
        bytes[offset..offset + 2]
            .try_into()
            .expect("bounded PE field"),
    )
}

fn dword(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("bounded PE field"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Cursor;
    use std::time::{SystemTime, UNIX_EPOCH};

    const OPTIONAL: usize = 0x98;
    const RAW: usize = 0x200;
    const RVA: u32 = 0x1000;

    struct FixtureFolder(PathBuf);

    impl FixtureFolder {
        fn new() -> Self {
            let time = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root =
                std::env::temp_dir().join(format!("pf-dll-scan-{}-{time}", std::process::id()));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
    }

    impl Drop for FixtureFolder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn put_word(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put_dword(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn fixture(x86: bool, dll: bool) -> Vec<u8> {
        let mut bytes = vec![0; 0x800];
        bytes[..2].copy_from_slice(b"MZ");
        put_dword(&mut bytes, 60, 0x80);
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        put_word(&mut bytes, 0x84, if x86 { 0x14c } else { 0x8664 });
        put_word(&mut bytes, 0x86, 1);
        let optional_size = if x86 { 224 } else { 240 };
        put_word(&mut bytes, 0x94, optional_size);
        put_word(&mut bytes, 0x96, if dll { 0x2002 } else { 0x0002 });
        put_word(&mut bytes, OPTIONAL, if x86 { 0x10b } else { 0x20b });
        put_dword(&mut bytes, OPTIONAL + if x86 { 28 } else { 24 }, 0x400000);
        put_dword(&mut bytes, OPTIONAL + 60, RAW as u32);
        put_word(&mut bytes, OPTIONAL + 68, 3);
        put_dword(&mut bytes, OPTIONAL + if x86 { 92 } else { 108 }, 16);
        let section = OPTIONAL + usize::from(optional_size);
        put_dword(&mut bytes, section + 8, 0x600);
        put_dword(&mut bytes, section + 12, RVA);
        put_dword(&mut bytes, section + 16, 0x600);
        put_dword(&mut bytes, section + 20, RAW as u32);
        bytes
    }

    fn add_imports(bytes: &mut [u8], names: &[&str]) {
        put_dword(bytes, OPTIONAL + 112 + 8, RVA);
        put_dword(bytes, OPTIONAL + 112 + 12, 20 * (names.len() as u32 + 1));
        for (index, name) in names.iter().enumerate() {
            let offset = 0x200 + index * 0x40;
            put_dword(bytes, RAW + index * 20 + 12, RVA + offset as u32);
            bytes[RAW + offset..RAW + offset + name.len()].copy_from_slice(name.as_bytes());
        }
    }

    fn add_delay(bytes: &mut [u8], x86: bool, uses_rva: bool) {
        let table = OPTIONAL + if x86 { 96 } else { 112 };
        put_dword(bytes, table + 13 * 8, RVA + 0x100);
        put_dword(bytes, table + 13 * 8 + 4, 64);
        put_dword(bytes, RAW + 0x100, u32::from(uses_rva));
        let address = RVA + 0x300 + if uses_rva { 0 } else { 0x400000 };
        put_dword(bytes, RAW + 0x104, address);
        bytes[RAW + 0x300..RAW + 0x309].copy_from_slice(b"Delay.dll");
    }

    fn read_imports(bytes: &[u8]) -> Result<BTreeMap<String, (String, bool)>, ScanError> {
        let mut reader = Cursor::new(bytes);
        let image = Image::read(&mut reader, bytes.len() as u64)?;
        image.imports(&mut reader)
    }

    #[cfg(windows)]
    #[test]
    fn reads_the_import_table_of_a_real_windows_image() {
        let path = std::env::current_exe().unwrap();
        let mut reader = fs::File::open(path).unwrap();
        let size = reader.metadata().unwrap().len();
        let image = Image::read(&mut reader, size).unwrap();
        let names = image.imports(&mut reader).unwrap();
        assert!(names.values().any(|(_, delayed)| !delayed));
    }

    #[test]
    fn reads_regular_and_delayed_imports_without_duplicates() {
        let mut bytes = fixture(false, false);
        add_imports(&mut bytes, &["One.dll", "ONE.DLL", "Other.dll"]);
        add_delay(&mut bytes, false, true);
        let names = read_imports(&bytes).unwrap();
        assert_eq!(
            names.into_values().collect::<Vec<_>>(),
            [
                ("Delay.dll".into(), true),
                ("One.dll".into(), false),
                ("Other.dll".into(), false)
            ]
        );
        let mut x86 = fixture(true, false);
        add_delay(&mut x86, true, false);
        assert_eq!(
            read_imports(&x86).unwrap()["delay.dll"],
            ("Delay.dll".into(), true)
        );
    }

    #[test]
    fn rejects_truncation_invalid_rvas_descriptors_and_module_paths() {
        let mut bytes = fixture(false, false);
        add_imports(&mut bytes, &["One.dll"]);
        assert!(read_imports(&bytes[..0x3ff]).is_err());
        let mut outside_section = bytes.clone();
        put_dword(&mut outside_section, RAW + 12, RVA + 0x600);
        assert!(read_imports(&outside_section).is_err());
        let mut missing_terminator = bytes.clone();
        put_dword(&mut missing_terminator, OPTIONAL + 112 + 12, 20);
        assert!(read_imports(&missing_terminator).is_err());
        let mut outside_name = bytes;
        add_imports(&mut outside_name, &["..\\One.dll"]);
        assert!(read_imports(&outside_name).is_err());
    }

    #[test]
    fn resolves_only_same_folder_matching_pe_machine_and_distinguishes_api_sets() {
        let root = FixtureFolder::new();
        let app = root.0.join("Widget");
        fs::create_dir_all(app.join("plugins")).unwrap();
        let mut exe = fixture(false, false);
        add_imports(
            &mut exe,
            &[
                "same.dll",
                "wrong.dll",
                "elsewhere.dll",
                "missing.dll",
                "api-ms-win-core-file-l1-1-0.dll",
            ],
        );
        fs::write(app.join("Widget.exe"), exe).unwrap();
        fs::write(app.join("Same.DLL"), fixture(false, true)).unwrap();
        fs::write(app.join("Wrong.dll"), fixture(true, true)).unwrap();
        fs::write(app.join("plugins/Elsewhere.dll"), fixture(false, true)).unwrap();
        let scan = super::super::scan(&app).unwrap();
        let report = scan.dependencies_for(Path::new("Widget.exe")).unwrap();
        let statuses = report
            .imports
            .into_iter()
            .map(|item| (item.name, item.status))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            statuses["same.dll"],
            DependencyStatus::Bundled("Same.DLL".into())
        );
        assert_eq!(statuses["wrong.dll"], DependencyStatus::WrongArchitecture);
        assert_eq!(statuses["elsewhere.dll"], DependencyStatus::OtherFolder);
        assert_eq!(statuses["missing.dll"], DependencyStatus::Unresolved);
        assert_eq!(
            statuses["api-ms-win-core-file-l1-1-0.dll"],
            DependencyStatus::ApiSet
        );
        assert!(scan.dependencies_for(Path::new("../Widget.exe")).is_err());
    }
}
