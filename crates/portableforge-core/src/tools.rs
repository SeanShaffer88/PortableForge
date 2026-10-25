//! Pinned external tools (Slice 5).
//!
//! PortableForge orchestrates two official PortableApps.com programs —
//! the Launcher Generator and the Installer Generator — plus the
//! official Application Template used as a syntax reference. They are
//! **not** PortableForge code and are never reimplemented; they are
//! pinned here by exact version, size, and SHA-256, measured from the
//! real downloads on 2026-10-23 (see `tools-pins/PINS.md`).
//!
//! The Sandboxie-Plus installer pin (added 2026-10-25) is different
//! in kind: Sandboxie is a whole program of its own that PortableForge
//! *bundles unmodified* (its license expressly permits redistributing
//! the original unmodified binaries free of charge; the license texts
//! ship in `third-party/`) and offers to run when a computer has no
//! test box of either kind. The pin was measured from the official
//! download the same way as the others.
//!
//! A tool may only be used after [`verify_tool_file`] passes. A missing
//! or mismatched file is a plain-language error, never a silent run.

use std::path::{Path, PathBuf};

use crate::snapshot::sha256_hex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolId {
    LauncherGenerator,
    InstallerGenerator,
    AppTemplate,
    /// The bundled, unmodified Sandboxie-Plus installer (Mode A2's
    /// way in on a computer that has no test box yet).
    SandboxieInstaller,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolPin {
    pub id: ToolId,
    /// What this tool is called in plain words.
    pub plain_name: &'static str,
    pub version: &'static str,
    pub filename: &'static str,
    pub download_url: &'static str,
    pub size_bytes: u64,
    pub sha256: &'static str,
    /// Executable to run once the tool itself is installed, and what it
    /// receives (the package folder). Empty for the template (data only).
    pub generator_exe: &'static str,
}

pub const TOOL_PINS: &[ToolPin] = &[
    ToolPin {
        id: ToolId::LauncherGenerator,
        plain_name: "PortableApps.com starter maker",
        version: "2.2.2.1",
        filename: "PortableApps.comLauncher_2.2.2.1.paf.exe",
        download_url: "https://downloads.sourceforge.net/project/portableapps/PortableApps.com%20Launcher/PortableApps.comLauncher_2.2.2.1.paf.exe",
        size_bytes: 2_719_624,
        sha256: "f12bf700c931bfbe21ea6751dececc681a4d2cdb9a0940bc100e3ff38edf54e0",
        generator_exe: "PortableApps.comLauncher.exe",
    },
    ToolPin {
        id: ToolId::InstallerGenerator,
        plain_name: "PortableApps.com setup maker",
        version: "3.9.22",
        filename: "PortableApps.comInstaller_3.9.22.paf.exe",
        download_url: "https://downloads.sourceforge.net/project/portableapps/PortableApps.com%20Installer/PortableApps.comInstaller_3.9.22.paf.exe",
        size_bytes: 6_425_848,
        sha256: "689456f1fb1320c45e82a077afe6b87fb388fd3f75c41d9ee9ce8d45f3966b67",
        generator_exe: "PortableApps.comInstaller.exe",
    },
    ToolPin {
        id: ToolId::AppTemplate,
        plain_name: "PortableApps.com example package",
        version: "3.5.2",
        filename: "PortableApps.com_Application_Template_3.5.2.zip",
        download_url: "https://downloads.sourceforge.net/portableapps/PortableApps.com_Application_Template_3.5.2.zip",
        size_bytes: 341_702,
        sha256: "f432bf127820aa36d160c5e83e75df2d87ea3bda25bfc3c36da18f94c76ffa00",
        generator_exe: "",
    },
    ToolPin {
        id: ToolId::SandboxieInstaller,
        plain_name: "Sandboxie quick box installer",
        version: "1.18.8",
        filename: "Sandboxie-Plus-x64-v1.18.8.exe",
        download_url: "https://github.com/sandboxie-plus/Sandboxie/releases/download/v1.18.8/Sandboxie-Plus-x64-v1.18.8.exe",
        size_bytes: 24_930_680,
        sha256: "f27078ff74a07906fd51476079e1a7a8f5e20c8bb75a9d2ae86169d47179ae76",
        generator_exe: "",
    },
];

pub fn pin(id: ToolId) -> &'static ToolPin {
    TOOL_PINS
        .iter()
        .find(|p| p.id == id)
        .expect("all ids pinned")
}

