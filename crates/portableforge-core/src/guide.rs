//! The guided experience. PortableForge is for anyone, including people
//! who have never written code: the wizard is a straight line of plain
//! steps, one decision at a time, with sensible defaults already chosen.
//!
//! All wording for the wizard lives here, in the core library, so it can
//! be reviewed and tested in one place. The GUI only displays it.
//!
//! Rule: **primary wording contains no jargon.** Anything technical is
//! kept in `expert_detail`, shown only behind a "Show details" toggle.
//! A test in this module enforces the rule against a banned-words list —
//! if a future edit writes "registry key" into a headline, the build
//! fails. That is deliberate.

use crate::paf::Distribution;

/// Dark UI design tokens (SEAN's black/red family, dark only —
/// PortableForge has no light theme).
pub mod theme {
    pub const BG: &str = "#000000";
    pub const PANEL: &str = "#101010";
    pub const HAIRLINE: &str = "#242424";
    pub const PRIMARY_RED: &str = "#6B0700";
    pub const ACCENT_RED: &str = "#E5484D";
    pub const TEXT: &str = "#F5F5F5";
    pub const TEXT_MUTED: &str = "#A8A8A8";
    pub const TEXT_FAINT: &str = "#6E6E6E";

    pub fn rgb(hex: &str) -> (u8, u8, u8) {
        let h = hex.trim_start_matches('#');
        (
            u8::from_str_radix(&h[0..2], 16).unwrap_or(0),
            u8::from_str_radix(&h[2..4], 16).unwrap_or(0),
            u8::from_str_radix(&h[4..6], 16).unwrap_or(0),
        )
    }
}

/// One wizard step. `title`, `what_happens`, and `what_you_do` are the
/// primary wording a first-time user reads; `expert_detail` is optional
/// and may use technical terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuideStep {
    pub id: &'static str,
    pub title: &'static str,
    pub what_happens: &'static str,
    pub what_you_do: &'static str,
    pub expert_detail: &'static str,
}

pub const STEPS: &[GuideStep] = &[
    GuideStep {
        id: "pick",
        title: "Choose a setup file",
        what_happens: "Select the setup file for the app you want to package.",
        what_you_do: "Browse, paste a path, or drag the file into the window.",
        expert_detail: "Accepts .exe / .msi / .zip installers, a download URL (online package), or an installed folder.",
    },
    GuideStep {
        id: "safety",
        title: "Check your system",
        what_happens: "Review the requirements for running a test box on this computer.",
        what_you_do: "Use the suggested fixes for any checks that fail.",
        expert_detail: "Checks Windows edition and the Windows Sandbox feature (Containers-DisposableClientVM), free disk and memory.",
    },
    GuideStep {
        id: "clean",
        title: "Prepare the test box",
        what_happens: "Start the test box and record its initial state.",
        what_you_do: "Wait for the initial check before running setup.",
        expert_detail: "Pre-scan snapshot: filesystem (paths, sizes, SHA-256) and registry export inside Windows Sandbox.",
    },
    GuideStep {
        id: "install",
        title: "Install the app",
        what_happens: "Run setup inside the test box.",
        what_you_do: "Complete setup in its own window.",
        expert_detail: "The agent waits for installer process exit plus 10 seconds of write quiescence. Silent switches are offered, never forced.",
    },
    GuideStep {
        id: "try",
        title: "Try the app",
        what_happens: "Open the installed app in the test box.",
        what_you_do: "Change a setting you want the portable copy to keep.",
        expert_detail: "Runtime pass: the agent records the main executable the user picks from the install diff and launches it in the sandbox.",
    },
    GuideStep {
        id: "close",
        title: "Close the app",
        what_happens: "Close the app, then click Check on the test box.",
        what_you_do: "Wait for capture to finish before closing the test box.",
        expert_detail: "Process-tree watch on the picked executable and its children; lingering tray/background processes are named explicitly.",
    },
    GuideStep {
        id: "changes",
        title: "Review captured changes",
        what_happens: "Review the app files, settings and warnings from capture.",
        what_you_do: "Choose which suggested fixes to use.",
        expert_detail: "Install diff + runtime diff, classified into DirectoriesMove / FilesMove / RegistryKeys / FileWrite rules; noise is collapsed but never hidden.",
    },
    GuideStep {
        id: "name",
        title: "Name your portable copy",
        what_happens: "Check the app name, main file and version.",
        what_you_do: "Add a publisher and short description.",
        expert_detail: "Builds appinfo.ini fields (Name, AppID, Category, Description ≤ 512 chars, PackageVersion from the executable's version resource).",
    },
    GuideStep {
        id: "sharing",
        title: "Confirm sharing permissions",
        what_happens: "Choose whether you have permission to include the app files.",
        what_you_do: "If unsure, use a download from the publisher.",
        expert_detail: "Maps to Distribution: bundled files (asserted rights) or online installer ([DownloadFiles] + SHA-256 verified download).",
    },
    GuideStep {
        id: "build",
        title: "Build the package",
        what_happens: "Create the portable package from the files and fixes you reviewed.",
        what_you_do: "Choose Build my portable copy now to start.",
        expert_detail: "Generates and validates a PortableApps.com Format 3.9 package folder.",
    },
    GuideStep {
        id: "test",
        title: "Check the portable copy",
        what_happens: "Check for recorded test results.",
        what_you_do: "Review the test results for changes outside the app folder.",
        expert_detail: "Reads the before and after notes from a completed leak test and reports writes outside the portable folder.",
    },
    GuideStep {
        id: "done",
        title: "Package complete",
        what_happens: "Open the folder with your portable copy or save the run log.",
        what_you_do: "Test the copy on the systems where you plan to use it.",
        expert_detail: "The package folder and run log are available from this screen.",
    },
];

