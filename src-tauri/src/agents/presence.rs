//! Which agents are alive right now: interactive Claude Code and Codex
//! processes. Codex's background app server (which AgentNotch itself starts
//! to read usage) is not counted.

use std::ffi::OsString;
use std::path::Path;

use serde::Serialize;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Presence {
    pub claude: u32,
    pub codex: u32,
}

/// Classifies one process. Claude Code's native installer names its binary
/// after the version (…/claude/versions/2.1.285), so the path also decides.
pub fn classify(name: &str, exe: Option<&Path>, cmd: &[OsString]) -> Option<&'static str> {
    let name = name.to_ascii_lowercase();
    let name = name.trim_end_matches(".exe");
    let path = exe
        .map(|p| p.to_string_lossy().replace('\\', "/").to_ascii_lowercase())
        .unwrap_or_default();
    let background = cmd.iter().any(|a| {
        let a = a.to_string_lossy();
        // Helpers, and Electron child processes of the desktop apps.
        a == "app-server"
            || a == "--claude-bridge"
            || a == "--agent-event"
            || a.starts_with("--type=")
    });
    // Claude and Codex desktop apps share the executable names.
    let desktop_app = [
        "anthropicclaude",
        "/claude.app/",
        "/codex.app/",
        "/program files/",
        "/windowsapps/",
    ]
    .iter()
    .any(|marker| path.contains(marker));
    if background || desktop_app {
        return None;
    }
    match name {
        "claude" => Some("claude"),
        "codex" => Some("codex"),
        _ if path.contains("/claude/versions/") => Some("claude"),
        _ => None,
    }
}

pub struct Scanner {
    system: System,
}

impl Default for Scanner {
    fn default() -> Self {
        Self {
            system: System::new(),
        }
    }
}

impl Scanner {
    pub fn scan(&mut self) -> Presence {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_exe(UpdateKind::OnlyIfNotSet)
                .with_cmd(UpdateKind::OnlyIfNotSet),
        );
        let mut presence = Presence::default();
        for process in self.system.processes().values() {
            match classify(
                &process.name().to_string_lossy(),
                process.exe(),
                process.cmd(),
            ) {
                Some("claude") => presence.claude += 1,
                Some("codex") => presence.codex += 1,
                _ => {}
            }
        }
        presence
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn classifies_agents_and_skips_helpers() {
        assert_eq!(
            classify("claude.exe", None, &cmd(&["claude"])),
            Some("claude")
        );
        assert_eq!(classify("codex", None, &cmd(&["codex"])), Some("codex"));
        assert_eq!(
            classify("codex", None, &cmd(&["codex", "app-server"])),
            None
        );
        assert_eq!(
            classify(
                "2.1.285",
                Some(Path::new("/home/a/.local/share/claude/versions/2.1.285")),
                &[]
            ),
            Some("claude")
        );
        assert_eq!(
            classify(
                "claude.exe",
                Some(Path::new(r"C:\Users\a\.local\bin\claude.exe")),
                &[]
            ),
            Some("claude")
        );
        // Claude Desktop is not Claude Code.
        assert_eq!(
            classify(
                "claude.exe",
                Some(Path::new(
                    r"C:\Users\a\AppData\Local\AnthropicClaude\app-1.0\claude.exe"
                )),
                &[]
            ),
            None
        );
        assert_eq!(
            classify("claude.exe", None, &cmd(&["claude.exe", "--type=renderer"])),
            None
        );
        assert_eq!(
            classify(
                "AgentNotch",
                None,
                &cmd(&["AgentNotch", "--agent-event", "claude"])
            ),
            None
        );
        assert_eq!(classify("zsh", None, &[]), None);
    }
}
