//! Input rules for compiling a trusted, prepared PortableApps.com package.

use std::collections::BTreeMap;

const APPINFO_FIELDS: &[(&str, &[&str])] = &[
    ("format", &["type", "version"]),
    (
        "details",
        &[
            "name",
            "appid",
            "baseappname",
            "publisher",
            "homepage",
            "category",
            "description",
            "language",
            "trademarks",
        ],
    ),
    (
        "license",
        &[
            "shareable",
            "opensource",
            "freeware",
            "commercialuse",
            "eulaversion",
        ],
    ),
    ("version", &["packageversion", "displayversion"]),
    ("dependencies", &["requiresadmin", "requires64bitos"]),
    ("control", &["icons", "start", "extracticon"]),
];
const INSTALLER_FIELDS: &[(&str, &[&str])] = &[
    ("source", &["includeinstallersource"]),
    ("checkrunning", &["closeexe", "closename"]),
];

#[derive(Debug)]
struct Package {
    app_id: String,
    base_app_name: String,
    launcher: String,
    extract_icon: Option<String>,
    appinfo: String,
}

type Ini = BTreeMap<(String, String), String>;

fn read_ini(text: &str, allowed: &[(&str, &[&str])]) -> Result<Ini, String> {
    let mut fields = BTreeMap::new();
    let mut section = String::new();
    for raw in text.trim_start_matches('\u{feff}').lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = name.to_ascii_lowercase();
            if !allowed.iter().any(|(name, _)| *name == section) {
                return Err(format!(
                    "Unsupported section [{name}] in the package settings."
                ));
            }
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or("Expected a name=value entry in the package settings.")?;
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        if !allowed
            .iter()
            .any(|(name, keys)| *name == section && keys.contains(&key.as_str()))
        {
            return Err(format!("Unsupported setting [{section}] {key}."));
        }
        // The official builder interpolates values directly into NSIS source.
        if value
            .chars()
            .any(|c| !c.is_ascii() || c.is_control() || matches!(c, '$' | '"' | '\'' | '`'))
        {
            return Err(format!("Unsafe characters in [{section}] {key}."));
        }
        if fields
            .insert((section.clone(), key.clone()), value.to_owned())
            .is_some()
        {
            return Err(format!("Duplicate setting [{section}] {key}."));
        }
    }
    Ok(fields)
}

fn value<'a>(fields: &'a Ini, section: &str, key: &str) -> &'a str {
    fields
        .get(&(section.into(), key.into()))
        .map_or("", String::as_str)
}