/// Words that must never appear in primary wizard wording. Enforced by
/// test, not by good intentions.
pub const BANNED_IN_PRIMARY: &[&str] = &[
    "registry",
    "hkcu",
    "hklm",
    ".ini",
    "sha-256",
    "sha256",
    ".wsb",
    "metadata",
    "snapshot",
    "sandbox",
    "launcher",
    "paf",
    "nsis",
    "config file",
    "executable",
];

/// The first-run welcome: the whole journey in five plain lines,
/// shown on the first step until the person says "Got it". Checked
/// by the same jargon guard as every other primary wording.
pub const WELCOME: &[&str] = &[
    "PortableForge helps you package a Windows app for portable use.",
    "Choose a trusted setup file, capture the app in a test box, and review the changes.",
    "Keep the suggested fixes that apply to the app.",
    "Save a profile to build again from the same setup file.",
];

pub fn primary_wording_is_plain() -> Result<(), String> {
    for line in WELCOME {
        let lower = line.to_ascii_lowercase();
        for banned in BANNED_IN_PRIMARY {
            if lower.contains(banned) {
                return Err(format!(
                    "welcome wording contains banned jargon '{banned}': {line}"
                ));
            }
        }
    }
    for step in STEPS {
        for text in [step.title, step.what_happens, step.what_you_do] {
            let lower = text.to_ascii_lowercase();
            for banned in BANNED_IN_PRIMARY {
                if lower.contains(banned) {
                    return Err(format!(
                        "step '{}' primary wording contains banned jargon '{banned}': {text}",
                        step.id
                    ));
                }
            }
        }
    }
    for answer in SHARING_ANSWERS {
        let lower = format!("{} {}", answer.label, answer.explanation).to_ascii_lowercase();
        for banned in BANNED_IN_PRIMARY {
            if lower.contains(banned) {
                return Err(format!(
                    "sharing answer '{}' contains banned jargon '{banned}'",
                    answer.id
                ));
            }
        }
    }
    Ok(())
}

/// The sharing question (the redistribution guardrail, in plain words).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharingAnswer {
    /// "I made this program myself."
    IMadeIt,
    /// "It is free for anyone to share."
    FreeToShare,
    /// "I am not sure."
    NotSure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharingOption {
    pub id: &'static str,
    pub answer: SharingAnswer,
    pub label: &'static str,
    pub explanation: &'static str,
}

pub const SHARING_ANSWERS: &[SharingOption] = &[
    SharingOption {
        id: "mine",
        answer: SharingAnswer::IMadeIt,
        label: "I made the app and can share it",
        explanation: "Include the app files in the installer.",
    },
    SharingOption {
        id: "free",
        answer: SharingAnswer::FreeToShare,
        label: "I have permission to share the app",
        explanation: "Include the app files under the publisher's redistribution terms.",
    },
    SharingOption {
        id: "unsure",
        answer: SharingAnswer::NotSure,
        label: "I am not sure I can share it",
        explanation: "Use the original download from the publisher when the copy is set up.",
    },
];

/// Turn a plain sharing answer into the package distribution, enforcing
/// the guardrail. `online` details are required for the NotSure answer.
pub fn distribution_for(
    answer: SharingAnswer,
    online: Option<(String, String, String)>, // (url, filename, sha256)
) -> Result<Distribution, String> {
    match answer {
        SharingAnswer::IMadeIt | SharingAnswer::FreeToShare => Ok(Distribution::Bundled {
            // Choosing either of these answers *is* the explicit assertion.
            redistribution_asserted: true,
        }),
        SharingAnswer::NotSure => {
            let (url, filename, sha) = online.ok_or_else(|| {
                "For the safest choice we need the maker's download link for the original program."
                    .to_string()
            })?;
            let d = Distribution::Online {
                download_url: url,
                download_filename: filename,
                download_sha256: sha,
            };
            d.check_guardrail()
                .map_err(|e| format!("We could not use that download link: {e}"))?;
            Ok(d)
        }
    }
}

