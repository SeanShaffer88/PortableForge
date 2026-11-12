//! Read-only structural and portability preflight for inventoried folders.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use super::{EntryKind, FolderIssue, FolderScan, IssueSeverity};
use crate::path_safety::safe_name;

const APPINFO: &str = "App/AppInfo/appinfo.ini";
const INSTALLER: &str = "App/AppInfo/installer.ini";
const MAX_TEXT_BYTES: usize = 128 * 1024;
const MAX_RAW_CONFIGS: usize = 32;

pub fn validate(scan: &FolderScan, selected: Option<&Path>) -> Vec<FolderIssue> {
    let mut issues = Vec::new();
    if !scan.complete
        || scan
            .issues
            .iter()
            .any(|issue| issue.severity == IssueSeverity::Error)
    {
        issues.push(issue(
            "scan_incomplete",
            IssueSeverity::Error,
            None,
            "The folder could not be checked completely. Resolve the scan problems and scan again.",
        ));
        return issues;
    }

    let prepared = scan.is_prepared();
    let start = if prepared {
        check_prepared(scan, &mut issues)
    } else {
        if let Some(issue) = first_unsupported_payload_path(scan) {
            issues.push(issue);
        }
        check_raw_configs(scan, &mut issues);
        None
    };
    let recommended = if prepared {
        start.as_deref()
    } else {
        scan.recommendation.as_deref()
    };
    let chosen = selected.or(recommended);
    match chosen {
        Some(path) if !has_candidate(scan, path) => issues.push(issue(
            "invalid_selection",
            IssueSeverity::Error,
            None,
            "Choose an executable from this folder's scan results.",
        )),
        Some(path) if start.as_ref().is_some_and(|start| !same_path(start, path)) => {
            issues.push(issue(
                "selection_differs_from_start",
                IssueSeverity::Error,
                Some(Path::new(APPINFO)),
                "The selected executable differs from the prepared package's Start entry.",
            ));
        }
        Some(_) => {}
        None if !prepared => issues.push(issue(
            "choose_executable",
            IssueSeverity::Error,
            None,
            "Choose which executable starts the app.",
        )),
        None => {}
    }
    issues
}

fn first_unsupported_payload_path(scan: &FolderScan) -> Option<FolderIssue> {
    let entry = scan.entries.iter().find(|entry| {
        entry.relative_path.components().any(|component| {
            !matches!(component, Component::Normal(name) if name.to_str().is_some_and(safe_name))
        })
    })?;
    let path = &entry.relative_path;
    Some(issue(
        "unsupported_payload_path",
        IssueSeverity::Error,
        Some(path),
        &format!(
            "The installer builder cannot package {}. Rename or remove the entry in a working copy, then scan again.",
            path.display()
        ),
    ))
}

fn issue(
    code: &'static str,
    severity: IssueSeverity,
    path: Option<&Path>,
    message: &str,
) -> FolderIssue {
    FolderIssue {
        code,
        severity,
        relative_path: path.map(Path::to_path_buf),
        message: message.into(),
    }
}

pub(super) fn same_path(a: &Path, b: &Path) -> bool {
    let (mut a, mut b) = (a.components(), b.components());
    loop {
        match (a.next(), b.next()) {
            (Some(a), Some(b)) if same_name(a.as_os_str(), b.as_os_str()) => {}
            (None, None) => return true,
            _ => return false,
        }
    }
}

