//! PortableForge core library.
//!
//! All decision logic lives here and is testable off-Windows:
//! filesystem/registry snapshots, diffing, noise filtering, classification
//! of changes into PortableApps.com Launcher handling, PortableApps.com
//! Format 3.9 generation and validation, Windows Sandbox (.wsb) config
//! generation, and the session state model shared by the host wizard, the
//! CLI, and the in-sandbox agent.
//!
//! The GUI and the agent binaries render state and collect intent only;
//! no business rules live in them.

#[cfg(windows)]
pub mod artwork;
pub mod build;
pub mod classify;
#[cfg(any(windows, test))]
pub mod compile;
pub mod diff;
pub mod guide;
pub mod leaktest;
pub mod noise;
pub mod orchestrate;
pub mod paf;
#[cfg(windows)]
pub mod profile;
pub mod progress;
pub mod regfile;
pub mod review;
pub mod rollback;
pub mod sbie;
#[cfg(all(windows, target_arch = "x86_64"))]
pub mod sbie_verified;
pub mod session;
pub mod settings;
pub mod snapshot;
pub mod timeutil;
pub mod tools;
pub mod validate;
#[cfg(all(windows, target_arch = "x86_64"))]
pub mod wizard_package;
pub mod wsb;

pub use classify::{classify, Classification, Finding, Severity};
pub use diff::{Diff, FileChange, RegChange};
pub use paf::{Distribution, PafMetadata};
pub use session::{Phase, Session};
pub use snapshot::{FileEntry, RegEntry, Snapshot};
pub use validate::{validate_metadata, validate_tree, ValidationIssue};
pub use wsb::{SandboxConfig, SandboxMode};
