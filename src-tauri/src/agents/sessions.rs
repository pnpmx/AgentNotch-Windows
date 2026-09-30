//! Per-session agent activity, so several Claude Code and Codex sessions in
//! different terminal tabs can be followed at once. Hook processes update a
//! shared file under an exclusive lock; the widget only reads it.

use std::collections::BTreeMap;
use std::io::{Read, Seek, Write};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::paths;

const MESSAGE_LIMIT: usize = 4000;
const PROMPT_LIMIT: usize = 1200;
const DETAIL_LIMIT: usize = 48;
/// Sessions quiet for longer than this are dropped.
const EXPIRY_MS: i64 = 12 * 60 * 60 * 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SessionState {
    #[default]
    Idle,
    Working,
    Waiting,
    Done,
}

/// What the agent is doing right now, rendered by the UI in its language.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    /// thinking | edit | run | read | search | web | agent | tool
    pub kind: String,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TaskSummary {
    pub cost_usd: Option<f64>,
    pub duration_secs: i64,
    pub lines_added: Option<i64>,
    pub lines_removed: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Session {
    pub id: String,
    pub source: String,
    pub project: String,
    pub cwd: String,
    pub state: SessionState,
    pub activity: Option<Activity>,
    pub model: Option<String>,
    pub last_prompt: String,
    pub last_message: String,
    /// Unix ms when the current task started.
    pub started_at: i64,
    pub updated_at: i64,
    pub cost_usd: Option<f64>,
    pub lines_added: Option<i64>,
    pub lines_removed: Option<i64>,
    pub cost_at_start: Option<f64>,
    pub lines_added_at_start: Option<i64>,
    pub lines_removed_at_start: Option<i64>,
    pub last_task: Option<TaskSummary>,
}

pub type Sessions = BTreeMap<String, Session>;

pub fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let cut: String = text.chars().take(limit - 1).collect();
    format!("{}…", cut.trim_end())
}

fn file_name(path: &str) -> String {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .to_owned()
}

/// Short, display-only description of a tool call. File contents and long
/// commands are never kept.
pub fn activity_for_tool(tool: &str, input: &Value) -> Activity {
    let text = |key: &str| input.get(key).and_then(Value::as_str).unwrap_or("");
    let single_line = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let (kind, detail) = match tool {
        "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => (
            "edit",
            file_name(if text("file_path").is_empty() {
                text("notebook_path")
            } else {
                text("file_path")
            }),
        ),
        "Bash" | "PowerShell" => ("run", single_line(text("command"))),
        "Read" => ("read", file_name(text("file_path"))),
        "Grep" | "Glob" => ("search", text("pattern").to_owned()),
        "WebFetch" => (
            "web",
            text("url")
                .split("://")
                .nth(1)
                .unwrap_or(text("url"))
                .split('/')
                .next()
                .unwrap_or("")
                .to_owned(),
        ),
        "WebSearch" => ("web", text("query").to_owned()),
        "Task" | "Agent" => ("agent", single_line(text("description"))),
        other if other.starts_with("mcp__") => (
            "tool",
            other.rsplit("__").next().unwrap_or(other).to_owned(),
        ),
        other => ("tool", other.to_owned()),
    };
    Activity {
        kind: kind.into(),
        detail: truncate(&detail, DETAIL_LIMIT),
    }
}

fn project_of(cwd: &str) -> String {
    file_name(cwd)
}

/// Applies a Claude Code hook payload. Returns the finished task when the
/// payload marks the end of a main-agent turn.
pub fn apply_claude_hook(
    sessions: &mut Sessions,
    payload: &Value,
    now: i64,
) -> Option<TaskSummary> {
    let text = |key: &str| payload.get(key).and_then(Value::as_str).unwrap_or("");
    let id = text("session_id");
    if id.is_empty() {
        return None;
    }
    let session = sessions.entry(id.to_owned()).or_insert_with(|| Session {
        id: id.to_owned(),
        source: "claude".into(),
        started_at: now,
        ..Session::default()
    });
    if !text("cwd").is_empty() {
        session.cwd = text("cwd").to_owned();
        session.project = project_of(&session.cwd);
    }
    session.updated_at = now;
    match text("hook_event_name") {
        "UserPromptSubmit" => {
            session.state = SessionState::Working;
            session.last_prompt = truncate(text("prompt").trim(), PROMPT_LIMIT);
            session.started_at = now;
            session.activity = Some(Activity {
                kind: "thinking".into(),
                detail: String::new(),
            });
            session.cost_at_start = session.cost_usd;
            session.lines_added_at_start = session.lines_added;
            session.lines_removed_at_start = session.lines_removed;
            None
        }
        "PreToolUse" => {
            session.state = SessionState::Working;
            let input = payload.get("tool_input").cloned().unwrap_or(Value::Null);
            session.activity = Some(activity_for_tool(text("tool_name"), &input));
            None
        }
        "Notification" => {
            if matches!(
                text("notification_type"),
                "permission_prompt"
                    | "idle_prompt"
                    | "agent_needs_input"
                    | "elicitation_dialog"
                    | "elicitation_url_dialog"
            ) {
                session.state = SessionState::Waiting;
            }
            None
        }
        "Stop" if payload.get("agent_id").is_none() => {
            session.state = SessionState::Done;
            session.activity = None;
            session.last_message = truncate(text("last_assistant_message").trim(), MESSAGE_LIMIT);
            if let Some(model) = payload.get("model").and_then(Value::as_str) {
                session.model.get_or_insert_with(|| model.to_owned());
            }
            let diff = |now: Option<f64>, start: Option<f64>| match (now, start) {
                (Some(n), Some(s)) if n >= s => Some(n - s),
                (Some(n), None) => Some(n),
                _ => None,
            };
            let lines =
                |now: Option<i64>, start: Option<i64>| now.map(|n| (n - start.unwrap_or(0)).max(0));
            let task = TaskSummary {
                cost_usd: diff(session.cost_usd, session.cost_at_start),
                duration_secs: ((now - session.started_at) / 1000).max(0),
                lines_added: lines(session.lines_added, session.lines_added_at_start),
                lines_removed: lines(session.lines_removed, session.lines_removed_at_start),
            };
            session.last_task = Some(task.clone());
            Some(task)
        }
        _ => None,
    }
}