fn same_name(a: &OsStr, b: &OsStr) -> bool {
    if a.eq_ignore_ascii_case(b) {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;

        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn CompareStringOrdinal(
                a: *const u16,
                a_len: i32,
                b: *const u16,
                b_len: i32,
                ignore_case: i32,
            ) -> i32;
        }
        if a.is_ascii() && b.is_ascii() {
            return false;
        }
        let a: Vec<_> = a.encode_wide().collect();
        let b: Vec<_> = b.encode_wide().collect();
        let (Ok(a_len), Ok(b_len)) = (i32::try_from(a.len()), i32::try_from(b.len())) else {
            return false;
        };
        // SAFETY: both slices stay alive, with the exact supplied UTF-16 lengths.
        unsafe { CompareStringOrdinal(a.as_ptr(), a_len, b.as_ptr(), b_len, 1) == 2 }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn relative_target(value: &str) -> Option<PathBuf> {
    let value = value
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(value);
    let parts: Vec<_> = value.split(['\\', '/']).collect();
    if !parts
        .iter()
        .all(|part| safe_name(part) && !part.contains(['%', '$', '`']))
        || !parts.last()?.to_ascii_lowercase().ends_with(".exe")
    {
        return None;
    }
    Some(parts.into_iter().collect())
}

fn has_candidate(scan: &FolderScan, path: &Path) -> bool {
    path.to_str()
        .and_then(relative_target)
        .is_some_and(|clean| clean == path)
        && scan
            .candidates
            .iter()
            .any(|candidate| same_path(&candidate.relative_path, path))
        && inventoried_file(scan, path).is_some()
}

fn inventoried_file<'a>(scan: &'a FolderScan, path: &Path) -> Option<&'a Path> {
    scan.entries
        .iter()
        .find(|entry| entry.kind == EntryKind::File && same_path(&entry.relative_path, path))
        .map(|entry| entry.relative_path.as_path())
}

type Ini = BTreeMap<(String, String), String>;

fn read_ini(scan: &FolderScan, path: &Path) -> Result<Ini, &'static str> {
    let bytes = scan
        .read_small(path, MAX_TEXT_BYTES)
        .map_err(|_| "The settings file could not be read within the scan limits.")?;
    let text =
        decode_text(&bytes).ok_or("The settings file is not supported UTF-8 or UTF-16 text.")?;
    parse_ini(&text)
}

fn decode_text(bytes: &[u8]) -> Option<String> {
    let utf16 = bytes
        .strip_prefix(&[0xff, 0xfe])
        .map(|rest| (rest, u16::from_le_bytes as fn([u8; 2]) -> u16))
        .or_else(|| {
            bytes
                .strip_prefix(&[0xfe, 0xff])
                .map(|rest| (rest, u16::from_be_bytes as fn([u8; 2]) -> u16))
        });
    let text = if let Some((data, decode)) = utf16 {
        let (pairs, remainder) = data.as_chunks::<2>();
        if !remainder.is_empty() {
            return None;
        }
        let units: Vec<_> = pairs.iter().copied().map(decode).collect();
        String::from_utf16(&units).ok()?
    } else {
        let data = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
        std::str::from_utf8(data).ok()?.to_owned()
    };
    text.chars()
        .all(|ch| !ch.is_control() || matches!(ch, '\n' | '\r' | '\t'))
        .then_some(text)
}

fn parse_ini(text: &str) -> Result<Ini, &'static str> {
    let mut values = BTreeMap::new();
    let mut sections = BTreeSet::new();
    let mut section = String::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[') {
            let name = name
                .strip_suffix(']')
                .map(str::trim)
                .filter(|name| !name.is_empty() && !name.contains(['[', ']']))
                .ok_or("The settings file contains a malformed section.")?;
            section = name.to_ascii_lowercase();
            if !sections.insert(section.clone()) {
                return Err("The settings file contains a repeated section.");
            }
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or("The settings file contains a malformed entry.")?;
        let key = key.trim().to_ascii_lowercase();
        if section.is_empty() || key.is_empty() || key.contains(['[', ']']) {
            return Err("The settings file contains an entry without a section or setting name.");
        }
        if values
            .insert((section.clone(), key), value.trim().into())
            .is_some()
        {
            return Err("The settings file contains a repeated entry.");
        }
    }
    Ok(values)
}

fn field<'a>(ini: &'a Ini, section: &str, name: &str) -> &'a str {
    ini.get(&(section.into(), name.into()))
        .map_or("", String::as_str)
}

