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

pub mod classify;
pub mod diff;
pub mod guide;
pub mod leaktest;
pub mod noise;
pub mod orchestrate;
pub mod paf;
pub mod progress;
pub mod regfile;
pub mod review;
pub mod rollback;
pub mod session;
pub mod snapshot;
pub mod tools;
pub mod validate;
pub mod wsb;

pub use classify::{classify, Classification, Finding, Severity};
pub use diff::{Diff, FileChange, RegChange};
pub use paf::{Distribution, PafMetadata};
pub use session::{Phase, Session};
pub use snapshot::{FileEntry, RegEntry, Snapshot};
pub use validate::{validate_metadata, validate_tree, ValidationIssue};
pub use wsb::{SandboxConfig, SandboxMode};
