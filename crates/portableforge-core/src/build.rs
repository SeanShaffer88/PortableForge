//! The build pipeline: turn package details, the accepted review
//! fixes, and the harvested program files into a finished
//! PortableApps.com Format 3.9 package tree, then validate it.
//!
//! This is the step the wizard's "We build it for you" page and the
//! CLI `build` command both run. Before this module existed the pieces
//! were only connected by hand: `generate_tree` was called with an
//! empty fix list (dropping every fix the user accepted in review),
//! harvested files were copied by nobody, and the finished tree was
//! never checked as a whole.

use std::path::Path;

use crate::classify::Classification;
use crate::orchestrate::harvest_tree;
use crate::paf::PafMetadata;
use crate::validate::{
    generate_tree, has_errors, validate_metadata, validate_tree, ValidationIssue,
};

/// License-file names recognised at the top of the program's files,
/// in the order they win when several are present.
const LICENSE_FILE_NAMES: &[&str] = &[
    "LICENSE.txt",
    "LICENSE",
    "LICENSE.md",
    "LICENCE.txt",
    "LICENCE",
    "COPYING.txt",
    "COPYING",
    "NOTICE.txt",
    "NOTICE",
    "MIT-LICENSE.txt",
];

/// Human byte sizes in decimal units, like the wizard's pick step.
pub fn format_bytes(n: u64) -> String {
    if n < 1_000 {
        format!("{n} bytes")
    } else if n < 1_000_000 {
        format!("{:.1} KB", n as f64 / 1_000.0)
    } else if n < 1_000_000_000 {
        format!("{:.1} MB", n as f64 / 1_000_000.0)
    } else {
        format!("{:.2} GB", n as f64 / 1_000_000_000.0)
    }
}