fn check_prepared(scan: &FolderScan, issues: &mut Vec<FolderIssue>) -> Option<PathBuf> {
    let expected = Path::new(APPINFO);
    let appinfo_path = inventoried_file(scan, expected);
    let result = match appinfo_path {
        Some(path) => read_ini(scan, path),
        None => Err("The prepared package needs App/AppInfo/appinfo.ini."),
    };
    let info = match result {
        Ok(info) => info,
        Err(message) => {
            issues.push(issue(
                "invalid_appinfo",
                IssueSeverity::Error,
                Some(expected),
                message,
            ));
            check_installer(scan, issues);
            return None;
        }
    };
    let path = appinfo_path.unwrap_or(expected);
    check_app_metadata(&info, path, issues);
    let start = check_target(
        scan,
        path,
        field(&info, "control", "start"),
        Path::new(""),
        "invalid_start",
        "Control Start must name an executable inside this package.",
        issues,
    );

    let app_id = field(&info, "details", "appid");
    if valid_app_id(app_id) {
        let launcher = Path::new("App/AppInfo/Launcher").join(format!("{app_id}.ini"));
        if let Some(path) = inventoried_file(scan, &launcher) {
            check_launcher(scan, path, issues);
        } else {
            issues.push(issue("launcher_settings_unverified", IssueSeverity::Warning, Some(&launcher),
                "No PortableApps.com Launcher settings were found. This package may use its own launcher."));
        }
    }
    check_installer(scan, issues);
    start
}

fn valid_app_id(app_id: &str) -> bool {
    crate::paf::valid_app_id(app_id) && safe_name(app_id)
}

fn check_app_metadata(info: &Ini, path: &Path, issues: &mut Vec<FolderIssue>) {
    let get = |section, key| field(info, section, key);
    let version = get("format", "version");
    if !get("format", "type").eq_ignore_ascii_case("PortableApps.comFormat")
        || !version.split_once('.').is_some_and(|(major, minor)| {
            major.parse::<u16>().is_ok() && minor.parse::<u16>().is_ok()
        })
    {
        issues.push(issue(
            "invalid_package_format",
            IssueSeverity::Error,
            Some(path),
            "The package format needs Type PortableApps.comFormat and a numeric format version.",
        ));
    } else if version != "3.9" {
        issues.push(issue("format_version_unverified", IssueSeverity::Warning, Some(path),
            "This package declares a format version other than 3.9. Build compatibility is unverified."));
    }

    for (section, key, message) in [
        ("details", "name", "The app display name is missing."),
        ("details", "publisher", "The app publisher is missing."),
        ("details", "homepage", "The app homepage is missing."),
        ("details", "category", "The app category is missing."),
        ("details", "description", "The app description is missing."),
        ("details", "language", "The app language is missing."),
        (
            "version",
            "displayversion",
            "The app display version is missing.",
        ),
    ] {
        if get(section, key).is_empty() {
            issues.push(issue(
                "missing_metadata",
                IssueSeverity::Error,
                Some(path),
                message,
            ));
        }
    }
    let package_version = get("version", "packageversion");
    if !valid_app_id(get("details", "appid")) {
        issues.push(issue("invalid_app_id", IssueSeverity::Error, Some(path),
            "The app ID is missing or contains characters that cannot be used in a package folder name."));
    }
    if !crate::paf::valid_package_version(package_version)
        || !package_version
            .split('.')
            .all(|part| part.parse::<u16>().is_ok())
    {
        issues.push(issue(
            "invalid_package_version",
            IssueSeverity::Error,
            Some(path),
            "The package version needs four numbers between 0 and 65535.",
        ));
    }
    if !get("control", "icons")
        .parse::<u16>()
        .is_ok_and(|icons| icons > 0)
    {
        issues.push(issue(
            "invalid_icon_count",
            IssueSeverity::Error,
            Some(path),
            "Control Icons must specify at least one menu icon.",
        ));
    }
    let licenses = ["shareable", "opensource", "freeware", "commercialuse"];
    if licenses.into_iter().any(|key| {
        !matches!(
            get("license", key).to_ascii_lowercase().as_str(),
            "true" | "false"
        )
    }) {
        issues.push(issue("invalid_license_fields", IssueSeverity::Error, Some(path),
            "The package license fields need true or false values. Check Shareable, OpenSource, Freeware, and CommercialUse."));
    }
}

