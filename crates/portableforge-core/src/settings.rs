//! The wizard's own small memory: the setup files it has seen
//! lately, the chosen text size, and an unfinished run's draft, kept
//! in one JSON file in the person's home folder. Plain data only —
//! every decision still happens in the wizard or the other modules.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// How many setup files the pick step remembers.
pub const RECENT_LIMIT: usize = 5;

/// The text-size choices the wizard offers, as zoom factors.
pub const TEXT_SCALES: [f32; 3] = [1.0, 1.25, 1.5];

/// An unfinished run, saved as the person goes so closing the window
/// (or a crash, or the computer sleeping) never loses the run.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WizardDraft {
    /// The guide step id the person was on (`pick`, `name`, …).
    pub step: String,
    pub installer_path: String,
    pub app_name: String,
    pub description: String,
    pub publisher: String,
    #[serde(default)]
    pub homepage: String,
    pub main_exe: String,
    pub package_version: String,
    pub online_url: String,
    pub online_sha: String,
    /// The person vouched for the safe test box on the safety step
    /// ("use it anyway") — resumed runs keep their word.
    #[serde(default)]
    pub force_sandbox: bool,
    #[serde(default)]
    pub cleaned_capture: Option<CleanedCapture>,
    #[serde(default)]
    pub profile_update_base: Option<SavedProfileApproval>,
}

/// The separately stored identity of a capture that completed cleanup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanedCapture {
    pub folder: PathBuf,
    pub receipt_sha256: String,
}