/// Applies Codex's `agent-turn-complete` notification.
pub fn apply_codex_notify(
    sessions: &mut Sessions,
    payload: &Value,
    now: i64,
) -> Option<TaskSummary> {
    if payload.get("type").and_then(Value::as_str) != Some("agent-turn-complete") {
        return None;
    }
    let text = |key: &str| payload.get(key).and_then(Value::as_str).unwrap_or("");
    let id = if text("thread-id").is_empty() {
        "codex"
    } else {
        text("thread-id")
    };
    let session = sessions
        .entry(format!("codex-{id}"))
        .or_insert_with(|| Session {
            id: format!("codex-{id}"),
            source: "codex".into(),
            started_at: now,
            ..Session::default()
        });
    session.cwd = text("cwd").to_owned();
    session.project = project_of(&session.cwd);
    session.state = SessionState::Done;
    session.updated_at = now;
    session.activity = None;
    session.last_message = truncate(text("last-assistant-message").trim(), MESSAGE_LIMIT);
    if let Some(prompt) = payload
        .get("input-messages")
        .and_then(Value::as_array)
        .and_then(|m| m.last())
        .and_then(Value::as_str)
    {
        session.last_prompt = truncate(prompt.trim(), PROMPT_LIMIT);
    }
    let task = TaskSummary {
        duration_secs: 0,
        ..TaskSummary::default()
    };
    session.last_task = Some(task.clone());
    Some(task)
}

/// Applies the Claude Code status line: model, cost and line counts.
pub fn apply_status_line(sessions: &mut Sessions, payload: &Value, now: i64) {
    let Some(id) = payload.get("session_id").and_then(Value::as_str) else {
        return;
    };
    let session = sessions.entry(id.to_owned()).or_insert_with(|| Session {
        id: id.to_owned(),
        source: "claude".into(),
        started_at: now,
        ..Session::default()
    });
    session.updated_at = session.updated_at.max(now);
    let number = |v: Option<&Value>| v.and_then(Value::as_f64).filter(|n| n.is_finite());
    let cost = payload.get("cost");
    session.cost_usd = number(cost.and_then(|c| c.get("total_cost_usd"))).or(session.cost_usd);
    session.lines_added = number(cost.and_then(|c| c.get("total_lines_added")))
        .map(|n| n as i64)
        .or(session.lines_added);
    session.lines_removed = number(cost.and_then(|c| c.get("total_lines_removed")))
        .map(|n| n as i64)
        .or(session.lines_removed);
    if let Some(name) = payload
        .get("model")
        .and_then(|m| m.get("display_name").or_else(|| m.get("id")))
        .and_then(Value::as_str)
    {
        session.model = Some(name.to_owned());
    }
    if let Some(cwd) = payload.get("cwd").and_then(Value::as_str) {
        session.cwd = cwd.to_owned();
        session.project = project_of(cwd);
    }
}

pub fn prune(sessions: &mut Sessions, now: i64) {
    sessions.retain(|_, s| now - s.updated_at < EXPIRY_MS);
}

pub fn load() -> Sessions {
    std::fs::read(paths::agent_sessions())
        .ok()
        .and_then(|d| serde_json::from_slice(&d).ok())
        .unwrap_or_default()
}