/// Every file under `root` except the package's `Data\` folder (the
/// user's live data, not build output), sorted for stable output.
fn package_files(root: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for item in entries.flatten() {
            let path = item.path();
            let Ok(kind) = item.file_type() else { continue };
            if kind.is_dir() {
                if path != root.join("Data") {
                    stack.push(path);
                }
            } else if kind.is_file() {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// Count files under a folder, recursively (0 when it isn't there).
fn count_files(dir: &Path) -> usize {
    let mut n = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for item in entries.flatten() {
            let path = item.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() {
                n += 1;
            }
        }
    }
    n
}

#[derive(Debug, Clone, PartialEq)]
pub struct BuildOutcome {
    /// Text files PortableForge authored, as package-relative paths.
    pub written: Vec<String>,
    /// Program files copied into `App\<BaseAppName>\`.
    pub app_files_copied: usize,
    pub app_bytes_copied: u64,
    /// First-run files copied into `App\DefaultData\`.
    pub default_data_files_copied: usize,
    /// Validation of the finished tree (errors mean it is not usable).
    pub issues: Vec<ValidationIssue>,
    /// Harvest entries that were skipped (unsafe paths, unreadable).
    pub skipped: Vec<String>,
    /// The license file found in the program's files and copied into
    /// `App\AppInfo\`, as a package-relative path, when there was one.
    pub license_file: Option<String>,
    /// Whether a program picture was placed at `App\AppInfo\appicon.ico`.
    pub icon_installed: bool,
    /// Where the picture came from: `"supplied"` or `"harvested"`.
    pub icon_source: Option<String>,
    /// The main file's processor family (`x86`, `x64`, `ARM64`), read
    /// from its own header when the program files were supplied.
    pub main_exe_arch: Option<String>,
    /// Files already living in the package's `Data\` folder when the
    /// build started — a rebuild never touches them.
    pub preserved_data_files: usize,
    /// `(top folder, bytes, files)` biggest first; the package root's
    /// own files are under `(package root)`. `Data\` is user data at
    /// run time, not build output, so it is not counted.
    pub size_breakdown: Vec<(String, u64, usize)>,
}

impl BuildOutcome {
    pub fn has_errors(&self) -> bool {
        crate::validate::has_errors(&self.issues)
    }

    pub fn total_bytes(&self) -> u64 {
        self.size_breakdown.iter().map(|(_, b, _)| b).sum()
    }

    /// One plain summary line for the wizard log and the CLI.
    pub fn summary(&self) -> String {
        if self.has_errors() {
            let n = self.issues.iter().filter(|i| i.error).count();
            format!(
                "The package was made, but {n} problem{} must be fixed before it can be used — see the list.",
                if n == 1 { "" } else { "s" }
            )
        } else {
            let total_files: usize = self.size_breakdown.iter().map(|(_, _, f)| f).sum();
            let mut text = format!(
                "Package made: {} files written, {} program files copied, {} starting-settings files. Everything checks out. Total size: {} across {} files.",
                self.written.len(),
                self.app_files_copied,
                self.default_data_files_copied,
                format_bytes(self.total_bytes()),
                total_files
            );
            if self.preserved_data_files > 0 {
                text.push_str(&format!(
                    " The existing saved-data folder was kept exactly as it was ({} file{}).",
                    self.preserved_data_files,
                    if self.preserved_data_files == 1 {
                        ""
                    } else {
                        "s"
                    }
                ));
            }
            text
        }
    }
}

/// Build a complete package tree at `root`.
///
/// - `accepted` is the review's switched-on fixes — they go into the
///   Launcher and Installer settings (never silently dropped).
/// - `app_files`, when given, is a folder whose contents become
///   `App\<BaseAppName>\` (the harvested install folder).
/// - `default_data`, when given, is a folder structured like `Data\`
///   (e.g. it contains `Widget\settings.ini`) whose contents become
///   `App\DefaultData\`, the first-run seed.
///
/// Bad details refuse the build *before anything is written*, with
/// every problem listed (not just the first); a finished tree that
/// fails validation is reported in the outcome, not hidden.
#[allow(clippy::too_many_arguments)]
pub fn build_package(
    root: &Path,
    meta: &PafMetadata,
    accepted: &[Classification],
    app_files: Option<&Path>,
    default_data: Option<&Path>,
    icon: Option<&Path>,
    force: bool,
) -> Result<BuildOutcome, String> {
    // Pre-flight: the full problem list up front, so the person fixes
    // everything in one pass instead of one error at a time.
    let meta_issues = validate_metadata(meta);
    if has_errors(&meta_issues) {
        let mut msg = String::from("This build cannot start yet — please fix these first:");
        for i in &meta_issues {
            msg.push_str(&format!("\n- {}: {}", i.field, i.message));
        }
        return Err(msg);
    }
    let meta_warnings: Vec<ValidationIssue> =
        meta_issues.into_iter().filter(|i| !i.error).collect();

    // Never build into a folder that already holds something else:
    // mixing two programs into one package folder corrupts both. A
    // folder from an earlier PortableForge build (it has the
    // package's appinfo.ini) is fine — that is a rebuild.
    if !force && root.is_dir() {
        let has_entries = std::fs::read_dir(root)
            .map(|mut d| d.next().is_some())
            .unwrap_or(false);
        let looks_like_package = root
            .join("App")
            .join("AppInfo")
            .join("appinfo.ini")
            .is_file();
        if has_entries && !looks_like_package {
            return Err(format!(
                "The folder {} isn't empty, and it doesn't look like a package made before. \
                 Building there could mix two programs together. Pick an empty folder, \
                 or use the override (the build command's --force) if you really mean to build there.",
                root.display()
            ));
        }
    }

    // Read the main file's own header (when the program files are at
    // hand) so the report can name its processor family, the Launcher
    // settings gain the 64-bit line for an x64-only program, and a
    // "main file" that is not really a program file gets called out.
    let mut meta = meta.clone();
    let mut main_exe_arch: Option<String> = None;
    let mut main_exe_not_a_program = false;
    if let Some(src) = app_files {
        let exe_path = src.join(&meta.main_exe);
        if exe_path.is_file() {
            use std::io::Read;
            let head = std::fs::File::open(&exe_path)
                .and_then(|f| {
                    let mut buf = Vec::new();
                    f.take(65_536).read_to_end(&mut buf).map(|_| buf)
                })
                .unwrap_or_default();
            match crate::snapshot::pe_arch(&head) {
                Some(arch) => {
                    main_exe_arch = Some(arch.label());
                    if arch == crate::snapshot::PeArch::X64 {
                        meta.main_exe_64bit = Some(true);
                    }
                }
                None => main_exe_not_a_program = true,
            }
        }
    }

    // A rebuild over an existing package keeps its Data\ folder: it is
    // the user's live data, never build output. Count it before
    // anything runs so the summary can say it was kept.
    let preserved_data_files = count_files(&root.join("Data"));

    let written = generate_tree(root, &meta, accepted)?;
    let mut outcome = BuildOutcome {
        written,
        app_files_copied: 0,
        app_bytes_copied: 0,
        default_data_files_copied: 0,
        issues: Vec::new(),
        skipped: Vec::new(),
        license_file: None,
        icon_installed: false,
        icon_source: None,
        main_exe_arch,
        preserved_data_files,
        size_breakdown: Vec::new(),
    };
    if let Some(src) = app_files {
        if !src.is_dir() {
            return Err(format!(
                "The program-files folder {} could not be found.",
                src.display()
            ));
        }
        let dest = root.join("App").join(&meta.base_app_name);
        let report = harvest_tree(src, &dest)?;
        outcome.app_files_copied = report.files_copied;
        outcome.app_bytes_copied = report.bytes_copied;
        outcome.skipped.extend(report.skipped);
        // The program's own license travels with the package: detect
        // it at the top of the program's files and keep a copy where
        // the package tools look for it.
        for name in LICENSE_FILE_NAMES {
            let candidate = src.join(name);
            if candidate.is_file() {
                let dest = root.join("App").join("AppInfo").join(name);
                std::fs::copy(&candidate, &dest)
                    .map_err(|e| format!("We could not copy the license file {name}: {e}"))?;
                outcome.license_file = Some(format!(r"App\AppInfo\{name}"));
                break;
            }
        }
    }
    if let Some(src) = default_data {
        if !src.is_dir() {
            return Err(format!(
                "The starting-settings folder {} could not be found.",
                src.display()
            ));
        }
        let dest = root.join("App").join("DefaultData");
        let report = harvest_tree(src, &dest)?;
        outcome.default_data_files_copied = report.files_copied;
        outcome.skipped.extend(report.skipped);
    }
    if let Some(icon_path) = icon {
        if !icon_path.is_file() {
            return Err(format!(
                "The picture file {} could not be found.",
                icon_path.display()
            ));
        }
        let dest = root.join("App").join("AppInfo").join("appicon.ico");
        std::fs::copy(icon_path, &dest)
            .map_err(|e| format!("We could not copy the picture file: {e}"))?;
        outcome.icon_installed = true;
        outcome.icon_source = Some("supplied".to_string());
    }
    // No picture supplied: a harvested .ico from the program's own
    // files becomes the package picture automatically.
    if !outcome.icon_installed {
        if let Some(src) = app_files {
            let mut icons: Vec<std::path::PathBuf> = Vec::new();
            if let Ok(entries) = std::fs::read_dir(src) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file()
                        && path
                            .extension()
                            .map(|e| e.eq_ignore_ascii_case("ico"))
                            .unwrap_or(false)
                    {
                        icons.push(path);
                    }
                }
            }
            icons.sort();
            let exe_stem = Path::new(&meta.main_exe)
                .file_stem()
                .map(|s| s.to_string_lossy().to_ascii_lowercase());
            let chosen = icons
                .iter()
                .find(|p| {
                    p.file_stem()
                        .map(|s| s.to_string_lossy().to_ascii_lowercase())
                        == exe_stem
                })
                .or_else(|| icons.first());
            if let Some(chosen) = chosen {
                let dest = root.join("App").join("AppInfo").join("appicon.ico");
                if std::fs::copy(chosen, &dest).is_ok() {
                    outcome.icon_installed = true;
                    outcome.icon_source = Some("harvested".to_string());
                }
            }
        }
    }
    outcome.issues = meta_warnings;
    outcome.issues.extend(validate_tree(root, &meta));
    if main_exe_not_a_program {
        outcome.issues.push(ValidationIssue {
            error: false,
            field: "ProgramExecutable".to_string(),
            message: format!(
                "The main file {} does not look like a real Windows program file — its program header is missing. The package was still made, but check that the right file was captured.",
                meta.main_exe
            ),
        });
    }

    // Provenance, in Other\Source (where PortableForge already leaves
    // its note): a machine-readable build report, and a SHA-256
    // manifest of the package's authored and program files so any
    // later copy can be checked against the build. The manifest lists
    // the report; neither lists itself.
    let source_dir = root.join("Other").join("Source");
    std::fs::create_dir_all(&source_dir)
        .map_err(|e| format!("We could not make {}: {e}", source_dir.display()))?;
    // Size breakdown per top folder — computed before the report is
    // written; the report and the fingerprint list itself are the only
    // package files not counted.
    {
        let mut by_folder: std::collections::BTreeMap<String, (u64, usize)> =
            std::collections::BTreeMap::new();
        for path in package_files(root) {
            if path == source_dir.join("SHA256SUMS.txt") {
                continue;
            }
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('/', "\\");
            let top = rel.split('\\').next().unwrap_or("").to_string();
            let top = if top == rel {
                "(package root)".to_string()
            } else {
                top
            };
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            let entry = by_folder.entry(top).or_default();
            entry.0 += size;
            entry.1 += 1;
        }
        outcome.size_breakdown = by_folder
            .into_iter()
            .map(|(folder, (bytes, files))| (folder, bytes, files))
            .collect();
        outcome
            .size_breakdown
            .sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    }
    let report = serde_json::json!({
        "app_id": meta.app_id,
        "name": meta.name,
        "package_version": meta.package_version,
        "written": outcome.written,
        "app_files_copied": outcome.app_files_copied,
        "app_bytes_copied": outcome.app_bytes_copied,
        "default_data_files_copied": outcome.default_data_files_copied,
        "skipped": outcome.skipped,
        "license_file": outcome.license_file,
        "icon_installed": outcome.icon_installed,
        "icon_source": outcome.icon_source,
        "main_exe_arch": outcome.main_exe_arch,
        "preserved_data_files": outcome.preserved_data_files,
        "total_bytes": outcome.total_bytes(),
        "sizes": outcome.size_breakdown.iter().map(|(folder, bytes, files)| serde_json::json!({
            "folder": folder, "bytes": bytes, "files": files
        })).collect::<Vec<_>>(),
        "issues": outcome.issues.iter().map(|i| serde_json::json!({
            "error": i.error, "field": i.field, "message": i.message
        })).collect::<Vec<_>>(),
        "valid": !outcome.has_errors(),
    });
    let report_rel = r"Other\Source\build-report.json";
    std::fs::write(
        source_dir.join("build-report.json"),
        serde_json::to_string_pretty(&report).unwrap_or_default(),
    )
    .map_err(|e| format!("We could not write the build report: {e}"))?;
    outcome.written.push(report_rel.to_string());

    let manifest_files: Vec<std::path::PathBuf> = package_files(root)
        .into_iter()
        .filter(|p| *p != source_dir.join("SHA256SUMS.txt"))
        .collect();
    let mut manifest = String::new();
    for path in manifest_files {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('/', "\\");
        manifest.push_str(&format!(
            "{}  {}\n",
            crate::snapshot::sha256_hex(&bytes),
            rel
        ));
    }
    std::fs::write(source_dir.join("SHA256SUMS.txt"), &manifest)
        .map_err(|e| format!("We could not write the file list: {e}"))?;
    outcome
        .written
        .push(r"Other\Source\SHA256SUMS.txt".to_string());
    Ok(outcome)
}