impl WizardDraft {
    /// A draft is worth offering to resume only when the person had
    /// actually started — picked a file or begun naming.
    pub fn has_progress(&self) -> bool {
        !self.installer_path.trim().is_empty() || !self.app_name.trim().is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedProfileApproval {
    pub folder: PathBuf,
    pub label: String,
    pub manifest_sha256: String,
    #[serde(default)]
    pub details: ProfileDetails,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProfileDetails {
    pub version: String,
    pub setup_sha256: String,
    pub builds: Vec<SavedBuild>,
}

/// Independently remembered identity of a successful installer build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedBuild {
    pub directory: PathBuf,
    pub artifact: PathBuf,
    pub manifest_sha256: String,
    pub artifact_sha256: String,
    pub app_id: String,
    pub completed_utc: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WizardSettings {
    #[serde(default)]
    pub recent_installers: Vec<String>,
    #[serde(default = "default_text_scale")]
    pub text_scale: f32,
    #[serde(default)]
    pub appearance: Appearance,
    #[serde(default)]
    pub draft: Option<WizardDraft>,
    /// Whether the first-run welcome has been read and dismissed.
    #[serde(default)]
    pub welcomed: bool,
    /// Approvals are stored separately from the profile folders they cover.
    #[serde(default)]
    pub saved_profiles: Vec<SavedProfileApproval>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    #[default]
    System,
    Dark,
    Light,
}

fn default_text_scale() -> f32 {
    1.0
}

impl Default for WizardSettings {
    fn default() -> Self {
        Self {
            recent_installers: Vec::new(),
            text_scale: 1.0,
            appearance: Appearance::System,
            draft: None,
            welcomed: false,
            saved_profiles: Vec::new(),
        }
    }
}

impl WizardSettings {
    /// Remember an approval returned by a successful profile save.
    pub fn remember_profile(&mut self, approval: SavedProfileApproval) {
        self.saved_profiles
            .retain(|old| old.folder != approval.folder);
        self.saved_profiles.insert(0, approval);
    }

    /// Remember a setup file at the front of the list, without
    /// duplicates, keeping only the most recent [`RECENT_LIMIT`].
    pub fn remember_installer(&mut self, path: &str) {
        let path = path.trim();
        if path.is_empty() {
            return;
        }
        self.recent_installers.retain(|p| p != path);
        self.recent_installers.insert(0, path.to_string());
        self.recent_installers.truncate(RECENT_LIMIT);
    }

    /// The text scale, snapped to the nearest offered choice so a
    /// hand-edited file can never produce a wild zoom.
    pub fn text_scale(&self) -> f32 {
        TEXT_SCALES
            .iter()
            .copied()
            .min_by(|a, b| {
                (a - self.text_scale)
                    .abs()
                    .partial_cmp(&(b - self.text_scale).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(1.0)
    }
}

/// Where the settings file lives: `<home>\PortableForge\wizard-settings.json`.
pub fn settings_path() -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join("PortableForge")
        .join("wizard-settings.json")
}

/// Read the settings; anything missing or unreadable simply means
/// "first run" — defaults, never an error the person has to see.
pub fn load(path: &Path) -> WizardSettings {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Save draft and display settings, preserving approvals from disk.
pub fn save(path: &Path, settings: &WizardSettings) -> Result<(), String> {
    write_settings(path, settings, None).map(|_| ())
}

/// Add an approval explicitly, then refresh the caller's approval list.
pub fn save_approval(
    path: &Path,
    settings: &mut WizardSettings,
    approval: SavedProfileApproval,
) -> Result<(), String> {
    settings.saved_profiles =
        write_settings(path, settings, Some(LibraryUpdate::Approve(approval)))?;
    Ok(())
}

pub fn save_build(
    path: &Path,
    settings: &mut WizardSettings,
    approval: &SavedProfileApproval,
    build: SavedBuild,
) -> Result<(), String> {
    settings.saved_profiles =
        write_settings(path, settings, Some(LibraryUpdate::Built(approval, build)))?;
    Ok(())
}

pub fn relocate_approval(
    path: &Path,
    settings: &mut WizardSettings,
    approval: &SavedProfileApproval,
    destination: PathBuf,
) -> Result<(), String> {
    settings.saved_profiles = write_settings(
        path,
        settings,
        Some(LibraryUpdate::Relocate(approval, destination)),
    )?;
    Ok(())
}

enum LibraryUpdate<'a> {
    Approve(SavedProfileApproval),
    Built(&'a SavedProfileApproval, SavedBuild),
    Relocate(&'a SavedProfileApproval, PathBuf),
}

fn write_settings(
    path: &Path,
    settings: &WizardSettings,
    update: Option<LibraryUpdate<'_>>,
) -> Result<Vec<SavedProfileApproval>, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("We could not make {}: {e}", parent.display()))?;
    }
    // Hold the lock through the read, merge and atomic replacement.
    #[cfg(windows)]
    let _lock = {
        use std::os::windows::fs::OpenOptionsExt;

        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(path.with_extension("lock"))
            .map_err(|error| format!("We could not lock the wizard settings: {error}"))?
    };
    #[cfg(not(windows))]
    let _lock = {
        static SETTINGS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        SETTINGS_LOCK.lock().map_err(|error| error.to_string())?
    };
    let mut updated = settings.clone();
    updated.saved_profiles = match std::fs::read_to_string(path) {
        Ok(text) => {
            serde_json::from_str::<WizardSettings>(&text)
                .map_err(|error| format!("We could not read the wizard settings: {error}"))?
                .saved_profiles
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(format!("We could not read the wizard settings: {error}")),
    };
    match update {
        Some(LibraryUpdate::Approve(approval)) => updated.remember_profile(approval),
        Some(LibraryUpdate::Built(approval, build)) => {
            let current = locate_approval(&mut updated, approval)?;
            current
                .details
                .builds
                .retain(|old| old.directory != build.directory);
            current.details.builds.insert(0, build);
        }
        Some(LibraryUpdate::Relocate(approval, destination)) => {
            if updated
                .saved_profiles
                .iter()
                .any(|item| item.folder == destination)
            {
                return Err("The destination already belongs to another saved profile.".into());
            }
            locate_approval(&mut updated, approval)?.folder = destination;
        }
        None => {}
    }
    let text = serde_json::to_string_pretty(&updated)
        .map_err(|e| format!("We could not write the settings: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("We could not write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .map_err(|e| format!("We could not save {}: {e}", path.display()))?;
    Ok(updated.saved_profiles)
}

fn locate_approval<'a>(
    settings: &'a mut WizardSettings,
    approval: &SavedProfileApproval,
) -> Result<&'a mut SavedProfileApproval, String> {
    settings
        .saved_profiles
        .iter_mut()
        .find(|current| {
            current.folder == approval.folder && current.manifest_sha256 == approval.manifest_sha256
        })
        .ok_or_else(|| "The profile approval changed since this operation began.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_settings_load_without_profile_approvals() {
        let settings: WizardSettings = serde_json::from_str(
            r#"{"recent_installers":[],"text_scale":1.25,"draft":null,"welcomed":true}"#,
        )
        .unwrap();
        assert!(settings.saved_profiles.is_empty());
        assert_eq!(settings.text_scale, 1.25);
        assert_eq!(settings.appearance, Appearance::System);
    }

    #[test]
    fn saved_profiles_keep_the_approval_from_the_save_result() {
        let mut settings = WizardSettings::default();
        let folder = PathBuf::from("profile");
        settings.remember_profile(SavedProfileApproval {
            folder: folder.clone(),
            label: "Example".into(),
            manifest_sha256: "a".repeat(64),
            details: ProfileDetails::default(),
        });
        settings.remember_profile(SavedProfileApproval {
            folder,
            label: "Example updated".into(),
            manifest_sha256: "b".repeat(64),
            details: ProfileDetails::default(),
        });
        assert_eq!(settings.saved_profiles.len(), 1);
        let serialized = serde_json::to_string(&settings).unwrap();
        let restored: WizardSettings = serde_json::from_str(&serialized).unwrap();
        assert_eq!(restored.saved_profiles[0].manifest_sha256, "b".repeat(64));
    }

    #[test]
    fn stale_settings_preserve_saved_approvals() {
        let dir = std::env::temp_dir().join(format!("pf-approvals-{}", std::process::id()));
        let path = dir.join("wizard-settings.json");
        let mut first = WizardSettings::default();
        let mut second = WizardSettings::default();
        for (settings, label) in [(&mut first, "First"), (&mut second, "Second")] {
            save_approval(
                &path,
                settings,
                SavedProfileApproval {
                    folder: dir.join(label),
                    label: label.into(),
                    manifest_sha256: "a".repeat(64),
                    details: ProfileDetails::default(),
                },
            )
            .unwrap();
        }
        first.text_scale = 1.25;
        save(&path, &first).unwrap();
        let restored = load(&path);
        assert_eq!(restored.saved_profiles, second.saved_profiles);
        assert_eq!(restored.saved_profiles.len(), 2);
        assert_eq!(restored.text_scale, 1.25);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recent_list_dedupes_caps_and_ignores_blanks() {
        let mut s = WizardSettings::default();
        s.remember_installer("");
        s.remember_installer("  ");
        assert!(s.recent_installers.is_empty());
        for i in 0..7 {
            s.remember_installer(&format!(r"C:\Downloads\app{i}.exe"));
        }
        assert_eq!(s.recent_installers.len(), RECENT_LIMIT);
        assert_eq!(s.recent_installers[0], r"C:\Downloads\app6.exe");
        // Re-picking an older one moves it to the front, no duplicate.
        s.remember_installer(r"C:\Downloads\app4.exe");
        assert_eq!(s.recent_installers[0], r"C:\Downloads\app4.exe");
        assert_eq!(
            s.recent_installers
                .iter()
                .filter(|p| p.as_str() == r"C:\Downloads\app4.exe")
                .count(),
            1
        );
        assert_eq!(s.recent_installers.len(), RECENT_LIMIT);
    }

    #[test]
    fn text_scale_snaps_to_the_offered_choices() {
        let mut s = WizardSettings::default();
        assert_eq!(s.text_scale(), 1.0);
        s.text_scale = 1.3;
        assert_eq!(s.text_scale(), 1.25);
        s.text_scale = 99.0;
        assert_eq!(s.text_scale(), 1.5);
        s.text_scale = 0.1;
        assert_eq!(s.text_scale(), 1.0);
    }

    #[test]
    fn settings_round_trip_through_a_file() {
        let dir = std::env::temp_dir().join(format!("pf-settings-{}", std::process::id()));
        let path = dir.join("wizard-settings.json");
        let mut s = WizardSettings::default();
        s.remember_installer(r"C:\Downloads\widget-setup.exe");
        s.text_scale = 1.25;
        s.appearance = Appearance::Light;
        s.draft = Some(WizardDraft {
            step: "name".to_string(),
            installer_path: r"C:\Downloads\widget-setup.exe".to_string(),
            app_name: "Widget".to_string(),
            ..Default::default()
        });
        save(&path, &s).unwrap();
        let back = load(&path);
        assert_eq!(back, s);
        assert!(back.draft.as_ref().unwrap().has_progress());
        // A broken file reads as a first run, never an error.
        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(load(&path), WizardSettings::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_draft_is_not_progress() {
        assert!(!WizardDraft::default().has_progress());
    }
}
