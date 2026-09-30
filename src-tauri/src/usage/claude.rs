//! Claude Code usage via its status-line hook. Claude Code runs
//! `AgentNotch.exe --claude-bridge`, piping session JSON on stdin; the bridge
//! stores the rate limits for the widget and prints a one-line summary.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::parser::{
    parse_claude_session, parse_claude_status_line, ParseError, SessionInfo, UsageSnapshot,
};
use crate::paths;

pub const BRIDGE_FLAG: &str = "--claude-bridge";

#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    #[error("claude.bridge.executable")]
    Executable,
    #[error("claude.bridge.existing")]
    ExistingStatusLine,
    #[error("claude.bridge.malformed")]
    MalformedSettings,
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

pub fn load() -> Option<UsageSnapshot> {
    let data = std::fs::read(paths::claude_snapshot()).ok()?;
    let mut snapshot: UsageSnapshot = serde_json::from_slice(&data).ok()?;
    let valid = !snapshot.windows.is_empty()
        && snapshot.fetched_at <= paths::now() + 60
        && snapshot
            .windows
            .iter()
            .all(|w| (0.0..=100.0).contains(&w.used_percent));
    if !valid {
        return None;
    }
    snapshot.origin = "Claude Code".into();
    Some(snapshot)
}

/// Entry point for `--claude-bridge`. Never fails loudly: a broken status line
/// would be visible in every Claude Code session.
pub fn run_bridge() -> i32 {
    let mut input = Vec::new();
    if std::io::stdin().read_to_end(&mut input).is_err() {
        return 1;
    }
    let mut out = std::io::stdout();
    let now = paths::now();
    let Ok(root) = serde_json::from_slice::<Value>(&input) else {
        let _ = writeln!(std::io::stderr(), "AgentNotch bridge: malformed payload");
        return 1;
    };
    // Per-session model, cost and line counts for the activity panel.
    let _ = crate::agents::sessions::update(|all| {
        crate::agents::sessions::apply_status_line(all, &root, now * 1000)
    });
    // Session details arrive even before the first rate-limit reading.
    if let Some(session) = parse_claude_session(&root, now) {
        if let Ok(data) = serde_json::to_vec(&session) {
            let _ = paths::write_atomic(&paths::claude_session(), &data);
        }
    }
    match parse_claude_status_line(&root, now) {
        Ok(snapshot) => {
            if let Err(error) = write_snapshot(&snapshot) {
                let _ = writeln!(std::io::stderr(), "AgentNotch bridge: {error}");
                return 1;
            }
            let summary: Vec<String> = snapshot
                .windows
                .iter()
                .take(2)
                .map(|w| format!("{} {}%", short_label(&w.id), w.used_percent.round() as i64))
                .collect();
            let _ = writeln!(out, "Claude {}", summary.join(" · "));
            0
        }
        Err(ParseError::MissingRateLimits) => {
            let _ = writeln!(out, "Claude: limits after first reply");
            0
        }
        Err(error) => {
            let _ = writeln!(std::io::stderr(), "AgentNotch bridge: {error}");
            1
        }
    }
}

pub fn load_session() -> Option<SessionInfo> {
    let data = std::fs::read(paths::claude_session()).ok()?;
    serde_json::from_slice(&data).ok()
}

fn short_label(id: &str) -> &'static str {
    match id {
        "claude-five_hour" => "5h",
        "claude-seven_day" => "7d",
        _ => "$",
    }
}

fn write_snapshot(snapshot: &UsageSnapshot) -> std::io::Result<()> {
    paths::write_atomic(&paths::claude_snapshot(), &serde_json::to_vec(snapshot)?)
}

/// Claude Code runs status-line commands through a shell (Git Bash on
/// Windows), so forward slashes avoid backslash-escaping surprises.
pub fn bridge_command(executable: &Path) -> String {
    let path = executable.to_string_lossy().replace('\\', "/");
    format!("\"{path}\" {BRIDGE_FLAG}")
}

fn settings_path() -> PathBuf {
    paths::home().join(".claude").join("settings.json")
}