pub(crate) fn safe_name(name: &str) -> bool {
    if name.is_empty()
        || name.starts_with([' ', '.'])
        || name.ends_with([' ', '.'])
        || name.chars().any(|c| {
            matches!(c, '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*') || c.is_control()
        })
    {
        return false;
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_ascii_uppercase();
    !matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) && !["COM", "LPT"].iter().any(|prefix| {
        stem.strip_prefix(prefix).is_some_and(|suffix| {
            matches!(
                suffix,
                "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    })
}

fn exe_name(name: &str) -> bool {
    safe_name(name) && name.to_ascii_lowercase().ends_with(".exe")
}

fn package_settings(text: &str) -> Result<Package, String> {
    let fields = read_ini(text, APPINFO_FIELDS)?;
    let field = |section, key| value(&fields, section, key);
    if !field("format", "type").eq_ignore_ascii_case("PortableApps.comFormat")
        || field("format", "version") != "3.9"
    {
        return Err("This command requires a PortableApps.com Format 3.9 package.".into());
    }
    let app_id = field("details", "appid");
    let launcher = field("control", "start");
    if !crate::paf::valid_app_id(app_id)
        || !safe_name(app_id)
        || !exe_name(launcher)
        || !launcher.eq_ignore_ascii_case(&format!("{app_id}.exe"))
    {
        return Err(
            "AppID must be a safe folder name and Start must name its root launcher.".into(),
        );
    }
    if field("details", "name").is_empty()
        || field("version", "displayversion").is_empty()
        || !crate::paf::valid_package_version(field("version", "packageversion"))
        || !field("version", "packageversion")
            .split('.')
            .all(|n| n.parse::<u16>().is_ok())
        || !crate::paf::CATEGORIES.contains(&field("details", "category"))
    {
        return Err(
            "Fill in Name, Category, PackageVersion and DisplayVersion in appinfo.ini.".into(),
        );
    }
    if !matches!(field("details", "language"), "English" | "Multilingual") {
        return Err("This command supports English and Multilingual packages.".into());
    }
    for key in ["shareable", "opensource", "freeware", "commercialuse"] {
        if !matches!(
            field("license", key).to_ascii_lowercase().as_str(),
            "true" | "false"
        ) {
            return Err(format!("Set [License] {key} to true or false."));
        }
    }
    if !field("license", "shareable").eq_ignore_ascii_case("true") {
        return Err(
            "This command requires [License] Shareable=true and your redistribution assertion."
                .into(),
        );
    }
    let icon = field("control", "extracticon");
    if !icon.is_empty()
        && (!icon.to_ascii_lowercase().starts_with("app\\")
            || !icon.split('\\').all(safe_name)
            || !icon.to_ascii_lowercase().ends_with(".exe"))
    {
        return Err("ExtractIcon must name a package-local executable under App.".into());
    }
    Ok(Package {
        app_id: app_id.into(),
        base_app_name: field("details", "baseappname").into(),
        launcher: launcher.into(),
        extract_icon: (!icon.is_empty()).then(|| icon.into()),
        appinfo: render_appinfo(&fields),
    })
}

fn render_appinfo(fields: &Ini) -> String {
    let mut output = String::new();
    let mut last_section = "";
    for ((section, key), value) in fields {
        if section != last_section {
            output.push_str(&format!("\r\n[{section}]\r\n"));
            last_section = section;
        }
        output.push_str(&format!("{key}={value}\r\n"));
    }
    output
}

fn installer_settings(text: &str) -> Result<String, String> {
    let fields = read_ini(text, INSTALLER_FIELDS)?;
    let include = value(&fields, "source", "includeinstallersource");
    if !matches!(include.to_ascii_lowercase().as_str(), "" | "true" | "false") {
        return Err("Set IncludeInstallerSource to true or false.".into());
    }
    let close_exe = value(&fields, "checkrunning", "closeexe");
    let close_name = value(&fields, "checkrunning", "closename");
    let mut output = String::from("[Source]\r\nIncludeInstallerSource=true\r\n");
    if !close_exe.is_empty() || !close_name.is_empty() {
        if !exe_name(close_exe) || close_name.is_empty() {
            return Err("CheckRunning needs a plain executable name and a display name.".into());
        }
        output.push_str(&format!(
            "\r\n[CheckRunning]\r\nCloseEXE={close_exe}\r\nCloseName={close_name}\r\n"
        ));
    }
    Ok(output)
}

#[cfg(windows)]
mod icons;

#[cfg(windows)]
pub(crate) mod windows;

#[cfg(windows)]
pub use windows::{compile_installer, CompileReport};

#[cfg(test)]
mod tests {
    use super::*;

    const APPINFO: &str = "\
[Format]
Type=PortableApps.comFormat
Version=3.9
[Details]
Name=Widget Portable
AppID=WidgetPortable
Language=English
Category=Utilities
[License]
Shareable=true
OpenSource=true
Freeware=true
CommercialUse=true
[Version]
PackageVersion=1.0.0.0
DisplayVersion=1.0
[Control]
Start=WidgetPortable.exe
ExtractIcon=App\\Widget\\widget.exe";

    #[test]
    fn package_metadata_accepts_local_paths() {
        let package = package_settings(APPINFO).unwrap();
        assert_eq!(package.app_id, "WidgetPortable");
        assert_eq!(package.launcher, "WidgetPortable.exe");
        assert_eq!(
            package.extract_icon.as_deref(),
            Some(r"App\Widget\widget.exe")
        );
        assert!(package.appinfo.contains("start=WidgetPortable.exe\r\n"));
    }

    #[test]
    fn package_metadata_rejects_paths_and_nsis_expansions() {
        for (original, changed) in [
            ("English", r"..\Custom"),
            ("English", "English`"),
            ("Widget Portable", "${INJECTED}"),
            ("WidgetPortable.exe", r"..\WidgetPortable.exe"),
            ("WidgetPortable.exe", "WidgetPortable.exe:stream"),
            (r"App\Widget\widget.exe", r"App\..\widget.exe"),
        ] {
            assert!(
                package_settings(&APPINFO.replace(original, changed)).is_err(),
                "{changed}"
            );
        }
        assert!(package_settings(&format!("{APPINFO}\n[SpecialPaths]\nPlugins=..")).is_err());
    }

    #[test]
    fn installer_metadata_allows_only_known_offline_rules() {
        let parsed = installer_settings("[Source]\nIncludeInstallerSource=false\n[CheckRunning]\nCloseEXE=widget.exe\nCloseName=Widget").unwrap();
        assert!(parsed.contains("IncludeInstallerSource=true"));
        assert!(parsed.contains("CloseEXE=widget.exe"));
        assert!(installer_settings("[DownloadFiles]\nDownloadURL=https://example.test").is_err());
        assert!(installer_settings("[OptionalComponents]\nOptionalFile1=..\\file").is_err());
        assert!(
            installer_settings("[CheckRunning]\nCloseEXE=..\\file.exe\nCloseName=File").is_err()
        );
        assert!(installer_settings(
            "[Source]\nIncludeInstallerSource=true\nincludeinstallersource=false"
        )
        .is_err());
    }
}
