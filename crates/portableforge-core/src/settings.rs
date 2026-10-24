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
    pub main_exe: String,
    pub package_version: String,
    pub online_url: String,
    pub online_sha: String,
}

impl WizardDraft {
    /// A draft is worth offering to resume only when the person had
    /// actually started — picked a file or begun naming.
    pub fn has_progress(&self) -> bool {
        !self.installer_path.trim().is_empty() || !self.app_name.trim().is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WizardSettings {
    #[serde(default)]
    pub recent_installers: Vec<String>,
    #[serde(default = "default_text_scale")]
    pub text_scale: f32,
    #[serde(default)]
    pub draft: Option<WizardDraft>,
}

fn default_text_scale() -> f32 {
    1.0
}

impl Default for WizardSettings {
    fn default() -> Self {
        Self {
            recent_installers: Vec::new(),
            text_scale: 1.0,
            draft: None,
        }
    }
}

impl WizardSettings {
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

/// Write the settings, replacing the file in one move so a crash
/// mid-save cannot leave a half-written file behind.
pub fn save(path: &Path, settings: &WizardSettings) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("We could not make {}: {e}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("We could not write the settings: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("We could not write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .map_err(|e| format!("We could not save {}: {e}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
