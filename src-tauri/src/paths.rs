use std::path::PathBuf;

pub fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// Roaming app data on Windows (%APPDATA%\AgentNotch).
pub fn data_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(home).join("AgentNotch")
}

/// Machine-local cache for large files (%LOCALAPPDATA%\AgentNotch).
pub fn local_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(home)
        .join("AgentNotch")
}

pub fn claude_snapshot() -> PathBuf {
    data_dir().join("claude-usage.json")
}

pub fn claude_session() -> PathBuf {
    data_dir().join("claude-session.json")
}

pub fn agent_sessions() -> PathBuf {
    data_dir().join("agent-sessions.json")
}

pub fn stats() -> PathBuf {
    data_dir().join("stats.json")
}

pub fn agent_events() -> PathBuf {
    data_dir().join("agent-events.json")
}

/// Writes via a temporary file and rename so readers never see partial data.
pub fn write_atomic(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, data)?;
    std::fs::rename(tmp, path)
}

pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn settings() -> PathBuf {
    data_dir().join("settings.json")
}

pub fn models_dir() -> PathBuf {
    local_dir().join("models")
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
