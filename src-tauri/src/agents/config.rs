//! Reads and edits the agents' own configuration: default model and effort,
//! and the hooks that report activity to AgentNotch. Only the keys we own are
//! touched; everything else in the files is preserved.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Map, Value};

use super::events::EVENT_FLAG;
use crate::paths;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ConfigError {
    #[error("agents.malformed")]
    Malformed,
    #[error("agents.notifyInUse")]
    NotifyInUse,
    #[error("agents.io")]
    Io,
    #[error("agents.invalid")]
    Invalid,
}

impl From<std::io::Error> for ConfigError {
    fn from(_: std::io::Error) -> Self {
        ConfigError::Io
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelOption {
    pub id: String,
    pub name: String,
    pub efforts: Vec<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentConfig {
    /// Configured default model id; empty when unset (account default).
    pub model: String,
    pub effort: String,
    pub models: Vec<ModelOption>,
    pub hooks_installed: bool,
}

// ---------- Claude Code: ~/.claude/settings.json ----------

pub const CLAUDE_MODELS: [(&str, &str); 5] = [
    ("", "Default"),
    ("opus", "Opus"),
    ("sonnet", "Sonnet"),
    ("haiku", "Haiku"),
    ("fable", "Fable"),
];
pub const CLAUDE_EFFORTS: [&str; 4] = ["low", "medium", "high", "xhigh"];

fn claude_settings_path() -> PathBuf {
    paths::home().join(".claude").join("settings.json")
}

fn read_json(path: &Path) -> Result<Map<String, Value>, ConfigError> {
    match std::fs::read(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Map::new()),
        Err(_) => Err(ConfigError::Io),
        Ok(data) => match serde_json::from_slice::<Value>(&data) {
            Ok(Value::Object(map)) => Ok(map),
            _ => Err(ConfigError::Malformed),
        },
    }
}

fn write_json(path: &Path, map: Map<String, Value>) -> Result<(), ConfigError> {
    let data =
        serde_json::to_vec_pretty(&Value::Object(map)).map_err(|_| ConfigError::Malformed)?;
    paths::write_atomic(path, &data).map_err(ConfigError::from)
}

pub fn quoted_command(executable: &Path, args: &[&str]) -> String {
    let path = executable.to_string_lossy().replace('\\', "/");
    let mut command = format!("\"{path}\"");
    for arg in args {
        command.push(' ');
        command.push_str(arg);
    }
    command
}

fn claude_hook_command(exe: &Path) -> String {
    quoted_command(exe, &[EVENT_FLAG, "claude"])
}

fn claude_hooks_present(settings: &Map<String, Value>) -> bool {
    ["Stop", "Notification"].iter().all(|event| {
        settings
            .get("hooks")
            .and_then(|h| h.get(*event))
            .and_then(Value::as_array)
            .is_some_and(|groups| {
                groups.iter().any(|g| {
                    g.get("hooks")
                        .and_then(Value::as_array)
                        .is_some_and(|hooks| {
                            hooks.iter().any(|h| {
                                h.get("command")
                                    .and_then(Value::as_str)
                                    .is_some_and(|c| c.contains(EVENT_FLAG))
                            })
                        })
                })
            })
    })
}

pub fn claude_config() -> AgentConfig {
    claude_config_in(&claude_settings_path())
}

fn claude_config_in(path: &Path) -> AgentConfig {
    let settings = read_json(path).unwrap_or_default();
    AgentConfig {
        model: settings
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        effort: settings
            .get("effortLevel")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        models: CLAUDE_MODELS
            .iter()
            .map(|(id, name)| ModelOption {
                id: (*id).to_owned(),
                name: (*name).to_owned(),
                efforts: CLAUDE_EFFORTS.iter().map(|e| (*e).to_owned()).collect(),
            })
            .collect(),
        hooks_installed: claude_hooks_present(&settings),
    }
}

/// Empty strings remove the key, returning to Claude Code's default.
pub fn set_claude_defaults(model: Option<&str>, effort: Option<&str>) -> Result<(), ConfigError> {
    set_claude_defaults_in(&claude_settings_path(), model, effort)
}

fn set_claude_defaults_in(
    path: &Path,
    model: Option<&str>,
    effort: Option<&str>,
) -> Result<(), ConfigError> {
    let mut settings = read_json(path)?;
    if let Some(model) = model {
        if !CLAUDE_MODELS.iter().any(|(id, _)| *id == model) {
            return Err(ConfigError::Invalid);
        }
        if model.is_empty() {
            settings.remove("model");
        } else {
            settings.insert("model".into(), json!(model));
        }
    }
    if let Some(effort) = effort {
        if !effort.is_empty() && !CLAUDE_EFFORTS.contains(&effort) {
            return Err(ConfigError::Invalid);
        }
        if effort.is_empty() {
            settings.remove("effortLevel");
        } else {
            settings.insert("effortLevel".into(), json!(effort));
        }
    }
    write_json(path, settings)
}

pub fn install_claude_hooks() -> Result<(), ConfigError> {
    let exe = std::env::current_exe().map_err(|_| ConfigError::Io)?;
    install_claude_hooks_in(&claude_settings_path(), &exe)
}

fn install_claude_hooks_in(path: &Path, exe: &Path) -> Result<(), ConfigError> {
    let mut settings = read_json(path)?;
    if claude_hooks_present(&settings) {
        return Ok(());
    }
    if path.exists() {
        let backup =
            path.with_file_name(format!("settings.agentnotch-backup-{}.json", paths::now()));
        std::fs::copy(path, backup)?;
    }
    let hook = json!({ "type": "command", "command": claude_hook_command(exe), "async": true, "timeout": 10 });
    let hooks = settings.entry("hooks").or_insert_with(|| json!({}));
    let hooks = hooks.as_object_mut().ok_or(ConfigError::Malformed)?;
    for event in ["Stop", "Notification"] {
        let groups = hooks.entry(event).or_insert_with(|| json!([]));
        let groups = groups.as_array_mut().ok_or(ConfigError::Malformed)?;
        groups.push(json!({ "matcher": "*", "hooks": [hook.clone()] }));
    }
    write_json(path, settings)
}

// ---------- Codex: ~/.codex/config.toml ----------

fn codex_dir() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| paths::home().join(".codex"))
}

