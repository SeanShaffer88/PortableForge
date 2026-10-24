//! Validation against the PortableApps.com Format 3.9 rules and the
//! PortableForge guardrails. Errors block a build; warnings don't.

use std::path::Path;

use crate::paf::{valid_app_id, valid_package_version, PafMetadata, CATEGORIES};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationIssue {
    pub error: bool,
    pub field: String,
    pub message: String,
}

impl ValidationIssue {
    fn err(field: &str, message: impl Into<String>) -> Self {
        Self {
            error: true,
            field: field.into(),
            message: message.into(),
        }
    }
    fn warn(field: &str, message: impl Into<String>) -> Self {
        Self {
            error: false,
            field: field.into(),
            message: message.into(),
        }
    }
}

pub fn validate_metadata(meta: &PafMetadata) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    if meta.name.trim().is_empty() {
        issues.push(ValidationIssue::err("Name", "Name must not be empty."));
    }
    if meta.base_app_name.trim().is_empty() {
        issues.push(ValidationIssue::err(
            "BaseAppName",
            "The program's own folder name must not be empty — the package is built around it.",
        ));
    }
    if meta.publisher.trim().is_empty() {
        issues.push(ValidationIssue::warn(
            "Publisher",
            "Publisher is empty — the finished package shows who made the program.",
        ));
    }
    if meta.homepage.trim().is_empty() {
        issues.push(ValidationIssue::warn(
            "Homepage",
            "Homepage is empty — fine for a private copy, but a shared package should say where the program comes from.",
        ));
    }
    if !valid_app_id(&meta.app_id) {
        issues.push(ValidationIssue::err(
            "AppID",
            "AppID may contain only letters, numbers, periods, dashes, plus and underscores.",
        ));
    }
    if !CATEGORIES.contains(&meta.category.as_str()) {
        issues.push(ValidationIssue::err(
            "Category",
            format!(
                "Category must be one of the PortableApps.com Format categories: {}",
                CATEGORIES.join(", ")
            ),
        ));
    }
    let desc_len = meta.description.chars().count();
    if desc_len > 512 {
        issues.push(ValidationIssue::err(
            "Description",
            "Description must be at most 512 characters (Format 3.9).",
        ));
    } else if desc_len >= 480 {
        issues.push(ValidationIssue::warn(
            "Description",
            format!(
                "Description is {desc_len} of the 512 characters allowed — close to the limit."
            ),
        ));
    }
    if meta.description.trim().is_empty() {
        issues.push(ValidationIssue::warn(
            "Description",
            "Description is empty — the Platform shows this to users.",
        ));
    }
    if !valid_package_version(&meta.package_version) {
        issues.push(ValidationIssue::err(
            "PackageVersion",
            "PackageVersion must be in x.x.x.x numeric form.",
        ));
    }
    if meta.main_exe.trim().is_empty() || !meta.main_exe.to_ascii_lowercase().ends_with(".exe") {
        issues.push(ValidationIssue::err(
            "ProgramExecutable",
            "The main executable must be an .exe file name.",
        ));
    }
    if let Err(e) = meta.distribution.check_guardrail() {
        issues.push(ValidationIssue::err("Distribution", e));
    }
    issues
}

/// Sections the official Application Template 3.5.2's Launcher INI uses
/// (plus the numbered `[FileWriteN]` family PortableForge generates).
const LAUNCHER_SECTIONS: &[&str] = &[
    "Launch",
    "Activate",
    "LiveMode",
    "Environment",
    "DirectoriesMove",
    "FilesMove",
    "RegistryKeys",
];

/// Key allow-list for the two sections whose key sets are closed in the
/// template: `[Activate]` and `[FileWriteN]`. ([Launch] legitimately
/// carries many optional keys, so it is not key-checked.)
fn launcher_key_allowed(section: &str, key: &str) -> Option<bool> {
    match section {
        "Activate" => Some(matches!(key, "Registry" | "XML")),
        _ if section.starts_with("FileWrite") => {
            Some(matches!(key, "Type" | "File" | "Find" | "Replace"))
        }
        _ => None,
    }
}