/// CRC-32 (IEEE) for ZIP entries — table built on first use.
fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        *slot = c;
    }
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc = table[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

/// Write a plain `.zip` copy of the finished package tree (stored,
/// uncompressed entries — these packages are mostly already-compressed
/// program files). Entries are sorted and stamped with a fixed date,
/// so the same tree always zips to the same bytes. Returns how many
/// files went in. The package's `Data\` folder is left out, as in the
/// fingerprint list: it is the user's live data, not the package.
pub fn write_package_zip(root: &Path, zip_path: &Path) -> Result<usize, String> {
    let files = package_files(root);
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    // Fixed DOS date: 1980-01-01 (the earliest a zip can carry).
    const DOS_TIME: u16 = 0;
    const DOS_DATE: u16 = 0x21;
    for path in &files {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let data = std::fs::read(path)
            .map_err(|e| format!("We could not read {} for the zip copy: {e}", path.display()))?;
        let crc = crc32(&data);
        let size = data.len() as u32;
        let offset = out.len() as u32;
        let name = rel.as_bytes();
        // Local file header.
        out.extend_from_slice(&0x0403_4B50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0u16.to_le_bytes()); // flags
        out.extend_from_slice(&0u16.to_le_bytes()); // method: stored
        out.extend_from_slice(&DOS_TIME.to_le_bytes());
        out.extend_from_slice(&DOS_DATE.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra length
        out.extend_from_slice(name);
        out.extend_from_slice(&data);
        // Central directory header.
        central.extend_from_slice(&0x0201_4B50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes()); // version made by
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&DOS_TIME.to_le_bytes());
        central.extend_from_slice(&DOS_DATE.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra
        central.extend_from_slice(&0u16.to_le_bytes()); // comment
        central.extend_from_slice(&0u16.to_le_bytes()); // disk
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name);
    }
    let central_offset = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4B50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&central_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    if let Some(parent) = zip_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("We could not make {}: {e}", parent.display()))?;
    }
    std::fs::write(zip_path, &out).map_err(|e| format!("We could not write the zip copy: {e}"))?;
    Ok(files.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::{Classification, Finding, Severity};
    use crate::paf::Distribution;

    fn dirs_move() -> Classification {
        Classification {
            finding: Finding::DirectoriesMove {
                host_pattern: r"%APPDATA%\Acme\Widget".into(),
                data_target: "Widget".into(),
                evidence_file: r"C:\Users\X\AppData\Roaming\Acme\Widget\settings.ini".into(),
            },
            severity: Severity::Info,
            confidence: 0.9,
            why: String::new(),
        }
    }

    #[test]
    fn build_copies_files_seeds_data_and_keeps_accepted_fixes() {
        let base = std::env::temp_dir().join(format!("pf-build-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let app_src = base.join("harvest");
        std::fs::create_dir_all(&app_src).unwrap();
        std::fs::write(app_src.join("widget.exe"), b"MZfake").unwrap();
        std::fs::write(app_src.join("readme.txt"), b"hi").unwrap();
        let seed_src = base.join("seed").join("Widget");
        std::fs::create_dir_all(&seed_src).unwrap();
        std::fs::write(seed_src.join("settings.ini"), b"theme=dark").unwrap();

        let meta = crate::paf::tests::sample(Distribution::Bundled {
            redistribution_asserted: true,
        });
        let root = base.join("WidgetPortable");
        let outcome = build_package(
            &root,
            &meta,
            &[dirs_move()],
            Some(&app_src),
            Some(&base.join("seed")),
            None,
            false,
        )
        .unwrap();
        assert_eq!(outcome.app_files_copied, 2);
        assert_eq!(outcome.default_data_files_copied, 1);
        assert!(!outcome.has_errors(), "{:?}", outcome.issues);
        // The accepted fix survived into the Launcher settings — the
        // regression this module exists to prevent.
        let launcher = std::fs::read_to_string(
            root.join("App")
                .join("AppInfo")
                .join("Launcher")
                .join("WidgetPortable.ini"),
        )
        .unwrap();
        assert!(
            launcher.contains(r"Widget=%APPDATA%\Acme\Widget"),
            "{launcher}"
        );
        assert!(root
            .join("App")
            .join("DefaultData")
            .join("Widget")
            .join("settings.ini")
            .is_file());
        // Provenance: the build report is valid JSON naming the app,
        // and the manifest hashes the copied program file.
        let report_text =
            std::fs::read_to_string(root.join("Other").join("Source").join("build-report.json"))
                .unwrap();
        let report_json: serde_json::Value = serde_json::from_str(&report_text).unwrap();
        assert_eq!(report_json["app_id"], "WidgetPortable");
        assert_eq!(report_json["app_files_copied"], 2);
        assert_eq!(report_json["valid"], true);
        let manifest =
            std::fs::read_to_string(root.join("Other").join("Source").join("SHA256SUMS.txt"))
                .unwrap();
        let exe_hash = crate::snapshot::sha256_hex(b"MZfake");
        assert!(
            manifest.contains(&format!("{exe_hash}  App\\Widget\\widget.exe")),
            "{manifest}"
        );
        assert!(!manifest.contains("SHA256SUMS"));
        assert!(outcome.summary().contains("Everything checks out"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn preflight_lists_every_problem_and_writes_nothing() {
        let base = std::env::temp_dir().join(format!("pf-build-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let mut meta = crate::paf::tests::sample(Distribution::Bundled {
            redistribution_asserted: false,
        });
        meta.category = "Tools".into();
        meta.package_version = "1.2".into();
        let root = base.join("pkg");
        let err = build_package(&root, &meta, &[], None, None, None, false).unwrap_err();
        // All three problems in one message, not one at a time.
        assert!(err.contains("Category"), "{err}");
        assert!(err.contains("PackageVersion"), "{err}");
        assert!(err.contains("Distribution"), "{err}");
        // And nothing was written.
        assert!(!root.exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn license_and_icon_ride_along_and_data_survives_a_rebuild() {
        let base = std::env::temp_dir().join(format!("pf-build-extra-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let app_src = base.join("harvest");
        std::fs::create_dir_all(&app_src).unwrap();
        std::fs::write(app_src.join("widget.exe"), b"MZfake").unwrap();
        std::fs::write(app_src.join("LICENSE.txt"), b"MIT\n").unwrap();
        let icon = base.join("pretty.ico");
        std::fs::write(&icon, b"ICONBYTES").unwrap();
        let meta = crate::paf::tests::sample(Distribution::Bundled {
            redistribution_asserted: true,
        });
        let root = base.join("WidgetPortable");
        let outcome =
            build_package(&root, &meta, &[], Some(&app_src), None, Some(&icon), false).unwrap();
        assert_eq!(
            outcome.license_file.as_deref(),
            Some(r"App\AppInfo\LICENSE.txt")
        );
        assert!(root
            .join("App")
            .join("AppInfo")
            .join("LICENSE.txt")
            .is_file());
        assert!(outcome.icon_installed);
        assert_eq!(
            std::fs::read(root.join("App").join("AppInfo").join("appicon.ico")).unwrap(),
            b"ICONBYTES"
        );
        // Size breakdown covers the package and adds up.
        assert!(outcome.size_breakdown.iter().any(|(f, _, _)| f == "App"));
        assert!(outcome.total_bytes() > 0);
        let report_text =
            std::fs::read_to_string(root.join("Other").join("Source").join("build-report.json"))
                .unwrap();
        assert!(report_text.contains("\"total_bytes\""), "{report_text}");
        // The user runs the package; their data lands in Data\.
        let user_file = root.join("Data").join("settings").join("user.ini");
        std::fs::create_dir_all(user_file.parent().unwrap()).unwrap();
        std::fs::write(&user_file, b"mine=1").unwrap();
        // Rebuild: Data\ is kept exactly as it was, and the outcome says so.
        let outcome2 = build_package(&root, &meta, &[], Some(&app_src), None, None, false).unwrap();
        assert_eq!(outcome2.preserved_data_files, 1);
        assert_eq!(std::fs::read(&user_file).unwrap(), b"mine=1");
        assert!(outcome2.summary().contains("kept exactly as it was"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn building_twice_from_the_same_inputs_is_deterministic() {
        let base = std::env::temp_dir().join(format!("pf-build-det-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let app_src = base.join("harvest");
        std::fs::create_dir_all(&app_src).unwrap();
        std::fs::write(app_src.join("widget.exe"), b"MZfake").unwrap();
        let meta = crate::paf::tests::sample(Distribution::Bundled {
            redistribution_asserted: true,
        });
        let mut manifests = Vec::new();
        for name in ["one", "two"] {
            let root = base.join(name);
            build_package(
                &root,
                &meta,
                &[dirs_move()],
                Some(&app_src),
                None,
                None,
                false,
            )
            .unwrap();
            manifests.push(
                std::fs::read_to_string(root.join("Other").join("Source").join("SHA256SUMS.txt"))
                    .unwrap(),
            );
        }
        assert_eq!(manifests[0], manifests[1]);
        assert!(manifests[0].contains("widget.exe"));
        let _ = std::fs::remove_dir_all(&base);
    }

    fn fake_pe(machine: u16) -> Vec<u8> {
        let mut bytes = vec![0u8; 0x100];
        bytes[0..2].copy_from_slice(b"MZ");
        bytes[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        bytes[0x84..0x86].copy_from_slice(&machine.to_le_bytes());
        bytes
    }

    #[test]
    fn arch_is_detected_x64_gains_the_launcher_line_and_bad_files_warn() {
        let base = std::env::temp_dir().join(format!("pf-build-arch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let app_src = base.join("harvest");
        std::fs::create_dir_all(&app_src).unwrap();
        std::fs::write(app_src.join("widget.exe"), fake_pe(0x8664)).unwrap();
        std::fs::write(app_src.join("widget.ico"), b"ICO").unwrap();
        let meta = crate::paf::tests::sample(Distribution::Bundled {
            redistribution_asserted: true,
        });
        let root = base.join("pkg");
        let outcome = build_package(&root, &meta, &[], Some(&app_src), None, None, false).unwrap();
        assert_eq!(outcome.main_exe_arch.as_deref(), Some("x64"));
        let launcher = std::fs::read_to_string(
            root.join("App")
                .join("AppInfo")
                .join("Launcher")
                .join("WidgetPortable.ini"),
        )
        .unwrap();
        assert!(
            launcher.contains(r"ProgramExecutable64=Widget\widget.exe"),
            "{launcher}"
        );
        // The harvested .ico became the package picture on its own.
        assert!(outcome.icon_installed);
        assert_eq!(outcome.icon_source.as_deref(), Some("harvested"));
        assert!(!outcome.issues.iter().any(|i| i
            .message
            .contains("does not look like a real Windows program")));
        // A "program" with no real header earns the plain warning.
        std::fs::write(app_src.join("widget.exe"), b"not a program").unwrap();
        let root2 = base.join("pkg2");
        let outcome2 =
            build_package(&root2, &meta, &[], Some(&app_src), None, None, false).unwrap();
        assert_eq!(outcome2.main_exe_arch, None);
        assert!(outcome2.issues.iter().any(|i| !i.error
            && i.message
                .contains("does not look like a real Windows program")));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn nonempty_foreign_folder_is_refused_unless_forced() {
        let base = std::env::temp_dir().join(format!("pf-build-guard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("busy");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("someone-elses-file.txt"), b"mine").unwrap();
        let meta = crate::paf::tests::sample(Distribution::Bundled {
            redistribution_asserted: true,
        });
        let err = build_package(&root, &meta, &[], None, None, None, false).unwrap_err();
        assert!(err.contains("isn't empty"), "{err}");
        // The other file is untouched, and the override builds anyway.
        assert_eq!(
            std::fs::read(root.join("someone-elses-file.txt")).unwrap(),
            b"mine"
        );
        let outcome = build_package(&root, &meta, &[], None, None, None, true).unwrap();
        assert!(!outcome.written.is_empty());
        // And a normal rebuild (marker present) needs no override.
        let rebuild = build_package(&root, &meta, &[], None, None, None, false);
        assert!(rebuild.is_ok(), "{rebuild:?}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn zip_copy_is_well_formed_and_deterministic() {
        let base = std::env::temp_dir().join(format!("pf-build-zip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let app_src = base.join("harvest");
        std::fs::create_dir_all(&app_src).unwrap();
        std::fs::write(app_src.join("widget.exe"), b"MZfake").unwrap();
        let meta = crate::paf::tests::sample(Distribution::Bundled {
            redistribution_asserted: true,
        });
        let root = base.join("pkg");
        build_package(&root, &meta, &[], Some(&app_src), None, None, false).unwrap();
        let zip1 = base.join("a.zip");
        let zip2 = base.join("b.zip");
        let n = write_package_zip(&root, &zip1).unwrap();
        write_package_zip(&root, &zip2).unwrap();
        assert!(n > 3);
        let bytes = std::fs::read(&zip1).unwrap();
        assert_eq!(&bytes[0..2], b"PK");
        assert_eq!(bytes, std::fs::read(&zip2).unwrap());
        // The end record is findable and the exe is in there by name.
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("App/Widget/widget.exe"));
        assert_eq!(crc32(b"hello"), 0x3610_A686);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn build_without_the_program_files_reports_the_missing_exe() {
        let base = std::env::temp_dir().join(format!("pf-build-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let meta = crate::paf::tests::sample(Distribution::Bundled {
            redistribution_asserted: true,
        });
        let outcome =
            build_package(&base.join("pkg"), &meta, &[], None, None, None, false).unwrap();
        assert!(outcome.has_errors());
        assert!(outcome
            .issues
            .iter()
            .any(|i| i.error && i.message.contains("main executable")));
        let _ = std::fs::remove_dir_all(&base);
    }
}