/// Read-modify-write of the sessions file under an exclusive lock, so hooks
/// from parallel sessions never lose each other's updates.
pub fn update<T>(change: impl FnOnce(&mut Sessions) -> T) -> std::io::Result<T> {
    let path = paths::agent_sessions();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)?;
    file.lock()?;
    let mut data = Vec::new();
    file.read_to_end(&mut data)?;
    let mut sessions: Sessions = serde_json::from_slice(&data).unwrap_or_default();
    let result = change(&mut sessions);
    let bytes = serde_json::to_vec(&sessions)?;
    file.set_len(0)?;
    file.rewind()?;
    file.write_all(&bytes)?;
    file.sync_data()?;
    file.unlock()?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_activity_is_short_and_contentless() {
        let edit = activity_for_tool(
            "Write",
            &json!({"file_path": "/a/src/auth.ts", "file_text": "SECRET"}),
        );
        assert_eq!(
            (edit.kind.as_str(), edit.detail.as_str()),
            ("edit", "auth.ts")
        );
        let run = activity_for_tool("Bash", &json!({"command": "npm   test\n --watch=false"}));
        assert_eq!(run.detail, "npm test --watch=false");
        let long = activity_for_tool("Bash", &json!({"command": "x".repeat(200)}));
        assert_eq!(long.detail.chars().count(), DETAIL_LIMIT);
        assert_eq!(
            activity_for_tool("WebFetch", &json!({"url": "https://docs.rs/tauri/latest"})).detail,
            "docs.rs"
        );
        assert_eq!(
            activity_for_tool("mcp__github__create_issue", &json!({})).detail,
            "create_issue"
        );
    }

    #[test]
    fn claude_task_lifecycle_computes_deltas() {
        let mut sessions = Sessions::new();
        apply_status_line(
            &mut sessions,
            &json!({"session_id": "s1", "model": {"display_name": "Opus"},
            "cost": {"total_cost_usd": 1.0, "total_lines_added": 10, "total_lines_removed": 2}}),
            0,
        );
        apply_claude_hook(
            &mut sessions,
            &json!({"session_id": "s1", "hook_event_name": "UserPromptSubmit",
            "prompt": "Add tests", "cwd": "/w/my-app"}),
            1_000,
        );
        assert_eq!(sessions["s1"].state, SessionState::Working);
        apply_claude_hook(
            &mut sessions,
            &json!({"session_id": "s1", "hook_event_name": "PreToolUse",
            "tool_name": "Bash", "tool_input": {"command": "npm test"}}),
            2_000,
        );
        assert_eq!(sessions["s1"].activity.as_ref().unwrap().kind, "run");
        apply_status_line(
            &mut sessions,
            &json!({"session_id": "s1",
            "cost": {"total_cost_usd": 1.42, "total_lines_added": 166, "total_lines_removed": 25}}),
            3_000,
        );
        let task = apply_claude_hook(
            &mut sessions,
            &json!({"session_id": "s1", "hook_event_name": "Stop",
            "last_assistant_message": "All tests pass."}),
            241_000,
        )
        .unwrap();
        assert_eq!(task.duration_secs, 240);
        assert!((task.cost_usd.unwrap() - 0.42).abs() < 1e-9);
        assert_eq!(
            (task.lines_added, task.lines_removed),
            (Some(156), Some(23))
        );
        let s = &sessions["s1"];
        assert_eq!(
            (s.state, s.project.as_str(), s.model.as_deref()),
            (SessionState::Done, "my-app", Some("Opus"))
        );
        assert_eq!(s.last_prompt, "Add tests");
    }

    #[test]
    fn status_line_session_survives_pruning() {
        let mut sessions = Sessions::new();
        apply_status_line(
            &mut sessions,
            &json!({"session_id": "s", "cost": {"total_cost_usd": 1.0}}),
            1_000,
        );
        prune(&mut sessions, 2_000);
        assert_eq!(sessions["s"].cost_usd, Some(1.0));
        apply_claude_hook(
            &mut sessions,
            &json!({"session_id": "s", "hook_event_name": "UserPromptSubmit", "prompt": "x"}),
            2_000,
        );
        assert_eq!(sessions["s"].cost_at_start, Some(1.0));
    }

    #[test]
    fn waiting_and_subagent_stop() {
        let mut sessions = Sessions::new();
        apply_claude_hook(
            &mut sessions,
            &json!({"session_id": "s", "hook_event_name": "UserPromptSubmit", "prompt": "x"}),
            0,
        );
        assert!(apply_claude_hook(
            &mut sessions,
            &json!({"session_id": "s", "hook_event_name": "Stop", "agent_id": "sub"}),
            1
        )
        .is_none());
        assert_eq!(sessions["s"].state, SessionState::Working);
        apply_claude_hook(
            &mut sessions,
            &json!({"session_id": "s", "hook_event_name": "Notification",
            "notification_type": "permission_prompt"}),
            2,
        );
        assert_eq!(sessions["s"].state, SessionState::Waiting);
    }

    #[test]
    fn codex_turn_and_pruning() {
        let mut sessions = Sessions::new();
        apply_codex_notify(
            &mut sessions,
            &json!({"type": "agent-turn-complete", "thread-id": "t1", "cwd": "/w/api",
            "input-messages": ["first", "fix login"], "last-assistant-message": "Fixed."}),
            5,
        );
        let s = &sessions["codex-t1"];
        assert_eq!(
            (
                s.source.as_str(),
                s.last_prompt.as_str(),
                s.project.as_str()
            ),
            ("codex", "fix login", "api")
        );
        prune(&mut sessions, 5 + EXPIRY_MS + 1);
        assert!(sessions.is_empty());
    }
}