/// Spelling-level check of a Launcher INI against the template's shape:
/// unknown sections and unknown keys in the closed sections come back
/// as warnings — they are how a typo'd key silently does nothing.
pub fn check_launcher_ini(text: &str) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    let mut section = String::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].to_string();
            let is_filewrite = section.starts_with("FileWrite")
                && section[9..].chars().all(|c| c.is_ascii_digit())
                && section.len() > 9;
            if !LAUNCHER_SECTIONS.contains(&section.as_str()) && !is_filewrite {
                issues.push(ValidationIssue::warn(
                    "Launcher",
                    format!(
                        "Section [{section}] is not one the example package uses — check the spelling; a wrong section name is ignored silently."
                    ),
                ));
            }
            continue;
        }
        if let Some((key, _)) = line.split_once('=') {
            let key = key.trim();
            if let Some(allowed) = launcher_key_allowed(&section, key) {
                if !allowed {
                    issues.push(ValidationIssue::warn(
                        "Launcher",
                        format!(
                            "Key {key} in [{section}] is not one the example package uses — check the spelling; a wrong key is ignored silently."
                        ),
                    ));
                }
            }
        }
    }
    issues
}

/// Check a generated package tree on disk for the required Format 3.9
/// pieces. Paths are checked relative to `root` (the `<AppID>` folder).
pub fn validate_tree(root: &Path, meta: &PafMetadata) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    let required: Vec<(String, &str)> = vec![
        (
            r"App\AppInfo\appinfo.ini".to_string(),
            "appinfo.ini is required (App\\AppInfo).",
        ),
        (
            format!(r"App\AppInfo\Launcher\{}.ini", meta.app_id),
            "The Launcher .ini is required (App\\AppInfo\\Launcher).",
        ),
        (
            "help.html".to_string(),
            "help.html is required in the package root.",
        ),
        (
            format!(r"App\{}\{}", meta.base_app_name, meta.main_exe),
            "The main executable is missing from App\\.",
        ),
    ];
    for (rel, msg) in required {
        // Accept either separator style on the host running the check.
        let native = rel.replace('\\', std::path::MAIN_SEPARATOR_STR);
        if !root.join(&native).exists() {
            issues.push(ValidationIssue::err(
                "Tree",
                format!("{msg} Missing: {rel}"),
            ));
        }
    }
    if !root.join("Data").is_dir() {
        issues.push(ValidationIssue::warn(
            "Tree",
            "Data directory missing — the installer creates it, but shipping it empty is clearer.",
        ));
    }
    // When the Launcher INI is there, spell-check it against the
    // template's sections and closed key sets.
    let launcher_rel = format!(r"App\AppInfo\Launcher\{}.ini", meta.app_id)
        .replace('\\', std::path::MAIN_SEPARATOR_STR);
    if let Ok(text) = std::fs::read_to_string(root.join(&launcher_rel)) {
        issues.extend(check_launcher_ini(&text));
    }
    issues
}

pub fn has_errors(issues: &[ValidationIssue]) -> bool {
    issues.iter().any(|i| i.error)
}