/// Verify a downloaded tool file against its pin: exact size, SHA-256,
/// and the expected file signature (`MZ` for executables, `PK` for the
/// template zip). Any mismatch is an error with a plain explanation.
pub fn verify_tool_file(path: &Path, pin: &ToolPin) -> Result<(), String> {
    let bytes = std::fs::read(path)
        .map_err(|e| format!("We could not read the {} file: {e}", pin.plain_name))?;
    if bytes.len() as u64 != pin.size_bytes {
        return Err(format!(
            "The {} file is the wrong size ({} bytes, expected {}). It may be damaged or not the real thing — download it again.",
            pin.plain_name,
            bytes.len(),
            pin.size_bytes
        ));
    }
    let magic_ok = if pin.filename.ends_with(".zip") {
        bytes.starts_with(b"PK")
    } else {
        bytes.starts_with(b"MZ")
    };
    if !magic_ok {
        return Err(format!(
            "The {} file does not look like the right kind of file at all.",
            pin.plain_name
        ));
    }
    let actual = sha256_hex(&bytes);
    if actual != pin.sha256 {
        return Err(format!(
            "The {} file's fingerprint does not match the one we recorded. It may have been changed — we will not use it.",
            pin.plain_name
        ));
    }
    Ok(())
}

/// Find a *verified* tool in the cache folder. Returns the path only
/// when the file present matches the pin exactly.
pub fn find_verified_tool(cache_dir: &Path, pin: &ToolPin) -> Option<PathBuf> {
    let candidate = cache_dir.join(pin.filename);
    if candidate.is_file() && verify_tool_file(&candidate, pin).is_ok() {
        Some(candidate)
    } else {
        None
    }
}

/// The build invocation for a generator: the generator program with the
/// package folder as its one argument (the documented way both official
/// generators work). Execution itself happens in the host program on
/// Windows; this plan is what it runs, and only after verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildInvocation {
    pub program: String,
    pub args: Vec<String>,
}

pub fn generator_invocation(pin: &ToolPin, package_folder: &Path) -> BuildInvocation {
    BuildInvocation {
        program: pin.generator_exe.to_string(),
        args: vec![package_folder.to_string_lossy().into_owned()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_are_internally_consistent() {
        for p in TOOL_PINS {
            assert_eq!(p.sha256.len(), 64);
            assert!(p.sha256.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(p.download_url.starts_with("https://"));
            assert!(p.size_bytes > 0);
        }
        assert_eq!(pin(ToolId::InstallerGenerator).version, "3.9.22");
        assert_eq!(pin(ToolId::LauncherGenerator).version, "2.2.2.1");
    }

    #[test]
    fn verification_rejects_wrong_size_and_hash() {
        let dir = std::env::temp_dir().join(format!("pf-tools-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join(pin(ToolId::AppTemplate).filename);
        std::fs::write(&fake, b"PK-not-the-real-template").unwrap();
        let err = verify_tool_file(&fake, pin(ToolId::AppTemplate)).unwrap_err();
        assert!(err.contains("wrong size"), "{err}");
        assert!(find_verified_tool(&dir, pin(ToolId::AppTemplate)).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invocation_is_program_plus_package_folder() {
        let inv = generator_invocation(
            pin(ToolId::LauncherGenerator),
            Path::new(r"C:\PortableApps\WidgetPortable"),
        );
        assert_eq!(inv.program, "PortableApps.comLauncher.exe");
        assert_eq!(
            inv.args,
            vec![r"C:\PortableApps\WidgetPortable".to_string()]
        );
    }
}