fn check_target(
    scan: &FolderScan,
    settings: &Path,
    target: &str,
    base: &Path,
    code: &'static str,
    message: &str,
    issues: &mut Vec<FolderIssue>,
) -> Option<PathBuf> {
    let path = relative_target(target).map(|relative| base.join(relative));
    if let Some(candidate) = path.filter(|candidate| has_candidate(scan, candidate)) {
        return Some(candidate);
    }
    issues.push(issue(code, IssueSeverity::Error, Some(settings), message));
    None
}

fn check_launcher(scan: &FolderScan, path: &Path, issues: &mut Vec<FolderIssue>) {
    let launcher = match read_ini(scan, path) {
        Ok(launcher) => launcher,
        Err(message) => {
            issues.push(issue(
                "invalid_launcher",
                IssueSeverity::Error,
                Some(path),
                message,
            ));
            return;
        }
    };
    for key in [
        "programexecutable",
        "programexecutable64",
        "programexecutablewhenparameters",
        "programexecutablewhenparameters64",
    ] {
        let target = field(&launcher, "launch", key);
        if key == "programexecutable" || !target.is_empty() {
            check_target(
                scan,
                path,
                target,
                Path::new("App"),
                "invalid_launcher_target",
                "A launcher executable entry must name a scanned program inside the App folder.",
                issues,
            );
        }
    }
}

fn check_installer(scan: &FolderScan, issues: &mut Vec<FolderIssue>) {
    let expected = Path::new(INSTALLER);
    let Some(path) = inventoried_file(scan, expected) else {
        if scan
            .entries
            .iter()
            .any(|entry| same_path(&entry.relative_path, expected))
        {
            issues.push(issue(
                "invalid_installer",
                IssueSeverity::Error,
                Some(expected),
                "Installer settings must be a regular file.",
            ));
        }
        return;
    };
    let installer = match read_ini(scan, path) {
        Ok(installer) => installer,
        Err(message) => {
            issues.push(issue(
                "invalid_installer",
                IssueSeverity::Error,
                Some(path),
                message,
            ));
            return;
        }
    };
    if installer.keys().any(|(section, key)| {
        !matches!(
            (section.as_str(), key.as_str()),
            ("source", "includeinstallersource") | ("checkrunning", "closeexe" | "closename")
        )
    }) {
        issues.push(issue(
            "installer_settings_unverified",
            IssueSeverity::Warning,
            Some(path),
            "This file includes installer options that the folder check does not verify.",
        ));
    }
    let source = field(&installer, "source", "includeinstallersource");
    let close = field(&installer, "checkrunning", "closeexe");
    let close_name = field(&installer, "checkrunning", "closename");
    if !matches!(source.to_ascii_lowercase().as_str(), "" | "true" | "false")
        || (!close.is_empty()
            && close != "NONE"
            && (!safe_name(close) || !close.to_ascii_lowercase().ends_with(".exe")))
        || close_name.chars().any(|ch| ch.is_control())
    {
        issues.push(issue("invalid_installer_values", IssueSeverity::Error, Some(path),
            "Installer settings contain an invalid source flag, close executable name, or display name."));
    }
}

