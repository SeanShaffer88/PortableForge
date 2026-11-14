//! Redirect environment-based AppData lookups for a portable app's process.

use serde::{Deserialize, Serialize};

#[cfg(all(windows, target_arch = "x86_64"))]
use std::fmt::Write as _;

/// The AppData environment variables the launcher can override in the app process.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppDataScope {
    Roaming,
    Local,
}

impl AppDataScope {
    /// Windows path relative to the package's Data directory.
    pub fn data_folder(self) -> &'static str {
        match self {
            Self::Roaming => r"AppData\Roaming",
            Self::Local => r"AppData\Local",
        }
    }

    #[cfg(all(windows, target_arch = "x86_64"))]
    fn variable(self) -> &'static str {
        match self {
            Self::Roaming => "APPDATA",
            Self::Local => "LOCALAPPDATA",
        }
    }
}

/// Check saved and in-memory selections before writing launcher settings.
pub fn validate_appdata_rules(scopes: &[AppDataScope]) -> Result<(), String> {
    for scope in [AppDataScope::Roaming, AppDataScope::Local] {
        if scopes.iter().filter(|value| **value == scope).count() > 1 {
            return Err("Select each AppData location only once.".into());
        }
    }
    Ok(())
}

#[cfg(all(windows, target_arch = "x86_64"))]
pub(super) fn append_launcher_rules(ini: &mut String, scopes: &[AppDataScope]) {
    if scopes.is_empty() {
        return;
    }
    ini.push_str("\n[Environment]\n");
    for scope in scopes {
        writeln!(
            ini,
            "{}=%PAL:DataDir%\\{}",
            scope.variable(),
            scope.data_folder()
        )
        .expect("writing to a String cannot fail");
    }
}

/// Create selected directories on every launch, including after upgrades.
#[cfg(all(windows, target_arch = "x86_64"))]
pub(super) fn launcher_directory_setup(scopes: &[AppDataScope]) -> Option<String> {
    if scopes.is_empty() {
        return None;
    }
    let mut source = String::from(
        "; PortableForge creates these folders before launching the application.\n${SegmentFile}\n\n${SegmentPreExec}\n",
    );
    for scope in scopes {
        writeln!(
            source,
            "  ClearErrors\n  CreateDirectory \"$DataDirectory\\{}\"\n  ${{If}} ${{Errors}}\n    MessageBox MB_OK|MB_ICONSTOP \"PortableForge could not create the portable AppData folder. Check that the package folder is writable.\"\n    Abort\n  ${{EndIf}}",
            scope.data_folder()
        )
        .expect("writing to a String cannot fail");
    }
    source.push_str("!macroend\n");
    Some(source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_location_may_be_selected_once() {
        use AppDataScope::{Local, Roaming};
        assert!(validate_appdata_rules(&[]).is_ok());
        assert!(validate_appdata_rules(&[Local, Roaming]).is_ok());
        assert!(validate_appdata_rules(&[Local, Roaming, Local]).is_err());
        assert!(validate_appdata_rules(&[Roaming, Roaming]).is_err());
    }

    #[cfg(all(windows, target_arch = "x86_64"))]
    #[test]
    fn launcher_uses_process_environment_and_skips_empty_configuration() {
        let mut ini = "[Launch]\n".to_owned();
        let unmodified = ini.clone();
        append_launcher_rules(&mut ini, &[]);
        assert_eq!(ini, unmodified);
        append_launcher_rules(&mut ini, &[AppDataScope::Roaming, AppDataScope::Local]);
        assert_eq!(
            ini,
            format!(
                "{unmodified}\n[Environment]\nAPPDATA=%PAL:DataDir%\\AppData\\Roaming\nLOCALAPPDATA=%PAL:DataDir%\\AppData\\Local\n"
            )
        );
        assert!(launcher_directory_setup(&[]).is_none());
    }
}
