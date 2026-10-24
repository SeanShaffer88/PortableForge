//! The build pipeline: turn package details, the accepted review
//! fixes, and the harvested program files into a finished
//! PortableApps.com Format 3.9 package tree, then validate it.
//!
//! This is the step the wizard's "We build it for you" page and the
//! CLI `build` command both run. Before this module existed the pieces
//! were only connected by hand: `generate_tree` was called with an
//! empty fix list (dropping every fix the user accepted in review),
//! harvested files were copied by nobody, and the finished tree was
//! never checked as a whole.

use std::path::Path;

use crate::classify::Classification;
use crate::orchestrate::harvest_tree;
use crate::paf::PafMetadata;
use crate::validate::{generate_tree, validate_tree, ValidationIssue};

#[derive(Debug, Clone, PartialEq)]
pub struct BuildOutcome {
    /// Text files PortableForge authored, as package-relative paths.
    pub written: Vec<String>,
    /// Program files copied into `App\<BaseAppName>\`.
    pub app_files_copied: usize,
    pub app_bytes_copied: u64,
    /// First-run files copied into `App\DefaultData\`.
    pub default_data_files_copied: usize,
    /// Validation of the finished tree (errors mean it is not usable).
    pub issues: Vec<ValidationIssue>,
    /// Harvest entries that were skipped (unsafe paths, unreadable).
    pub skipped: Vec<String>,
}

impl BuildOutcome {
    pub fn has_errors(&self) -> bool {
        crate::validate::has_errors(&self.issues)
    }

    /// One plain summary line for the wizard log and the CLI.
    pub fn summary(&self) -> String {
        if self.has_errors() {
            let n = self.issues.iter().filter(|i| i.error).count();
            format!(
                "The package was made, but {n} problem{} must be fixed before it can be used — see the list.",
                if n == 1 { "" } else { "s" }
            )
        } else {
            format!(
                "Package made: {} files written, {} program files copied, {} starting-settings files. Everything checks out.",
                self.written.len(),
                self.app_files_copied,
                self.default_data_files_copied
            )
        }
    }
}

/// Build a complete package tree at `root`.
///
/// - `accepted` is the review's switched-on fixes — they go into the
///   Launcher and Installer settings (never silently dropped).
/// - `app_files`, when given, is a folder whose contents become
///   `App\<BaseAppName>\` (the harvested install folder).
/// - `default_data`, when given, is a folder structured like `Data\`
///   (e.g. it contains `Widget\settings.ini`) whose contents become
///   `App\DefaultData\`, the first-run seed.
///
/// Bad details refuse the build (via `generate_tree`); a finished tree
/// that fails validation is reported in the outcome, not hidden.
pub fn build_package(
    root: &Path,
    meta: &PafMetadata,
    accepted: &[Classification],
    app_files: Option<&Path>,
    default_data: Option<&Path>,
) -> Result<BuildOutcome, String> {
    let written = generate_tree(root, meta, accepted)?;
    let mut outcome = BuildOutcome {
        written,
        app_files_copied: 0,
        app_bytes_copied: 0,
        default_data_files_copied: 0,
        issues: Vec::new(),
        skipped: Vec::new(),
    };
    if let Some(src) = app_files {
        if !src.is_dir() {
            return Err(format!(
                "The program-files folder {} could not be found.",
                src.display()
            ));
        }
        let dest = root.join("App").join(&meta.base_app_name);
        let report = harvest_tree(src, &dest)?;
        outcome.app_files_copied = report.files_copied;
        outcome.app_bytes_copied = report.bytes_copied;
        outcome.skipped.extend(report.skipped);
    }
    if let Some(src) = default_data {
        if !src.is_dir() {
            return Err(format!(
                "The starting-settings folder {} could not be found.",
                src.display()
            ));
        }
        let dest = root.join("App").join("DefaultData");
        let report = harvest_tree(src, &dest)?;
        outcome.default_data_files_copied = report.files_copied;
        outcome.skipped.extend(report.skipped);
    }
    outcome.issues = validate_tree(root, meta);
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::{Classification, Finding, Severity};
    use crate::paf::Distribution;

    fn dirs_move() -> Classification {
        Classification {
            finding: Finding::DirectoriesMove {
                host_pattern: r"%APPDATA%\Acme\Widget".into(),
                data_target: "Widget".into(),
                evidence_file: r"C:\Users\X\AppData\Roaming\Acme\Widget\settings.ini".into(),
            },
            severity: Severity::Info,
            confidence: 0.9,
            why: String::new(),
        }
    }

    #[test]
    fn build_copies_files_seeds_data_and_keeps_accepted_fixes() {
        let base = std::env::temp_dir().join(format!("pf-build-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let app_src = base.join("harvest");
        std::fs::create_dir_all(&app_src).unwrap();
        std::fs::write(app_src.join("widget.exe"), b"MZfake").unwrap();
        std::fs::write(app_src.join("readme.txt"), b"hi").unwrap();
        let seed_src = base.join("seed").join("Widget");
        std::fs::create_dir_all(&seed_src).unwrap();
        std::fs::write(seed_src.join("settings.ini"), b"theme=dark").unwrap();

        let meta = crate::paf::tests::sample(Distribution::Bundled {
            redistribution_asserted: true,
        });
        let root = base.join("WidgetPortable");
        let outcome = build_package(
            &root,
            &meta,
            &[dirs_move()],
            Some(&app_src),
            Some(&base.join("seed")),
        )
        .unwrap();
        assert_eq!(outcome.app_files_copied, 2);
        assert_eq!(outcome.default_data_files_copied, 1);
        assert!(!outcome.has_errors(), "{:?}", outcome.issues);
        // The accepted fix survived into the Launcher settings — the
        // regression this module exists to prevent.
        let launcher = std::fs::read_to_string(
            root.join("App")
                .join("AppInfo")
                .join("Launcher")
                .join("WidgetPortable.ini"),
        )
        .unwrap();
        assert!(
            launcher.contains(r"Widget=%APPDATA%\Acme\Widget"),
            "{launcher}"
        );
        assert!(root
            .join("App")
            .join("DefaultData")
            .join("Widget")
            .join("settings.ini")
            .is_file());
        assert!(outcome.summary().contains("Everything checks out"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn build_without_the_program_files_reports_the_missing_exe() {
        let base = std::env::temp_dir().join(format!("pf-build-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let meta = crate::paf::tests::sample(Distribution::Bundled {
            redistribution_asserted: true,
        });
        let outcome = build_package(&base.join("pkg"), &meta, &[], None, None).unwrap();
        assert!(outcome.has_errors());
        assert!(outcome
            .issues
            .iter()
            .any(|i| i.error && i.message.contains("main executable")));
        let _ = std::fs::remove_dir_all(&base);
    }
}