fn check_raw_configs(scan: &FolderScan, issues: &mut Vec<FolderIssue>) {
    let configs = scan.entries.iter().filter(|entry| {
        entry.kind == EntryKind::File
            && entry
                .relative_path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    matches!(
                        extension.to_ascii_lowercase().as_str(),
                        "ini"
                            | "cfg"
                            | "conf"
                            | "config"
                            | "json"
                            | "toml"
                            | "yaml"
                            | "yml"
                            | "properties"
                    )
                })
    });
    for (index, entry) in configs.enumerate() {
        if index == MAX_RAW_CONFIGS {
            issues.push(issue("configuration_scan_limit", IssueSeverity::Warning, None,
                "The folder contains more settings files than this scan checks. The remaining files are unverified."));
            break;
        }
        let path = entry.relative_path.as_path();
        let text = if entry.size_bytes <= MAX_TEXT_BYTES as u64 {
            match scan.read_small(path, MAX_TEXT_BYTES) {
                Ok(bytes) => decode_text(&bytes),
                Err(_) => {
                    issues.push(issue(
                        "configuration_read_failed",
                        IssueSeverity::Error,
                        Some(path),
                        "This settings file could not be read safely. Scan the folder again after checking the file.",
                    ));
                    continue;
                }
            }
        } else {
            None
        };
        let Some(text) = text else {
            issues.push(issue("configuration_unverified", IssueSeverity::Warning, Some(path),
                "The settings file could not be inspected within the supported size and text limits."));
            continue;
        };
        let (absolute, parent) = config_path_hints(&text);
        if absolute {
            issues.push(issue("absolute_configuration_path", IssueSeverity::Warning, Some(path),
                "This settings file may refer to a fixed drive or network share. Review whether the app needs that location."));
        }
        if parent {
            issues.push(issue("parent_configuration_path", IssueSeverity::Warning, Some(path),
                "This settings file may refer to a parent folder. Review where the path resolves after moving the app."));
        }
    }
}

fn is_separator(byte: &u8) -> bool {
    matches!(byte, b'\\' | b'/')
}

fn config_path_hints(text: &str) -> (bool, bool) {
    let mut absolute = false;
    let mut parent = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with(['#', ';']) {
            continue;
        }
        let bytes = line.as_bytes();
        for (index, tail) in (0..bytes.len()).map(|i| (i, &bytes[i..])) {
            let previous = index.checked_sub(1).map(|i| bytes[i]);
            let boundary = previous.is_none_or(|byte| {
                !byte.is_ascii_alphanumeric() && !matches!(byte, b'_' | b'.' | b'-')
            });
            if !boundary {
                continue;
            }
            absolute |= matches!(tail, [drive, b':', sep, ..] if drive.is_ascii_alphabetic() && is_separator(sep))
                || (previous.is_none_or(|byte| !matches!(byte, b':' | b'\\' | b'/'))
                    && network_path(tail));
            parent |= tail.strip_prefix(b"..").is_some_and(|rest| {
                rest.first().is_none_or(|end| {
                    is_separator(end)
                        || end.is_ascii_whitespace()
                        || matches!(end, b'"' | b'\'' | b',' | b';' | b']' | b'}' | b')')
                })
            });
        }
        if absolute && parent {
            break;
        }
    }
    (absolute, parent)
}

