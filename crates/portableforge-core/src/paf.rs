//! PortableApps.com Format 3.9 package model + INI generation.
//!
//! Field rules follow the Format 3.9 specification
//! (portableapps.com/development/portableapps.com_format):
//! PackageVersion is strictly `x.x.x.x`, Description ≤ 512 chars,
//! Category is one of a fixed list, AppID uses a restricted charset.

use serde::{Deserialize, Serialize};

use crate::classify::{Classification, Finding};

/// The PAF 3.9 category list — exact strings, English regardless of the
/// app's own language.
pub const CATEGORIES: &[&str] = &[
    "Accessibility",
    "Development",
    "Education",
    "Games",
    "Graphics & Pictures",
    "Internet",
    "Music & Video",
    "Office",
    "Security",
    "Utilities",
];

/// How the finished package gets the base app's files — the
/// redistribution guardrail (SEAN, 2026-10-23: yes, with the choice
/// explicit). This is a deliberate user decision in the wizard, stamped
/// into the session log; it is never a silent default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Distribution {
    /// Files are bundled into the `.paf.exe`. Only valid when the user
    /// has explicitly asserted they have redistribution rights.
    Bundled { redistribution_asserted: bool },
    /// Online installer: the `.paf.exe` downloads the original installer
    /// at install time and verifies it by SHA-256 (`[DownloadFiles]`).
    Online {
        download_url: String,
        download_filename: String,
        download_sha256: String,
    },
}

