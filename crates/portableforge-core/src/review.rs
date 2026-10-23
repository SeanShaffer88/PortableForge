//! The review model behind the "What it left behind" step.
//!
//! Each classified change becomes one review item with a decision the
//! user can flip. Defaults follow the severity rules from the design:
//! - ordinary fixes start **on**,
//! - file-association claims start **off** (opt-in only),
//! - red flags are not fixes at all — they must be **acknowledged**
//!   before the flow can continue, and they force `RequiresAdmin`
//!   style honesty into the final report,
//! - uninstall metadata and noise are not shown as decisions.

use serde::{Deserialize, Serialize};

use crate::classify::{Classification, Finding, Severity};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Decision {
    /// A fix the user can switch on/off.
    Fix { on: bool },
    /// Not a fix: a warning the user must acknowledge.
    Acknowledge { seen: bool },
    /// Informational only (program files, name-tag metadata).
    Info,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewItem {
    pub classification: Classification,
    pub decision: Decision,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Review {
    pub items: Vec<ReviewItem>,
}

pub fn default_decision(c: &Classification) -> Decision {
    match &c.finding {
        Finding::AssociationCandidate { .. } => Decision::Fix { on: false },
        Finding::RedFlag { .. } => Decision::Acknowledge { seen: false },
        Finding::AppFile { .. } | Finding::UninstallMetadata { .. } => Decision::Info,
        _ if c.severity == Severity::RedFlag => Decision::Acknowledge { seen: false },
        _ => Decision::Fix { on: true },
    }
}

impl Review {
    pub fn from_classifications(list: Vec<Classification>) -> Self {
        Self {
            items: list
                .into_iter()
                .map(|c| ReviewItem {
                    decision: default_decision(&c),
                    classification: c,
                })
                .collect(),
        }
    }

    /// Classifications whose fixes are switched on — the input to
    /// package generation.
    pub fn accepted(&self) -> Vec<Classification> {
        self.items
            .iter()
            .filter(|i| matches!(i.decision, Decision::Fix { on: true }))
            .map(|i| i.classification.clone())
            .collect()
    }

    /// The flow may continue only when every red flag has been seen.
    pub fn all_acknowledged(&self) -> bool {
        self.items.iter().all(|i| match i.decision {
            Decision::Acknowledge { seen } => seen,
            _ => true,
        })
    }

    pub fn has_red_flags(&self) -> bool {
        self.items
            .iter()
            .any(|i| matches!(i.decision, Decision::Acknowledge { .. }))
    }

    /// Plain counts for the wizard header, e.g. "4 fixes on, 1 off, 1 warning".
    pub fn summary(&self) -> String {
        let mut on = 0;
        let mut off = 0;
        let mut warnings = 0;
        for i in &self.items {
            match i.decision {
                Decision::Fix { on: true } => on += 1,
                Decision::Fix { on: false } => off += 1,
                Decision::Acknowledge { .. } => warnings += 1,
                Decision::Info => {}
            }
        }
        let mut parts = vec![format!("{on} fixes on")];
        if off > 0 {
            parts.push(format!("{off} off"));
        }
        if warnings > 0 {
            parts.push(format!(
                "{warnings} warning{}",
                if warnings == 1 { "" } else { "s" }
            ));
        }
        parts.join(", ")
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("review serialisation cannot fail")
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|e| format!("invalid review JSON: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(finding: Finding, severity: Severity) -> Classification {
        Classification {
            finding,
            severity,
            confidence: 0.9,
            why: String::new(),
        }
    }

    #[test]
    fn defaults_follow_the_rules() {
        let review = Review::from_classifications(vec![
            item(
                Finding::DirectoriesMove {
                    host_pattern: "%APPDATA%\\A\\B".into(),
                    data_target: "B".into(),
                    evidence_file: "x".into(),
                },
                Severity::Info,
            ),
            item(
                Finding::AssociationCandidate {
                    key: "HKCR\\x".into(),
                },
                Severity::Review,
            ),
            item(
                Finding::RedFlag {
                    kind: "service".into(),
                    detail: "svc".into(),
                },
                Severity::RedFlag,
            ),
        ]);
        // Fix on, association off, red flag blocks until acknowledged.
        assert_eq!(review.accepted().len(), 1);
        assert!(review.has_red_flags());
        assert!(!review.all_acknowledged());
        let mut review = review;
        review.items[2].decision = Decision::Acknowledge { seen: true };
        assert!(review.all_acknowledged());
        assert!(review.summary().contains("1 fixes on"));
        assert!(review.summary().contains("1 off"));
        assert!(review.summary().contains("1 warning"));
        // And it persists.
        let back = Review::from_json(&review.to_json()).unwrap();
        assert_eq!(review, back);
    }
}
