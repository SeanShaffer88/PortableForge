//! Windows Sandbox configuration (`.wsb`) generation.
//!
//! Format verified against Microsoft Learn's Windows Sandbox
//! configuration documentation: XML `<Configuration>` with mapped
//! folders, a logon command, and device/networking switches.
//!
//! **100% Rust rule:** the logon command starts the Rust agent binary
//! directly. No PowerShell, batch, or script files are generated or
//! used anywhere in PortableForge.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SandboxMode {
    /// Mode A — Windows Sandbox (true isolation, disposable).
    WindowsSandbox,
    /// Mode B — the user's own VM snapshot.
    OwnVm,
    /// Mode C — host snapshot + restore-point rollback (NOT isolated;
    /// always labelled as such).
    HostSnapshot,
}

impl SandboxMode {
    /// Plain-language name + honesty label for the UI and reports.
    pub fn describe(&self) -> (&'static str, &'static str) {
        match self {
            SandboxMode::WindowsSandbox => (
                "Safe test box",
                "Your computer is untouched — everything happens in a throwaway box that is deleted afterwards.",
            ),
            SandboxMode::OwnVm => (
                "Your own test computer",
                "Everything happens inside the virtual computer you pointed us at.",
            ),
            SandboxMode::HostSnapshot => (
                "Careful mode on this computer",
                "This happens on your real computer and is undone afterwards. It is watched and put back, but it is NOT sealed off.",
            ),
        }
    }

    pub fn is_truly_isolated(&self) -> bool {
        !matches!(self, SandboxMode::HostSnapshot)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxConfig {
    /// Host folder with the original installer (mapped read-only).
    pub input_dir: String,
    /// Host folder with `portableforge-agent.exe` (mapped read-only).
    pub tools_dir: String,
    /// Host exchange folder (mapped read/write): snapshots, harvest, logs.
    pub exchange_dir: String,
    /// Where the folders appear inside the sandbox.
    pub sandbox_input: String,
    pub sandbox_tools: String,
    pub sandbox_exchange: String,
    pub networking: bool,
    /// Clipboard sharing between the real computer and the box.
    /// Default off: nothing should move between them unnoticed.
    pub clipboard: bool,
    pub memory_mb: u32,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            input_dir: String::new(),
            tools_dir: String::new(),
            exchange_dir: String::new(),
            sandbox_input: r"C:\PortableForge\Input".into(),
            sandbox_tools: r"C:\PortableForge\Tools".into(),
            sandbox_exchange: r"C:\PortableForge\Exchange".into(),
            networking: false,
            clipboard: false,
            memory_mb: 4096,
        }
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

impl SandboxConfig {
    /// Render the `.wsb` file. The agent path inside the sandbox is the
    /// logon command — a direct executable launch, no shell script.
    pub fn to_wsb(&self) -> String {
        let networking = if self.networking { "Enable" } else { "Disable" };
        let clipboard = if self.clipboard { "Enable" } else { "Disable" };
        format!(
            "<Configuration>\n\
             \x20 <Networking>{networking}</Networking>\n\
             \x20 <ClipboardRedirection>{clipboard}</ClipboardRedirection>\n\
             \x20 <PrinterRedirection>Disable</PrinterRedirection>\n\
             \x20 <AudioInput>Disable</AudioInput>\n\
             \x20 <VideoInput>Disable</VideoInput>\n\
             \x20 <ProtectedClient>Enable</ProtectedClient>\n\
             \x20 <MemoryInMB>{}</MemoryInMB>\n\
             \x20 <MappedFolders>\n\
             \x20   <MappedFolder>\n\
             \x20     <HostFolder>{}</HostFolder>\n\
             \x20     <SandboxFolder>{}</SandboxFolder>\n\
             \x20     <ReadOnly>true</ReadOnly>\n\
             \x20   </MappedFolder>\n\
             \x20   <MappedFolder>\n\
             \x20     <HostFolder>{}</HostFolder>\n\
             \x20     <SandboxFolder>{}</SandboxFolder>\n\
             \x20     <ReadOnly>true</ReadOnly>\n\
             \x20   </MappedFolder>\n\
             \x20   <MappedFolder>\n\
             \x20     <HostFolder>{}</HostFolder>\n\
             \x20     <SandboxFolder>{}</SandboxFolder>\n\
             \x20     <ReadOnly>false</ReadOnly>\n\
             \x20   </MappedFolder>\n\
             \x20 </MappedFolders>\n\
             \x20 <LogonCommand>\n\
             \x20   <Command>{}\\portableforge-agent.exe run --exchange {}</Command>\n\
             \x20 </LogonCommand>\n\
             </Configuration>\n",
            self.memory_mb,
            xml_escape(&self.input_dir),
            xml_escape(&self.sandbox_input),
            xml_escape(&self.tools_dir),
            xml_escape(&self.sandbox_tools),
            xml_escape(&self.exchange_dir),
            xml_escape(&self.sandbox_exchange),
            xml_escape(&self.sandbox_tools),
            xml_escape(&self.sandbox_exchange),
        )
    }

    pub fn check(&self) -> Result<(), String> {
        for (name, dir) in [
            ("input", &self.input_dir),
            ("tools", &self.tools_dir),
            ("exchange", &self.exchange_dir),
        ] {
            if dir.trim().is_empty() {
                return Err(format!("The {name} folder is not set."));
            }
        }
        if self.memory_mb < 512 {
            return Err("The test box needs at least 512 MB of memory.".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> SandboxConfig {
        SandboxConfig {
            input_dir: r"C:\Users\Sean\Downloads".into(),
            tools_dir: r"C:\PortableForge\Tools".into(),
            exchange_dir: r"D:\Work & Play\Exchange".into(),
            ..Default::default()
        }
    }

    #[test]
    fn wsb_is_well_formed_and_safe_by_default() {
        let wsb = cfg().to_wsb();
        assert!(wsb.contains("<Networking>Disable</Networking>"));
        assert!(wsb.contains("<ProtectedClient>Enable</ProtectedClient>"));
        assert_eq!(wsb.matches("<ReadOnly>true</ReadOnly>").count(), 2);
        assert_eq!(wsb.matches("<ReadOnly>false</ReadOnly>").count(), 1);
        // XML escaping of the ampersand path.
        assert!(wsb.contains(r"D:\Work &amp; Play\Exchange"));
        // Logon command is the Rust agent directly: no scripts, ever.
        assert!(wsb.contains("portableforge-agent.exe run"));
        assert!(!wsb.to_ascii_lowercase().contains("powershell"));
        assert!(!wsb.to_ascii_lowercase().contains(".ps1"));
        assert!(!wsb.to_ascii_lowercase().contains(".bat"));
        assert!(!wsb.to_ascii_lowercase().contains(".cmd"));
    }

    #[test]
    fn host_snapshot_mode_never_claims_isolation() {
        assert!(!SandboxMode::HostSnapshot.is_truly_isolated());
        let (_, label) = SandboxMode::HostSnapshot.describe();
        assert!(label.contains("NOT sealed off"));
        assert!(SandboxMode::WindowsSandbox.is_truly_isolated());
    }
}
