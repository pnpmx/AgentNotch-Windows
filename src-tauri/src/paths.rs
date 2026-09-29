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