impl Distribution {
    /// The guardrail, in code: bundling without an assertion is an error,
    /// and an online package without a verifiable hash is an error.
    pub fn check_guardrail(&self) -> Result<(), String> {
        match self {
            Distribution::Bundled {
                redistribution_asserted,
            } => {
                if *redistribution_asserted {
                    Ok(())
                } else {
                    Err(
                        "Bundled-files packaging requires the user to explicitly assert \
                         redistribution rights for the base app. Choose the online-installer \
                         distribution, or record the assertion."
                            .into(),
                    )
                }
            }
            Distribution::Online {
                download_url,
                download_sha256,
                download_filename,
            } => {
                if !download_url.starts_with("https://") {
                    return Err("Online distribution requires an https:// download URL.".into());
                }
                if download_filename.trim().is_empty() {
                    return Err("Online distribution requires the download filename.".into());
                }
                let h = download_sha256.trim();
                if h.len() != 64 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Err(
                        "Online distribution requires the download's SHA-256 (64 hex chars) \
                         so the installer can verify what it fetched."
                            .into(),
                    );
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PafMetadata {
    /// e.g. `Widget Portable`
    pub name: String,
    /// e.g. `WidgetPortable` (letters, numbers, `. - + _`)
    pub app_id: String,
    pub base_app_name: String,
    pub publisher: String,
    pub homepage: String,
    pub category: String,
    pub description: String,
    pub language: String,
    pub shareable: bool,
    pub open_source: bool,
    pub freeware: bool,
    pub commercial_use: bool,
    /// `x.x.x.x`
    pub package_version: String,
    /// e.g. `1.2 Release 1`
    pub display_version: String,
    pub requires_admin: bool,
    /// Main executable inside `App\<BaseAppName>\`, e.g. `widget.exe`.
    pub main_exe: String,
    /// Set when the main file is known to be a 64-bit build (the build
    /// detects it from the file's own header): the Launcher settings
    /// then carry the matching 64-bit program line.
    #[serde(default)]
    pub main_exe_64bit: Option<bool>,
    pub distribution: Distribution,
}

impl PafMetadata {
    /// The launcher executable / package folder base name (`<AppID>`).
    pub fn launcher_name(&self) -> String {
        format!("{}.exe", self.app_id)
    }

    pub fn appinfo_ini(&self) -> String {
        let admin = if self.requires_admin { "yes" } else { "no" };
        format!(
            "[Format]\n\
             Type=PortableApps.comFormat\n\
             Version=3.9\n\n\
             [Details]\n\
             Name={name}\n\
             AppID={app_id}\n\
             BaseAppName={base}\n\
             Publisher={publisher}\n\
             Homepage={homepage}\n\
             Category={category}\n\
             Description={description}\n\
             Language={language}\n\n\
             [License]\n\
             Shareable={shareable}\n\
             OpenSource={open_source}\n\
             Freeware={freeware}\n\
             CommercialUse={commercial}\n\n\
             [Version]\n\
             PackageVersion={pv}\n\
             DisplayVersion={dv}\n\n\
             [Dependencies]\n\
             RequiresAdmin={admin}\n\n\
             [Control]\n\
             Icons=1\n\
             Start={app_id}.exe\n\
             ExtractIcon=App\\{base}\\{main_exe}\n",
            name = ini_safe(&self.name),
            app_id = ini_safe(&self.app_id),
            base = ini_safe(&self.base_app_name),
            publisher = ini_safe(&self.publisher),
            homepage = ini_safe(&self.homepage),
            category = ini_safe(&self.category),
            description = ini_safe(&self.description),
            language = ini_safe(&self.language),
            shareable = self.shareable,
            open_source = self.open_source,
            freeware = self.freeware,
            commercial = self.commercial_use,
            pv = ini_safe(&self.package_version),
            dv = ini_safe(&self.display_version),
            admin = admin,
            main_exe = ini_safe(&self.main_exe),
        )
    }

    /// Launcher `.ini` (`App\AppInfo\Launcher\<AppID>.ini`) built from the
    /// accepted classifications.
    ///
    /// Syntax verified against the official PortableApps.com Application
    /// Template 3.5.2 (a primary source, downloaded and inspected):
    /// - `[DirectoriesMove]`: `<DataFolderName>=<HostPath>`
    /// - `[FilesMove]`: `<FileName>=<HostDirectory>`
    /// - `[RegistryKeys]`: `<RegFileStem>=<FullKey>` (the stem names the
    ///   `.reg` file the launcher keeps in `Data\`)
    /// - `[FileWriteN]`: `Type=Replace` with the path-migration triple
    ///   (package dir, base dir, drive — most specific first).
    pub fn launcher_ini(&self, accepted: &[Classification]) -> String {
        let mut out = String::new();
        out.push_str("[Launch]\n");
        out.push_str(&format!(
            "ProgramExecutable={}\\{}\n",
            ini_safe(&self.base_app_name),
            ini_safe(&self.main_exe)
        ));
        if self.main_exe_64bit == Some(true) {
            out.push_str(&format!(
                "ProgramExecutable64={}\\{}\n",
                ini_safe(&self.base_app_name),
                ini_safe(&self.main_exe)
            ));
        }
        out.push_str("DirectoryMoveOK=yes\n");
        out.push_str("SupportsUNC=yes\n");

        let mut dirs: Vec<(&String, &String, &String)> = Vec::new(); // (data folder, host, evidence)
        let mut files: Vec<(&String, &String)> = Vec::new();
        let mut regs: Vec<&String> = Vec::new();
        let mut writes: Vec<&String> = Vec::new();
        for c in accepted {
            match &c.finding {
                Finding::DirectoriesMove {
                    host_pattern,
                    data_target,
                    evidence_file,
                } => dirs.push((data_target, host_pattern, evidence_file)),
                Finding::FilesMove {
                    host_pattern,
                    data_target,
                } => files.push((data_target, host_pattern)),
                Finding::RegistryKey { key } => regs.push(key),
                Finding::FileWriteRewrite { path, .. } => writes.push(path),
                _ => {}
            }
        }

        if !regs.is_empty() || !writes.is_empty() {
            out.push_str("\n[Activate]\n");
            if !regs.is_empty() {
                out.push_str("Registry=true\n");
            }
            if !writes.is_empty() {
                out.push_str("XML=true\n");
            }
        }
        if !dirs.is_empty() {
            out.push_str("\n[DirectoriesMove]\n");
            for (data_folder, host, _) in &dirs {
                // Defensive (PAL manual): `settings` is reserved for
                // registry storage in Data — never a move key.
                let key = if data_folder.eq_ignore_ascii_case("settings") {
                    format!("{data_folder}Data")
                } else {
                    (*data_folder).clone()
                };
                out.push_str(&format!("{key}={host}\n"));
            }
        }
        if !files.is_empty() {
            out.push_str("\n[FilesMove]\n");
            for (file_name, host_dir) in &files {
                out.push_str(&format!("{file_name}={host_dir}\n"));
            }
        }
        if !regs.is_empty() {
            out.push_str("\n[RegistryKeys]\n");
            let mut used: Vec<String> = Vec::new();
            for key in &regs {
                // Stem = last key component, sanitised; dedupe with a
                // numeric suffix only on collision (template style).
                let mut stem: String = key
                    .rsplit('\\')
                    .next()
                    .unwrap_or("Settings")
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
                    .collect();
                if stem.is_empty() {
                    stem = "Settings".to_string();
                }
                let mut candidate = stem.clone();
                let mut n = 2;
                while used.contains(&candidate) {
                    candidate = format!("{stem}{n}");
                    n += 1;
                }
                used.push(candidate.clone());
                out.push_str(&format!("{candidate}={key}\n"));
            }
        }
        // FileWrite: one path-migration triple per affected file, in the
        // template's order (most specific path first). The file's Data
        // location comes from a matching DirectoriesMove when the file
        // lived under a moved folder; otherwise Data\settings.
        let mut n = 0usize;
        for path in &writes {
            let file_name = path.rsplit('\\').next().unwrap_or(path);
            let mut data_loc = format!("%PAL:DataDir%\\settings\\{file_name}");
            for (data_folder, host, _) in &dirs {
                // host is like %APPDATA%\Acme\Widget; the rewrite path is
                // an absolute sandbox path ending with the same tail.
                let tail = host.split_once('\\').map(|x| x.1).unwrap_or("");
                if !tail.is_empty()
                    && path
                        .to_ascii_lowercase()
                        .contains(&tail.to_ascii_lowercase())
                {
                    data_loc = format!("%PAL:DataDir%\\{data_folder}\\{file_name}");
                    break;
                }
            }
            for (find, replace) in [
                (
                    "%PAL:LastDrive%%PAL:LastPackagePartialDir%\\",
                    "%PAL:Drive%%PAL:PackagePartialDir%\\",
                ),
                (
                    "%PAL:LastDrive%%PAL:LastPortableAppsBaseDir%\\",
                    "%PAL:Drive%%PAL:PortableAppsBaseDir%\\",
                ),
                ("%PAL:LastDrive%\\", "%PAL:Drive%\\"),
            ] {
                n += 1;
                out.push_str(&format!(
                    "\n[FileWrite{n}]\nType=Replace\nFile={data_loc}\nFind={find}\nReplace={replace}\n"
                ));
            }
        }
        out
    }

    /// `installer.ini` with preservation rules derived from accepted
    /// findings: when the launcher moves data into the App directory
    /// while running, the installer must preserve it on upgrade in case
    /// of a crash before the launcher can move it back (the reason the
    /// official template ships an installer.ini at all).
    pub fn installer_ini_with(&self, accepted: &[Classification]) -> String {
        let mut out = self.installer_ini();
        let mut preserve_dirs: Vec<String> = Vec::new();
        let mut preserve_files: Vec<String> = Vec::new();
        for c in accepted {
            match &c.finding {
                Finding::DirectoriesMove { host_pattern, .. }
                    if host_pattern.starts_with("%PAL:AppDir%") =>
                {
                    preserve_dirs.push(
                        host_pattern
                            .trim_start_matches("%PAL:AppDir%\\")
                            .to_string(),
                    );
                }
                Finding::FilesMove { host_pattern, .. }
                    if host_pattern.starts_with("%PAL:AppDir%") =>
                {
                    preserve_files.push(
                        host_pattern
                            .trim_start_matches("%PAL:AppDir%\\")
                            .to_string(),
                    );
                }
                _ => {}
            }
        }
        if !preserve_dirs.is_empty() {
            out.push_str("\n[DirectoriesToPreserve]\n");
            for (i, d) in preserve_dirs.iter().enumerate() {
                out.push_str(&format!("PreserveDirectory{}=App\\{d}\n", i + 1));
            }
        }
        if !preserve_files.is_empty() {
            out.push_str("\n[FilesToPreserve]\n");
            for (i, f) in preserve_files.iter().enumerate() {
                out.push_str(&format!("PreserveFile{}=App\\{f}\n", i + 1));
            }
        }
        out
    }

    /// `installer.ini` — empty-ish for bundled packages; carries the
    /// `[DownloadFiles]` block for online packages.
    pub fn installer_ini(&self) -> String {
        let mut out = String::from("[Source]\nIncludeInstallerSource=false\n");
        if let Distribution::Online {
            download_url,
            download_filename,
            download_sha256,
        } = &self.distribution
        {
            out.push_str(&format!(
                "\n[DownloadFiles]\n\
                 DownloadURL={}\n\
                 DownloadName={}\n\
                 DownloadFilename={}\n\
                 DownloadSHA256={}\n\
                 DownloadTo=\n",
                ini_safe(download_url),
                ini_safe(&self.name),
                ini_safe(download_filename),
                ini_safe(download_sha256)
            ));
        }
        out
    }

    pub fn help_html(&self) -> String {
        format!(
            "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\">\
             <title>{name}</title></head><body>\n<h1>{name}</h1>\n\
             <p>{desc}</p>\n\
             <p>Version: {version} (package {package_version})<br>Publisher: {publisher}</p>\n\
             <p>Made with PortableForge as a PortableApps.com Format package. \
             All user data is stored in the <b>Data</b> directory next to the launcher, \
             so the whole folder can be copied or backed up as one piece.</p>\n\
             </body></html>\n",
            name = html_escape(&self.name),
            desc = html_escape(&self.description),
            version = html_escape(&self.display_version),
            package_version = html_escape(&self.package_version),
            publisher = html_escape(&self.publisher)
        )
    }
}

/// Escape user text for HTML. Names and descriptions come from people
/// (and from program metadata we did not write); without this a `<` in
/// a description would become markup in the generated help page.
pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Make a value safe for one INI line: CR/LF would let a crafted name
/// or description inject whole new INI lines into generated files.
pub fn ini_safe(s: &str) -> String {
    s.chars().filter(|c| *c != '\r' && *c != '\n').collect()
}

/// Pad a program's own version tag (`1.2`, `1.2.3`) out to the four
/// numbers a PackageVersion needs. `None` when the tag is not plain
/// numbers and dots — never guess at letters or suffixes.
pub fn pad_package_version(v: &str) -> Option<String> {
    let parts: Vec<&str> = v.trim().split('.').collect();
    if parts.is_empty() || parts.len() > 4 {
        return None;
    }
    let mut nums: Vec<String> = Vec::new();
    for p in parts {
        if p.is_empty() || !p.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        // Normalise leading zeros away ("01" -> "1").
        nums.push(p.parse::<u64>().ok()?.to_string());
    }
    while nums.len() < 4 {
        nums.push("0".to_string());
    }
    Some(nums.join("."))
}

/// Parse/validate a PackageVersion (`x.x.x.x`, four numeric parts).
pub fn valid_package_version(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() == 4
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

pub fn valid_app_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | '_'))
        && !id.contains(' ')
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn sample(distribution: Distribution) -> PafMetadata {
        PafMetadata {
            name: "Widget Portable".into(),
            app_id: "WidgetPortable".into(),
            base_app_name: "Widget".into(),
            publisher: "Acme & PortableApps.com".into(),
            homepage: "https://example.com/widget-portable".into(),
            category: "Utilities".into(),
            description: "A widget tool".into(),
            language: "English".into(),
            shareable: true,
            open_source: true,
            freeware: true,
            commercial_use: true,
            package_version: "1.2.0.1".into(),
            display_version: "1.2 Release 1".into(),
            requires_admin: false,
            main_exe: "widget.exe".into(),
            main_exe_64bit: None,
            distribution,
        }
    }

    #[test]
    fn guardrail_blocks_unasserted_bundling_and_unhashed_downloads() {
        assert!(Distribution::Bundled {
            redistribution_asserted: false
        }
        .check_guardrail()
        .is_err());
        assert!(Distribution::Bundled {
            redistribution_asserted: true
        }
        .check_guardrail()
        .is_ok());
        assert!(Distribution::Online {
            download_url: "https://example.com/w.exe".into(),
            download_filename: "w.exe".into(),
            download_sha256: "abc".into(),
        }
        .check_guardrail()
        .is_err());
        assert!(Distribution::Online {
            download_url: "https://example.com/w.exe".into(),
            download_filename: "w.exe".into(),
            download_sha256: "a".repeat(64),
        }
        .check_guardrail()
        .is_ok());
        assert!(Distribution::Online {
            download_url: "http://example.com/w.exe".into(),
            download_filename: "w.exe".into(),
            download_sha256: "a".repeat(64),
        }
        .check_guardrail()
        .is_err());
    }

    #[test]
    fn appinfo_ini_matches_format_39_shape() {
        let m = sample(Distribution::Bundled {
            redistribution_asserted: true,
        });
        let ini = m.appinfo_ini();
        assert!(ini.contains("Type=PortableApps.comFormat"));
        assert!(ini.contains("Version=3.9"));
        assert!(ini.contains("PackageVersion=1.2.0.1"));
        assert!(ini.contains("Start=WidgetPortable.exe"));
        assert!(ini.contains(r"ExtractIcon=App\Widget\widget.exe"));
    }

    #[test]
    fn online_installer_ini_carries_download_block() {
        let m = sample(Distribution::Online {
            download_url: "https://example.com/widget-setup.exe".into(),
            download_filename: "widget-setup.exe".into(),
            download_sha256: "b".repeat(64),
        });
        let ini = m.installer_ini();
        assert!(ini.contains("[DownloadFiles]"));
        assert!(ini.contains("DownloadSHA256="));
        assert!(ini.contains("widget-setup.exe"));
    }

    #[test]
    fn launcher_ini_matches_official_template_semantics() {
        use crate::classify::{Classification, Finding, Severity};
        let m = sample(Distribution::Bundled {
            redistribution_asserted: true,
        });
        let accepted = vec![
            Classification {
                finding: Finding::DirectoriesMove {
                    host_pattern: r"%APPDATA%\Acme\Widget".into(),
                    data_target: "Widget".into(),
                    evidence_file: r"C:\Users\X\AppData\Roaming\Acme\Widget\settings.ini".into(),
                },
                severity: Severity::Info,
                confidence: 0.9,
                why: String::new(),
            },
            Classification {
                finding: Finding::RegistryKey {
                    key: r"HKEY_CURRENT_USER\Software\Acme\Widget".into(),
                },
                severity: Severity::Info,
                confidence: 0.9,
                why: String::new(),
            },
            Classification {
                finding: Finding::FileWriteRewrite {
                    path: r"C:\Users\WDAGUtilityAccount\AppData\Roaming\Acme\Widget\settings.ini"
                        .into(),
                    matched_text: "WDAGUtilityAccount".into(),
                },
                severity: Severity::Review,
                confidence: 0.9,
                why: String::new(),
            },
        ];
        let ini = m.launcher_ini(&accepted);
        // Template form: DataFolder=HostPath, Stem=FullKey.
        assert!(ini.contains(r"Widget=%APPDATA%\Acme\Widget"), "{ini}");
        assert!(
            ini.contains(r"Widget=HKEY_CURRENT_USER\Software\Acme\Widget"),
            "{ini}"
        );
        assert!(ini.contains("Registry=true"));
        // FileWrite triple, most specific first, file resolved into the
        // moved Data folder.
        let fw1 = ini.find("[FileWrite1]").unwrap();
        let fw3 = ini.find("[FileWrite3]").unwrap();
        assert!(fw1 < fw3);
        assert!(
            ini.contains(r"File=%PAL:DataDir%\Widget\settings.ini"),
            "{ini}"
        );
        assert!(ini.contains("Find=%PAL:LastDrive%%PAL:LastPackagePartialDir%\\"));
    }

    #[test]
    fn help_html_escapes_user_text_and_ini_values_cannot_inject_lines() {
        let mut m = sample(Distribution::Bundled {
            redistribution_asserted: true,
        });
        m.description = "A <b>widget</b> & \"friends\"".into();
        let html = m.help_html();
        assert!(html.contains("A &lt;b&gt;widget&lt;/b&gt; &amp; &quot;friends&quot;"));
        assert!(!html.contains("<b>widget</b>"));
        m.name = "Widget\n[Injected]\nEvil=1".into();
        let ini = m.appinfo_ini();
        // The injected text survives only as part of the Name value —
        // never as its own INI line or section header.
        assert!(ini
            .lines()
            .all(|l| !l.starts_with("[Injected]") && !l.starts_with("Evil")));
        assert!(ini.contains("Name=Widget[Injected]Evil=1"));
        assert_eq!(ini_safe("a\r\nb"), "ab");
    }

    #[test]
    fn help_html_names_version_publisher_and_maker() {
        let m = sample(Distribution::Bundled {
            redistribution_asserted: true,
        });
        let html = m.help_html();
        assert!(html.contains("Widget Portable"));
        assert!(html.contains("1.2 Release 1"));
        assert!(html.contains("1.2.0.1"));
        assert!(html.contains("Acme &amp; PortableApps.com"));
        assert!(html.contains("Made with PortableForge"));
    }

    #[test]
    fn version_tags_pad_to_four_numbers_or_refuse() {
        assert_eq!(pad_package_version("1.2").as_deref(), Some("1.2.0.0"));
        assert_eq!(pad_package_version("1.2.3").as_deref(), Some("1.2.3.0"));
        assert_eq!(pad_package_version("1.2.3.4").as_deref(), Some("1.2.3.4"));
        assert_eq!(pad_package_version("01.02").as_deref(), Some("1.2.0.0"));
        assert_eq!(pad_package_version(" 2.0 ").as_deref(), Some("2.0.0.0"));
        assert_eq!(pad_package_version("1.2-beta"), None);
        assert_eq!(pad_package_version("1.2.3.4.5"), None);
        assert_eq!(pad_package_version(""), None);
        assert_eq!(pad_package_version("1..2"), None);
    }

    #[test]
    fn version_and_appid_rules() {
        assert!(valid_package_version("1.2.0.1"));
        assert!(!valid_package_version("1.2"));
        assert!(!valid_package_version("1.2.0.1-rc"));
        assert!(valid_app_id("WidgetPortable"));
        assert!(valid_app_id("WidgetPortable-example.com"));
        assert!(!valid_app_id("Widget Portable"));
    }
}