fn read_settings(path: &Path) -> Result<Map<String, Value>, BridgeError> {
    if !path.exists() {
        return Ok(Map::new());
    }
    let data = std::fs::read(path)?;
    match serde_json::from_slice::<Value>(&data) {
        Ok(Value::Object(map)) => Ok(map),
        _ => Err(BridgeError::MalformedSettings),
    }
}

pub fn is_bridge_configured() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    configured_in(&settings_path(), &exe)
}

fn configured_in(settings: &Path, exe: &Path) -> bool {
    read_settings(settings)
        .ok()
        .and_then(|s| {
            s.get("statusLine")?
                .get("command")?
                .as_str()
                .map(str::to_owned)
        })
        .is_some_and(|c| c == bridge_command(exe))
}

pub fn install_bridge() -> Result<(), BridgeError> {
    let exe = std::env::current_exe().map_err(|_| BridgeError::Executable)?;
    install_into(&settings_path(), &exe)
}

fn install_into(settings_path: &Path, exe: &Path) -> Result<(), BridgeError> {
    let mut settings = read_settings(settings_path)?;
    if let Some(existing) = settings
        .get("statusLine")
        .and_then(|s| s.get("command"))
        .and_then(Value::as_str)
    {
        if !existing.contains(BRIDGE_FLAG) {
            return Err(BridgeError::ExistingStatusLine);
        }
    }
    if settings_path.exists() {
        let stamp = chrono_stamp();
        let backup =
            settings_path.with_file_name(format!("settings.agentnotch-backup-{stamp}.json"));
        std::fs::copy(settings_path, backup)?;
    } else if let Some(dir) = settings_path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    settings.insert(
        "statusLine".into(),
        serde_json::json!({ "type": "command", "command": bridge_command(exe), "refreshInterval": 30 }),
    );
    let data = serde_json::to_vec_pretty(&Value::Object(settings))
        .map_err(|_| BridgeError::MalformedSettings)?;
    let tmp = settings_path.with_extension("json.agentnotch-tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(tmp, settings_path)?;
    Ok(())
}

fn chrono_stamp() -> String {
    // Seconds are unique enough for a manual action and need no date crate.
    paths::now().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("agentnotch-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn command_uses_forward_slashes() {
        let cmd = bridge_command(Path::new(
            r"C:\Users\A B\AppData\Local\AgentNotch\AgentNotch.exe",
        ));
        assert_eq!(
            cmd,
            "\"C:/Users/A B/AppData/Local/AgentNotch/AgentNotch.exe\" --claude-bridge"
        );
    }

    #[test]
    fn install_preserves_other_settings_and_backs_up() {
        let dir = temp_dir("install");
        let settings = dir.join("settings.json");
        std::fs::write(
            &settings,
            r#"{"model":"opus","permissions":{"allow":["Bash"]}}"#,
        )
        .unwrap();
        let exe = Path::new("C:/AgentNotch.exe");
        install_into(&settings, exe).unwrap();
        let written: Value = serde_json::from_slice(&std::fs::read(&settings).unwrap()).unwrap();
        assert_eq!(written["model"], "opus");
        assert_eq!(written["statusLine"]["command"], bridge_command(exe));
        assert!(configured_in(&settings, exe));
        let backups = std::fs::read_dir(&dir)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains("backup")
            })
            .count();
        assert_eq!(backups, 1);
    }

    #[test]
    fn install_refuses_foreign_status_line() {
        let dir = temp_dir("foreign");
        let settings = dir.join("settings.json");
        std::fs::write(
            &settings,
            r#"{"statusLine":{"type":"command","command":"my-script.sh"}}"#,
        )
        .unwrap();
        assert!(matches!(
            install_into(&settings, Path::new("x.exe")),
            Err(BridgeError::ExistingStatusLine)
        ));
    }

    #[test]
    fn install_rejects_malformed_settings() {
        let dir = temp_dir("malformed");
        let settings = dir.join("settings.json");
        std::fs::write(&settings, "not json").unwrap();
        assert!(matches!(
            install_into(&settings, Path::new("x.exe")),
            Err(BridgeError::MalformedSettings)
        ));
    }
}