fn network_path(tail: &[u8]) -> bool {
    if !tail.starts_with(b"\\\\") && !tail.starts_with(b"//") {
        return false;
    }
    let mut host = tail.iter().skip_while(|byte| is_separator(byte)).copied();
    let host_len = host
        .clone()
        .take_while(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'?')
        })
        .count();
    host_len > 0 && host.nth(host_len).is_some_and(|byte| is_separator(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const APPINFO_FIXTURE: &str = "[Format]\nType=PortableApps.comFormat\nVersion=3.9\n\
        [Details]\nName=Sample Portable\nAppID=SamplePortable\nPublisher=Sample\nHomepage=https://example.test\n\
        Category=Utilities\nDescription=Fixture application\nLanguage=English\n\
        [License]\nShareable=false\nOpenSource=false\nFreeware=false\nCommercialUse=false\n\
        [Version]\nPackageVersion=1.0.0.0\nDisplayVersion=1.0\n\
        [Control]\nIcons=1\nStart=SamplePortable.exe\n";

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap();
            let root = std::env::temp_dir().join(format!(
                "pf-validation-{}-{}",
                std::process::id(),
                now.as_nanos()
            ));
            fs::create_dir(&root).unwrap();
            Self(root)
        }

        fn write(&self, relative: &str, bytes: impl AsRef<[u8]>) {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }

        fn exe(&self, relative: &str) {
            let mut header = [0; 240];
            header[..2].copy_from_slice(b"MZ");
            header[60..64].copy_from_slice(&64_u32.to_le_bytes());
            header[64..68].copy_from_slice(b"PE\0\0");
            header[68..70].copy_from_slice(&0x8664_u16.to_le_bytes());
            header[70..72].copy_from_slice(&1_u16.to_le_bytes());
            header[84..86].copy_from_slice(&112_u16.to_le_bytes());
            header[86..88].copy_from_slice(&2_u16.to_le_bytes());
            header[88..90].copy_from_slice(&0x20b_u16.to_le_bytes());
            header[156..158].copy_from_slice(&2_u16.to_le_bytes());
            self.write(relative, header);
        }

        fn scan(&self) -> FolderScan {
            super::super::scan(&self.0).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn prepared_validation_uses_start_and_checks_the_nested_launcher_target() {
        let fixture = Fixture::new();
        fixture.write(APPINFO, APPINFO_FIXTURE);
        fixture.exe("SamplePortable.exe");
        fixture.exe("App/Vendor/bin/Tool.exe");
        let launcher = "App/AppInfo/Launcher/SamplePortable.ini";
        fixture.write(
            launcher,
            "[Launch]\nProgramExecutable=Vendor\\bin\\Tool.exe\n",
        );
        fixture.write(INSTALLER, "[Source]\nIncludeInstallerSource=true\n[OptionalComponents]\nOptionalComponents=true\n");
        let scan = fixture.scan();
        let issues = validate(&scan, None);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(issues[0].code, "installer_settings_unverified");
        assert!(validate(&scan, Some(Path::new("App/Vendor/bin/Tool.exe")))
            .iter()
            .any(|issue| issue.code == "selection_differs_from_start"));

        fixture.write(
            launcher,
            "[Launch]\nProgramExecutable=..\\SamplePortable.exe\n",
        );
        assert!(validate(&fixture.scan(), None)
            .iter()
            .any(|issue| issue.code == "invalid_launcher_target"
                && issue.severity == IssueSeverity::Error));
    }

    #[test]
    fn raw_validation_blocks_an_incomplete_scan_or_unrecognized_selection() {
        let fixture = Fixture::new();
        fixture.exe("bin/Tool.exe");
        fixture.write(
            "settings.ini",
            "path=C:\\private-folder\nfallback=../other\n",
        );
        let mut scan = fixture.scan();
        let valid = validate(&scan, Some(Path::new("bin/Tool.exe")));
        assert_eq!(
            valid.iter().map(|issue| issue.code).collect::<Vec<_>>(),
            ["absolute_configuration_path", "parent_configuration_path"]
        );
        assert!(valid
            .iter()
            .all(|issue| !issue.message.contains("private-folder")));
        assert!(validate(&scan, Some(Path::new("../Tool.exe")))
            .iter()
            .any(|issue| issue.code == "invalid_selection"));
        scan.complete = false;
        assert_eq!(validate(&scan, None)[0].code, "scan_incomplete");
    }

    #[test]
    fn unsupported_payload_paths_block_only_raw_builds() {
        let fixture = Fixture::new();
        fixture.exe("SamplePortable.exe");
        fixture.write("bin/.data/contents.dat", b"fixture");
        let selected = Path::new("SamplePortable.exe");
        let raw = fixture.scan();
        assert!(raw.complete);
        let findings = validate(&raw, Some(selected));
        let finding = findings
            .iter()
            .find(|issue| issue.code == "unsupported_payload_path")
            .unwrap();
        assert_eq!(finding.severity, IssueSeverity::Error);
        assert_eq!(finding.relative_path, Some(Path::new("bin").join(".data")));

        fixture.write(APPINFO, APPINFO_FIXTURE);
        let prepared = fixture.scan();
        assert!(prepared.is_prepared());
        assert!(!validate(&prepared, Some(selected))
            .iter()
            .any(|issue| issue.code == "unsupported_payload_path"));
    }

    #[test]
    fn parsed_ini_is_case_insensitive_and_rejects_ambiguous_settings() {
        let ini =
            parse_ini("; comment\r\n[Launch]\r\nProgramExecutable = Vendor\\bin\\tool.exe\r\n")
                .unwrap();
        assert_eq!(
            field(&ini, "launch", "programexecutable"),
            r"Vendor\bin\tool.exe"
        );
        assert!(parse_ini("[Launch]\nPath=a\npATH=b").is_err());
        assert!(parse_ini("[Launch]\nPath=a\n[lAUNCh]\nOther=b").is_err());
        assert!(parse_ini("Path=a\n[Launch]").is_err());
        assert!(parse_ini("[Launch\nPath=a").is_err());
    }

    #[test]
    fn nested_executable_paths_are_preserved_and_escapes_rejected() {
        let wanted = Path::new("Vendor").join("bin").join("Tool.exe");
        assert_eq!(relative_target(r"Vendor\bin\Tool.exe"), Some(wanted));
        for unsafe_path in [
            r"C:\Tool.exe",
            r"C:Tool.exe",
            r"\\server\Tool.exe",
            r"Vendor\..\Tool.exe",
            r"Vendor\Tool.exe:secret",
            r"Vendor\NUL.exe",
            r"%PAL:AppDir%\Tool.exe",
            "tool.exe --flag",
        ] {
            assert_eq!(relative_target(unsafe_path), None, "{unsafe_path}");
        }
    }

    #[test]
    fn raw_config_hints_recognize_windows_paths_without_url_false_positives() {
        assert_eq!(
            config_path_hints(r#"{"path":"D:\\private\\settings", "fallback":"..\\shared"}"#),
            (true, true)
        );
        assert_eq!(
            config_path_hints(r"path=\\workstation\share\config.ini"),
            (true, false)
        );
        assert_eq!(
            config_path_hints("path=//workstation/share/settings"),
            (true, false)
        );
        assert_eq!(config_path_hints("path=folder/../settings"), (false, true));
        assert_eq!(config_path_hints("path=.."), (false, true));
        assert_eq!(config_path_hints("path=folder/.."), (false, true));
        assert_eq!(config_path_hints("path=filename.."), (false, false));
        assert_eq!(
            config_path_hints("url=https://example.com/docs\n; ignored=C:/data\nfile=ab..txt"),
            (false, false)
        );
    }

    #[test]
    fn text_read_supports_windows_boms_and_refuses_binary() {
        assert_eq!(decode_text(&[0xff, 0xfe, b'X', 0]), Some("X".into()));
        assert_eq!(decode_text(&[0xfe, 0xff, 0, b'X']), Some("X".into()));
        assert_eq!(decode_text(&[0xef, 0xbb, 0xbf, b'X']), Some("X".into()));
        assert_eq!(decode_text(&[0xff, 0xfe, b'X']), None);
        assert_eq!(decode_text(b"X\x00Y"), None);
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_use_ordinal_case_without_expanding_aliases() {
        assert!(same_path(
            Path::new("Vendor/Äpp.exe"),
            Path::new("vendor/äpp.EXE")
        ));
        assert!(!same_path(
            Path::new("Vendor/ß.exe"),
            Path::new("Vendor/SS.exe")
        ));
    }
}