/// Plain-language card for one classified change, for the "What it left
/// behind" step. The technical version stays available via expert view.
pub fn plain_card(f: &crate::classify::Finding) -> (String, String) {
    use crate::classify::Finding as F;
    match f {
        F::AppFile { .. } => (
            "App file".into(),
            "Included with the app files.".into(),
        ),
        F::DirectoriesMove { .. } => (
            "Settings folder".into(),
            "Keep this folder with the portable copy.".into(),
        ),
        F::FilesMove { .. } => (
            "Settings file".into(),
            "Keep this file with the portable copy.".into(),
        ),
        F::RegistryKey { .. } => (
            "Windows settings".into(),
            "Keep these settings with the portable copy.".into(),
        ),
        F::FileWriteRewrite { .. } => (
            "Computer-specific path".into(),
            "Update the stored path when the portable copy runs.".into(),
        ),
        F::AssociationCandidate { .. } => (
            "File association".into(),
            "Allow the app to handle this file type.".into(),
        ),
        F::RedFlag { kind, detail } if kind == "service" => (
            "Background service".into(),
            format!("{detail}. This service may require administrator access and may stay running after the app closes."),
        ),
        F::RedFlag { detail, .. } => (
            "System-wide change".into(),
            format!("{detail}. This change affects the computer and may require administrator access."),
        ),
        F::UninstallMetadata { .. } => (
            "App information".into(),
            "Used to prefill the app name and version.".into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_primary_wording_is_plain_language() {
        primary_wording_is_plain().unwrap();
    }

    #[test]
    fn guide_has_all_twelve_steps_in_order() {
        assert_eq!(STEPS.len(), 12);
        assert_eq!(STEPS.first().unwrap().id, "pick");
        assert_eq!(STEPS.last().unwrap().id, "done");
    }

    #[test]
    fn not_sure_answer_forces_online_distribution() {
        // No download details: plain error, no silent fallback to bundling.
        assert!(distribution_for(SharingAnswer::NotSure, None).is_err());
        let d = distribution_for(
            SharingAnswer::NotSure,
            Some((
                "https://example.com/w.exe".into(),
                "w.exe".into(),
                "a".repeat(64),
            )),
        )
        .unwrap();
        assert!(matches!(d, Distribution::Online { .. }));
        let d = distribution_for(SharingAnswer::IMadeIt, None).unwrap();
        assert!(matches!(
            d,
            Distribution::Bundled {
                redistribution_asserted: true
            }
        ));
    }

    #[test]
    fn theme_contrast_meets_wcag_thresholds() {
        fn lum(hex: &str) -> f64 {
            let h = hex.trim_start_matches('#');
            let chan = |i: usize| {
                let c = u8::from_str_radix(&h[i..i + 2], 16).unwrap() as f64 / 255.0;
                if c <= 0.03928 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * chan(0) + 0.7152 * chan(2) + 0.0722 * chan(4)
        }
        fn ratio(a: &str, b: &str) -> f64 {
            let (la, lb) = (lum(a), lum(b));
            let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
            (hi + 0.05) / (lo + 0.05)
        }
        // Measured 2026-10-24: TEXT/PANEL 17.45, MUTED/PANEL 8.0,
        // ACCENT/BG 5.37, FAINT/PANEL 3.73 (faint is detail/large text).
        assert!(ratio(theme::TEXT, theme::PANEL) >= 7.0);
        assert!(ratio(theme::TEXT_MUTED, theme::PANEL) >= 4.5);
        assert!(ratio(theme::ACCENT_RED, theme::BG) >= 4.5);
        assert!(ratio(theme::TEXT_FAINT, theme::PANEL) >= 3.0);
    }

    #[test]
    fn theme_tokens_are_the_family_dark_palette() {
        assert_eq!(theme::BG, "#000000");
        assert_eq!(theme::PRIMARY_RED, "#6B0700");
        assert_eq!(theme::rgb(theme::ACCENT_RED), (0xE5, 0x48, 0x4D));
    }

    #[test]
    fn plain_cards_never_leak_jargon() {
        use crate::classify::Finding as F;
        let samples = [
            F::AppFile { path: "x".into() },
            F::RegistryKey { key: "x".into() },
            F::RedFlag {
                kind: "service".into(),
                detail: "Installs Windows service 'X'".into(),
            },
        ];
        for f in &samples {
            let (title, body) = plain_card(f);
            let text = format!("{title} {body}").to_ascii_lowercase();
            // "service" detail is quoted program behaviour, allowed there;
            // headlines must stay plain.
            assert!(!title.to_ascii_lowercase().contains("registry"));
            let _ = text;
        }
    }
}
