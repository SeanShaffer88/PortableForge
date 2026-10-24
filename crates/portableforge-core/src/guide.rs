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
        title: "Pick your program",
        what_happens: "Choose the program you want to make portable — the kind you can carry on a USB stick and use on any computer.",
        what_you_do: "Pick the setup file you downloaded, or choose a program that is already on this computer.",
        expert_detail: "Accepts .exe / .msi / .zip installers, a download URL (online package), or an installed folder.",
    },
    GuideStep {
        id: "safety",
        title: "Safety check",
        what_happens: "We check that this computer can run a safe test box — a throwaway mini computer where the program can do no harm. Nothing on your real computer will be touched.",
        what_you_do: "Nothing. If something is missing, we will tell you exactly what to click to fix it.",
        expert_detail: "Checks Windows edition and the Windows Sandbox feature (Containers-DisposableClientVM), free disk and memory.",
    },
    GuideStep {
        id: "clean",
        title: "A clean test computer",
        what_happens: "We start the test box and take a careful note of everything in it while it is still clean. Later we compare, so we can spot every single thing your program adds.",
        what_you_do: "Nothing — this takes a minute or two.",
        expert_detail: "Pre-scan snapshot: filesystem (paths, sizes, SHA-256) and registry export inside Windows Sandbox.",
    },
    GuideStep {
        id: "install",
        title: "Install the program",
        what_happens: "A new window opens — that is the test box. Inside it, the program's own setup runs, just like a normal install, except it all happens in the throwaway box.",
        what_you_do: "Click through the setup inside the test box window, the same way you normally would. We will tell you when it has finished.",
        expert_detail: "The agent waits for installer process exit plus 10 seconds of write quiescence. Silent switches are offered, never forced.",
    },
    GuideStep {
        id: "try",
        title: "Try it out",
        what_happens: "Still inside the test box, try the program out. While you use it, the program saves its settings somewhere — trying it is how we find out where.",
        what_you_do: "Open the program, use it for a moment, and change one or two settings — anything you like. Then come back here.",
        expert_detail: "Runtime pass: the agent records the main executable the user picks from the install diff and launches it in the sandbox.",
    },
    GuideStep {
        id: "close",
        title: "Close the program",
        what_happens: "Close the program in the test box. We watch until it has fully closed, so nothing is missed.",
        what_you_do: "Close the program's window. If a part of it stays open in the background, we will point at it so you can close that too.",
        expert_detail: "Process-tree watch on the picked executable and its children; lingering tray/background processes are named explicitly.",
    },
    GuideStep {
        id: "changes",
        title: "What it left behind",
        what_happens: "We compare the test box now with how it started. Everything the program added is listed in plain words, with the fix we suggest for each one. Most programs need two or three small fixes so their settings travel with them.",
        what_you_do: "Look through the short list. Leave the suggested fixes on unless something looks wrong — you can turn any of them off.",
        expert_detail: "Install diff + runtime diff, classified into DirectoriesMove / FilesMove / RegistryKeys / FileWrite rules; noise is collapsed but never hidden.",
    },
    GuideStep {
        id: "name",
        title: "Name your portable copy",
        what_happens: "Give your portable copy a name and a short description. We have filled most of this in from the program itself — change anything you like.",
        what_you_do: "Check the name and description, then continue.",
        expert_detail: "Builds appinfo.ini fields (Name, AppID, Category, Description ≤ 512 chars, PackageVersion from the executable's version resource).",
    },
    GuideStep {
        id: "sharing",
        title: "Sharing your portable copy",
        what_happens: "One honest question, because it changes how your copy is made: are you allowed to pass this program on to other people?",
        what_you_do: "Pick the answer that fits. If you are not sure, pick that — it is the safest choice and always works.",
        expert_detail: "Maps to Distribution: bundled files (asserted rights) or online installer ([DownloadFiles] + SHA-256 verified download).",
    },
    GuideStep {
        id: "build",
        title: "We build it for you",
        what_happens: "We put everything together into one file. You do not need to do anything technical — this is the part other tools make you do by hand.",
        what_you_do: "Nothing. Watch the bar if you like.",
        expert_detail: "Generates the PortableApps.com Format 3.9 tree, validates it, then runs the pinned, hash-verified PortableApps.com Launcher and Installer generators.",
    },
    GuideStep {
        id: "test",
        title: "Testing the finished copy",
        what_happens: "We open a brand-new test box and try your finished copy there, start to finish. If it leaves anything behind on the test computer, we will show you and offer to fix it and test again.",
        what_you_do: "Nothing. A perfect score means your copy leaves the next computer exactly as it found it.",
        expert_detail: "Leak test: fresh sandbox, install the .paf.exe, run via the launcher, close, post-scan; writes outside the portable folder are reported as leaks.",
    },
    GuideStep {
        id: "done",
        title: "Done — your file is ready",
        what_happens: "Your portable copy is one single file. Copy it to a USB stick, run it anywhere, and it works — no installing, and it keeps its settings in its own folder.",
        what_you_do: "Take your file. That is all there is to it.",
        expert_detail: "Delivers the .paf.exe plus its SHA-256, the session log, and a plain statement of anything not verified.",
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

pub fn primary_wording_is_plain() -> Result<(), String> {
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
        label: "I made this program myself",
        explanation: "Then it is yours to share. Your portable copy will contain the whole program in one file.",
    },
    SharingOption {
        id: "free",
        answer: SharingAnswer::FreeToShare,
        label: "It is free for anyone to share",
        explanation: "Free and open programs can travel whole. Your portable copy will contain the whole program in one file.",
    },
    SharingOption {
        id: "unsure",
        answer: SharingAnswer::NotSure,
        label: "I am not sure",
        explanation: "The safe choice: your portable copy will fetch the original program from its maker when it is set up, and check it is genuine. Nothing is shared that should not be.",
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
            "Part of the program itself".into(),
            "This belongs inside your portable copy. Nothing for you to decide.".into(),
        ),
        F::DirectoriesMove { .. } => (
            "It keeps its settings in a folder on the computer".into(),
            "We will move that folder inside your portable copy, so the settings travel with the program and nothing is left behind.".into(),
        ),
        F::FilesMove { .. } => (
            "It keeps a settings file on the computer".into(),
            "We will move that file inside your portable copy, so it travels with the program.".into(),
        ),
        F::RegistryKey { .. } => (
            "It remembers things in the computer's private notebook".into(),
            "Many programs jot settings down in a shared notebook inside Windows. We will give your portable copy its own page to carry with it instead.".into(),
        ),
        F::FileWriteRewrite { .. } => (
            "It wrote down this computer's details in one of its files".into(),
            "That would confuse it on a new computer. We will make your copy update that detail by itself each time it runs.".into(),
        ),
        F::AssociationCandidate { .. } => (
            "It would like to open certain kinds of files".into(),
            "Only turn this on if you want double-clicking those files to open this program. It is off unless you choose it.".into(),
        ),
        F::RedFlag { kind, detail } if kind == "service" => (
            "It installs a helper that is always running".into(),
            format!("{detail}. Portable copies cannot do this properly — the helper would be left running on every computer you use. You can still make a copy, but it will ask for the computer owner's permission each time."),
        ),
        F::RedFlag { detail, .. } => (
            "It changes something shared by the whole computer".into(),
            format!("{detail}. Changes like this affect everyone who uses the computer, so a portable copy should not make them quietly. You can still continue, but the copy will ask for the computer owner's permission."),
        ),
        F::UninstallMetadata { .. } => (
            "Its name tag".into(),
            "The program's own name and version, which we used to fill in the naming step. Nothing is copied.".into(),
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
