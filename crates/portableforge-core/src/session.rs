//! Session model: a packaging run is a folder on disk, not state in
//! memory, so a cancelled or interrupted run resumes from its last
//! completed step. The same JSON is the agent's command/heartbeat
//! channel inside the sandbox (`exchange\state.json`).

use serde::{Deserialize, Serialize};

use crate::guide;
use crate::wsb::SandboxMode;

/// Wizard phases, in order — one per guide step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Pick,
    SafetyCheck,
    PreScan,
    Install,
    TryOut,
    Close,
    ReviewChanges,
    Name,
    Sharing,
    Build,
    LeakTest,
    Done,
}

impl Phase {
    pub fn index(self) -> usize {
        match self {
            Phase::Pick => 0,
            Phase::SafetyCheck => 1,
            Phase::PreScan => 2,
            Phase::Install => 3,
            Phase::TryOut => 4,
            Phase::Close => 5,
            Phase::ReviewChanges => 6,
            Phase::Name => 7,
            Phase::Sharing => 8,
            Phase::Build => 9,
            Phase::LeakTest => 10,
            Phase::Done => 11,
        }
    }

    pub fn step(self) -> &'static guide::GuideStep {
        &guide::STEPS[self.index()]
    }

    /// Percentage of the guided flow completed when *entering* a phase.
    pub fn flow_percent(self) -> f64 {
        self.index() as f64 / guide::STEPS.len() as f64 * 100.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    /// Folder-safe session id, e.g. `2026-10-23-0815-widget`.
    pub id: String,
    pub phase: Phase,
    pub sandbox_mode: SandboxMode,
    /// The installer the user picked (host path as given).
    pub installer_path: Option<String>,
    /// Download URL when the source is online.
    pub source_url: Option<String>,
    /// Snapshot labels completed so far, in order.
    pub snapshots_done: Vec<String>,
    /// Plain-language log lines, shown in the wizard's log view.
    pub log: Vec<String>,
    /// Heartbeat from the in-sandbox agent (unix seconds), if running.
    pub agent_heartbeat_secs: Option<i64>,
    /// What the agent is doing right now, in its own plain words
    /// (e.g. "Looking through C:\Program Files — 1,250 files checked
    /// so far"), so a long scan never looks like a hang.
    #[serde(default)]
    pub agent_action: Option<String>,
    /// The helper program's own version, written by the helper into
    /// its state file so the host can warn when the two do not match
    /// (a stale helper beside a newer PortableForge).
    #[serde(default)]
    pub agent_version: Option<String>,
    /// When this run was created, as a UTC ISO-8601 stamp — never a
    /// local time whose zone a reader would have to guess.
    #[serde(default)]
    pub created_utc: String,
    /// The sharing answer id once chosen (`mine` / `free` / `unsure`).
    pub sharing_answer: Option<String>,
}

impl Session {
    pub fn new(id: impl Into<String>, sandbox_mode: SandboxMode) -> Self {
        Self {
            id: id.into(),
            phase: Phase::Pick,
            sandbox_mode,
            installer_path: None,
            source_url: None,
            snapshots_done: Vec::new(),
            log: Vec::new(),
            agent_heartbeat_secs: None,
            agent_action: None,
            agent_version: None,
            created_utc: crate::timeutil::now_iso(),
            sharing_answer: None,
        }
    }

    /// Add a log line, stamped with the current UTC time so a session
    /// log read later (or on another machine) is never ambiguous.
    pub fn say(&mut self, line: impl Into<String>) {
        self.log
            .push(format!("[{}] {}", crate::timeutil::now_iso(), line.into()));
    }

    pub fn advance_to(&mut self, phase: Phase) {
        self.phase = phase;
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("session serialisation cannot fail")
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|e| format!("invalid session JSON: {e}"))
    }

    /// Agent liveness: heartbeat within `max_age_secs` of `now_secs`.
    pub fn agent_alive(&self, now_secs: i64, max_age_secs: i64) -> bool {
        match self.agent_heartbeat_secs {
            Some(hb) => now_secs - hb <= max_age_secs,
            None => false,
        }
    }
}

/// Write a file in one move — write to a neighbour, then rename over
/// the target — so a reader (the host watching `state.json`, or
/// reading a finished note) never sees a half-written file.
pub fn write_file_atomic(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phases_map_to_guide_steps_and_progress() {
        assert_eq!(Phase::Pick.step().id, "pick");
        assert_eq!(Phase::Done.step().id, "done");
        assert_eq!(Phase::Pick.flow_percent(), 0.0);
        assert!(Phase::Done.flow_percent() > 90.0);
    }

    #[test]
    fn atomic_write_lands_whole_and_action_round_trips() {
        let dir = std::env::temp_dir().join(format!("pf-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        write_file_atomic(&path, "{\"a\":1}").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"a\":1}");
        assert!(!dir.join("state.tmp").exists());
        let mut s = Session::new("x", SandboxMode::WindowsSandbox);
        s.agent_action = Some("Looking through folders".into());
        let back = Session::from_json(&s.to_json()).unwrap();
        assert_eq!(
            back.agent_action.as_deref(),
            Some("Looking through folders")
        );
        // Older state files without the field still read.
        let old = s
            .to_json()
            .lines()
            .filter(|l| !l.contains("agent_action"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(Session::from_json(&old).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn session_round_trips_and_heartbeat_ages_out() {
        let mut s = Session::new("2026-10-23-widget", SandboxMode::WindowsSandbox);
        s.say("Started");
        s.advance_to(Phase::Install);
        s.agent_heartbeat_secs = Some(1_000);
        let back = Session::from_json(&s.to_json()).unwrap();
        assert_eq!(s, back);
        assert!(back.agent_alive(1_010, 15));
        assert!(!back.agent_alive(1_100, 15));
        assert!(!Session::new("x", SandboxMode::WindowsSandbox).agent_alive(0, 15));
    }
}