/// Index of the first table header; keys before it are top-level.
fn top_level_end(lines: &[String]) -> usize {
    lines
        .iter()
        .position(|l| l.trim_start().starts_with('['))
        .unwrap_or(lines.len())
}

fn key_line(lines: &[String], key: &str) -> Option<usize> {
    lines[..top_level_end(lines)].iter().position(|l| {
        let t = l.trim_start();
        t.strip_prefix(key)
            .is_some_and(|rest| rest.trim_start().starts_with('='))
    })
}

/// Value of a simple top-level `key = "string"` entry.
pub fn toml_get(text: &str, key: &str) -> Option<String> {
    let lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let line = &lines[key_line(&lines, key)?];
    let value = line.split_once('=')?.1.trim();
    let value = value.split(" #").next().unwrap_or(value).trim();
    Some(value.trim_matches(|c| c == '"' || c == '\'').to_owned())
}

/// Sets or removes (`None`) a top-level key, keeping comments and tables.
pub fn toml_set(text: &str, key: &str, value: Option<&str>) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let existing = key_line(&lines, key);
    match (existing, value) {
        (Some(i), Some(v)) => lines[i] = format!("{key} = {v}"),
        (Some(i), None) => {
            lines.remove(i);
        }
        (None, Some(v)) => {
            let mut at = top_level_end(&lines);
            while at > 0 && lines[at - 1].trim().is_empty() {
                at -= 1;
            }
            lines.insert(at, format!("{key} = {v}"));
        }
        (None, None) => {}
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

fn toml_string(value: &str) -> String {
    if value.contains('\'') {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        format!("'{value}'") // Literal string: Windows paths need no escaping.
    }
}

fn codex_notify_line(exe: &Path) -> String {
    let parts = [
        exe.to_string_lossy().into_owned(),
        EVENT_FLAG.to_owned(),
        "codex".to_owned(),
    ];
    format!(
        "[{}]",
        parts
            .iter()
            .map(|p| toml_string(p))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn codex_models(dir: &Path) -> Vec<ModelOption> {
    let Ok(data) = std::fs::read(dir.join("models_cache.json")) else {
        return Vec::new();
    };
    let Ok(root) = serde_json::from_slice::<Value>(&data) else {
        return Vec::new();
    };
    root.get("models")
        .and_then(Value::as_array)
        .map(|models| {
            models
                .iter()
                .filter(|m| m.get("visibility").and_then(Value::as_str) == Some("list"))
                .filter_map(|m| {
                    Some(ModelOption {
                        id: m.get("slug")?.as_str()?.to_owned(),
                        name: m
                            .get("display_name")
                            .and_then(Value::as_str)
                            .or_else(|| m.get("slug")?.as_str())?
                            .to_owned(),
                        efforts: m
                            .get("supported_reasoning_levels")
                            .and_then(Value::as_array)
                            .map(|levels| {
                                levels
                                    .iter()
                                    .filter_map(|l| l.get("effort")?.as_str().map(str::to_owned))
                                    .collect()
                            })
                            .unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn codex_config() -> AgentConfig {
    let dir = codex_dir();
    let text = std::fs::read_to_string(dir.join("config.toml")).unwrap_or_default();
    AgentConfig {
        model: toml_get(&text, "model").unwrap_or_default(),
        effort: toml_get(&text, "model_reasoning_effort").unwrap_or_default(),
        models: codex_models(&dir),
        hooks_installed: toml_get(&text, "notify").is_some_and(|n| n.contains(EVENT_FLAG)),
    }
}

fn safe_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.:".contains(c))
}

pub fn set_codex_defaults(model: Option<&str>, effort: Option<&str>) -> Result<(), ConfigError> {
    let path = codex_dir().join("config.toml");
    let mut text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(_) => return Err(ConfigError::Io),
    };
    for (key, value) in [("model", model), ("model_reasoning_effort", effort)] {
        let Some(value) = value else { continue };
        if !value.is_empty() && !safe_identifier(value) {
            return Err(ConfigError::Invalid);
        }
        let rendered = (!value.is_empty()).then(|| format!("\"{value}\""));
        text = toml_set(&text, key, rendered.as_deref());
    }
    paths::write_atomic(&path, text.as_bytes()).map_err(ConfigError::from)
}

pub fn install_codex_notify() -> Result<(), ConfigError> {
    let exe = std::env::current_exe().map_err(|_| ConfigError::Io)?;
    let path = codex_dir().join("config.toml");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let updated = install_codex_notify_text(&text, &exe)?;
    if updated != text {
        paths::write_atomic(&path, updated.as_bytes())?;
    }
    Ok(())
}

fn install_codex_notify_text(text: &str, exe: &Path) -> Result<String, ConfigError> {
    match toml_get(text, "notify") {
        Some(existing) if existing.contains(EVENT_FLAG) => Ok(text.to_owned()),
        // Codex runs a single notify program; never replace the user's own.
        Some(_) => Err(ConfigError::NotifyInUse),
        None => Ok(toml_set(text, "notify", Some(&codex_notify_line(exe)))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("agentnotch-cfg-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("settings.json")
    }

    #[test]
    fn claude_defaults_round_trip_and_preserve_keys() {
        let path = temp("claude");
        std::fs::write(
            &path,
            r#"{"permissions":{"allow":["Bash"]},"model":"sonnet"}"#,
        )
        .unwrap();
        set_claude_defaults_in(&path, Some("opus"), Some("xhigh")).unwrap();
        let cfg = claude_config_in(&path);
        assert_eq!((cfg.model.as_str(), cfg.effort.as_str()), ("opus", "xhigh"));
        set_claude_defaults_in(&path, Some(""), Some("")).unwrap();
        let raw: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(raw.get("model").is_none() && raw.get("effortLevel").is_none());
        assert_eq!(raw["permissions"]["allow"][0], "Bash");
        assert_eq!(
            set_claude_defaults_in(&path, Some("gpt"), None),
            Err(ConfigError::Invalid)
        );
    }

    #[test]
    fn claude_hooks_are_merged_once() {
        let path = temp("hooks");
        std::fs::write(&path, r#"{"hooks":{"Stop":[{"matcher":"*","hooks":[{"type":"command","command":"say done"}]}]}}"#)
            .unwrap();
        let exe = Path::new("C:/Apps/AgentNotch.exe");
        install_claude_hooks_in(&path, exe).unwrap();
        install_claude_hooks_in(&path, exe).unwrap();
        let raw: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let stop = raw["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2, "user's hook kept, ours added once");
        assert_eq!(stop[0]["hooks"][0]["command"], "say done");
        assert_eq!(raw["hooks"]["Notification"].as_array().unwrap().len(), 1);
        assert!(claude_config_in(&path).hooks_installed);
    }

    #[test]
    fn toml_edits_only_top_level_keys() {
        let text = "# settings\nmodel = \"a\"\nsandbox_mode = \"x\"\n\n[projects.\"/p\"]\nmodel = \"not-top\"\n";
        assert_eq!(toml_get(text, "model").as_deref(), Some("a"));
        let set = toml_set(text, "model_reasoning_effort", Some("\"high\""));
        assert!(
            set.contains("sandbox_mode = \"x\"\nmodel_reasoning_effort = \"high\"\n\n[projects")
        );
        let replaced = toml_set(&set, "model", Some("\"b\""));
        assert!(replaced.starts_with("# settings\nmodel = \"b\""));
        assert!(
            replaced.contains("model = \"not-top\""),
            "table keys untouched"
        );
        let removed = toml_set(&replaced, "model", None);
        assert_eq!(toml_get(&removed, "model"), None);
        assert!(!toml_get(text, "model_reasoning").is_some());
    }

    #[test]
    fn codex_notify_never_replaces_users_program() {
        let exe = Path::new(r"C:\Users\A\AgentNotch.exe");
        let installed = install_codex_notify_text("model = \"m\"\n", exe).unwrap();
        assert!(
            installed.contains(r"notify = ['C:\Users\A\AgentNotch.exe', '--agent-event', 'codex']")
        );
        assert_eq!(
            install_codex_notify_text(&installed, exe).unwrap(),
            installed
        );
        assert_eq!(
            install_codex_notify_text("notify = [\"bash\", \"n.sh\"]\n", exe),
            Err(ConfigError::NotifyInUse)
        );
    }

    #[test]
    fn identifiers_are_validated() {
        assert!(safe_identifier("gpt-6.1-sol"));
        assert!(!safe_identifier("x\"\nnotify = []"));
    }
}