/// Generate the package tree (directories + text files) from metadata and
/// accepted classifications. Harvested binaries are copied by the caller
/// (the agent's harvest step) into `App\<BaseAppName>\`; this function
/// writes everything PortableForge itself authors, and refuses to write
/// a tree whose metadata has validation errors.
pub fn generate_tree(
    root: &Path,
    meta: &PafMetadata,
    accepted: &[crate::classify::Classification],
) -> Result<Vec<String>, String> {
    let issues = validate_metadata(meta);
    if has_errors(&issues) {
        let msgs: Vec<String> = issues
            .iter()
            .filter(|i| i.error)
            .map(|i| format!("{}: {}", i.field, i.message))
            .collect();
        return Err(format!("Refusing to generate: {}", msgs.join("; ")));
    }
    let mut written: Vec<String> = Vec::new();
    let mut write = |rel: &str, content: &str| -> Result<(), String> {
        let native = rel.replace('\\', std::path::MAIN_SEPARATOR_STR);
        let path = root.join(&native);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        std::fs::write(&path, content).map_err(|e| format!("cannot write {rel}: {e}"))?;
        written.push(rel.to_string());
        Ok(())
    };
    std::fs::create_dir_all(root).map_err(|e| format!("cannot create package root: {e}"))?;
    write(r"App\AppInfo\appinfo.ini", &meta.appinfo_ini())?;
    write(
        r"App\AppInfo\installer.ini",
        &meta.installer_ini_with(accepted),
    )?;
    write(
        &format!(r"App\AppInfo\Launcher\{}.ini", meta.app_id),
        &meta.launcher_ini(accepted),
    )?;
    write("help.html", &meta.help_html())?;
    // Data ships empty; DefaultData and Source get readme markers so the
    // layout is self-explanatory.
    std::fs::create_dir_all(root.join("Data")).map_err(|e| e.to_string())?;
    write(
        r"App\DefaultData\README.txt",
        "First-run default settings harvested by PortableForge are placed here.\n\
         The launcher copies them into Data\\ on first run when Data\\ is empty.\n",
    )?;
    let dist_note = match &meta.distribution {
        crate::paf::Distribution::Bundled {
            redistribution_asserted,
        } => format!(
            "Distribution: bundled files. Redistribution rights asserted by the packager: {redistribution_asserted}."
        ),
        crate::paf::Distribution::Online {
            download_url,
            download_sha256,
            ..
        } => format!(
            "Distribution: online installer. Source: {download_url} (SHA-256 {download_sha256})."
        ),
    };
    write(
        r"Other\Source\PortableForge.txt",
        &format!(
            "Packaged with PortableForge.\n{dist_note}\n\
             The PortableApps.com Launcher and Installer are separate programs \
             used to build this package; their licenses are in their own distributions.\n"
        ),
    )?;
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paf::Distribution;

    fn meta() -> PafMetadata {
        crate::paf::tests::sample(Distribution::Bundled {
            redistribution_asserted: true,
        })
    }

    #[test]
    fn bad_metadata_is_flagged() {
        let mut m = meta();
        m.category = "Tools".into();
        m.package_version = "1.2".into();
        m.description = "x".repeat(600);
        let issues = validate_metadata(&m);
        assert!(has_errors(&issues));
        assert_eq!(issues.iter().filter(|i| i.error).count(), 3);
    }

    #[test]
    fn launcher_ini_spelling_check_catches_typos_only() {
        let m = meta();
        // Our own generated INI passes clean.
        let generated = m.launcher_ini(&[]);
        assert!(check_launcher_ini(&generated).is_empty(), "{generated}");
        // A typo'd key, a made-up section, and a wrong FileWrite key warn.
        let corrupt = "[Launch]\nProgramExecutable=Widget\\widget.exe\n\n[Bogus]\nX=1\n\n[Activate]\nRegistery=true\n\n[FileWrite1]\nType=Replace\nFile=a\nFind=b\nReplac=b\n";
        let issues = check_launcher_ini(corrupt);
        assert_eq!(issues.len(), 3, "{issues:?}");
        assert!(issues.iter().all(|i| !i.error));
        // A long-but-legal description earns the plain near-limit warning.
        let mut m2 = meta();
        m2.description = "x".repeat(500);
        let issues = validate_metadata(&m2);
        assert!(!has_errors(&issues));
        assert!(issues.iter().any(|i| i.message.contains("500 of the 512")));
    }

    #[test]
    fn unasserted_bundling_cannot_generate() {
        let mut m = meta();
        m.distribution = Distribution::Bundled {
            redistribution_asserted: false,
        };
        let dir = std::env::temp_dir().join(format!("pf-gen-test-{}", std::process::id()));
        let result = generate_tree(&dir, &m, &[]);
        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn generate_then_validate_tree() {
        let m = meta();
        let dir = std::env::temp_dir().join(format!("pf-gen-ok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let written = generate_tree(&dir, &m, &[]).unwrap();
        assert!(written.iter().any(|w| w.contains("appinfo.ini")));
        // Tree check: everything present except the harvested main exe.
        let issues = validate_tree(&dir, &m);
        assert_eq!(issues.iter().filter(|i| i.error).count(), 1);
        assert!(issues[0].message.contains("main executable"));
        // Drop a stand-in exe in place; tree is now clean.
        let exe_dir = dir.join("App").join("Widget");
        std::fs::create_dir_all(&exe_dir).unwrap();
        std::fs::write(exe_dir.join("widget.exe"), b"MZ").unwrap();
        let issues = validate_tree(&dir, &m);
        assert!(!has_errors(&issues), "{issues:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
