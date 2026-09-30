//! Agent activity events. Claude Code hooks and Codex's `notify` program both
//! run `AgentNotch --agent-event <source>`; the payload is normalised here and
//! appended to a small file the widget watches.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::paths;

pub const EVENT_FLAG: &str = "--agent-event";
const KEEP: usize = 20;
const MESSAGE_CHARS: usize = 160;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EventKind {
    /// The agent finished its turn.
    Done,
    /// The agent asks to run a tool and waits for approval.
    Permission,
    /// The agent is idle or otherwise waiting for the user.
    NeedsInput,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEvent {
    /// "claude" or "codex".
    pub source: String,
    pub kind: EventKind,
    /// Short text: the permission request or the start of the final answer.
    pub message: String,
    /// Last path component of the working directory.
    pub project: String,
    /// Unix milliseconds; also used as an id.
    pub at: i64,
}

fn project_name(cwd: Option<&str>) -> String {
    cwd.map(|c| c.trim_end_matches(['/', '\\']))
        .and_then(|c| c.rsplit(['/', '\\']).next())
        .unwrap_or("")
        .to_owned()
}

fn short(text: &str) -> String {
    let single_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if single_line.chars().count() <= MESSAGE_CHARS {
        return single_line;
    }
    let cut: String = single_line.chars().take(MESSAGE_CHARS - 1).collect();
    format!("{}…", cut.trim_end())
}

/// Claude Code hook payload (stdin) → event. Returns `None` for events that
/// should not interrupt the user, such as subagent completions or auth notices.
pub fn from_claude_hook(payload: &Value, at: i64) -> Option<AgentEvent> {
    let text = |key: &str| payload.get(key).and_then(Value::as_str);
    let (kind, message) = match text("hook_event_name")? {
        "Stop" => {
            if payload.get("agent_id").is_some() {
                return None; // A subagent finished; the main turn continues.
            }
            (
                EventKind::Done,
                text("last_assistant_message").unwrap_or(""),
            )
        }
        "Notification" => {
            let kind = match text("notification_type").unwrap_or("") {
                "permission_prompt" => EventKind::Permission,
                "idle_prompt"
                | "agent_needs_input"
                | "elicitation_dialog"
                | "elicitation_url_dialog" => EventKind::NeedsInput,
                _ => return None,
            };
            (kind, text("message").unwrap_or(""))
        }
        _ => return None,
    };
    Some(AgentEvent {
        source: "claude".into(),
        kind,
        message: short(message),
        project: project_name(text("cwd")),
        at,
    })
}

/// Codex `notify` payload (last argv argument, JSON) → event.
pub fn from_codex_notify(raw: &str, at: i64) -> Option<AgentEvent> {
    let payload: Value = serde_json::from_str(raw).ok()?;
    let text = |key: &str| payload.get(key).and_then(Value::as_str);
    let kind = match text("type")? {
        "agent-turn-complete" => EventKind::Done,
        "approval-requested" => EventKind::Permission,
        _ => return None,
    };
    Some(AgentEvent {
        source: "codex".into(),
        kind,
        message: short(text("last-assistant-message").unwrap_or("")),
        project: project_name(text("cwd")),
        at,
    })
}

pub fn load() -> Vec<AgentEvent> {
    std::fs::read(paths::agent_events())
        .ok()
        .and_then(|d| serde_json::from_slice(&d).ok())
        .unwrap_or_default()
}

pub fn append(event: AgentEvent) -> std::io::Result<()> {
    let mut events = load();
    events.push(event);
    let excess = events.len().saturating_sub(KEEP);
    events.drain(..excess);
    paths::write_atomic(&paths::agent_events(), &serde_json::to_vec(&events)?)
}

/// Entry point for `--agent-event claude|codex`. Hooks must never block or
/// fail an agent, so this always exits 0 and prints nothing.
pub fn run(args: &[String]) -> i32 {
    let source = args
        .iter()
        .position(|a| a == EVENT_FLAG)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str);
    let at = paths::now_millis();
    let event = match source {
        Some("claude") => {
            use std::io::Read;
            let mut input = Vec::new();
            let _ = std::io::stdin().read_to_end(&mut input);
            serde_json::from_slice::<Value>(&input)
                .ok()
                .and_then(|v| from_claude_hook(&v, at))
        }
        // Codex appends the JSON payload as the final argument.
        Some("codex") => args.last().and_then(|raw| from_codex_notify(raw, at)),
        _ => None,
    };
    if let Some(event) = event {
        let _ = append(event);
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn claude_stop_uses_last_message_and_project() {
        let payload = json!({
            "hook_event_name": "Stop",
            "cwd": "/home/user/my-app/",
            "last_assistant_message": "I've fixed   the failing\ntests."
        });
        let event = from_claude_hook(&payload, 5).unwrap();
        assert_eq!(event.kind, EventKind::Done);
        assert_eq!(event.message, "I've fixed the failing tests.");
        assert_eq!(event.project, "my-app");
    }

    #[test]
    fn claude_subagent_stop_is_ignored() {
        let payload = json!({"hook_event_name": "Stop", "agent_id": "sub-1"});
        assert!(from_claude_hook(&payload, 0).is_none());
    }

    #[test]
    fn claude_notifications_map_to_kinds() {
        let permission = json!({"hook_event_name": "Notification", "notification_type": "permission_prompt",
            "message": "Claude wants to run: Bash - npm test", "cwd": "C:\\work\\api"});
        let event = from_claude_hook(&permission, 0).unwrap();
        assert_eq!(event.kind, EventKind::Permission);
        assert_eq!(event.project, "api");
        let idle = json!({"hook_event_name": "Notification", "notification_type": "idle_prompt"});
        assert_eq!(
            from_claude_hook(&idle, 0).unwrap().kind,
            EventKind::NeedsInput
        );
        let auth = json!({"hook_event_name": "Notification", "notification_type": "auth_success"});
        assert!(from_claude_hook(&auth, 0).is_none());
    }

    #[test]
    fn codex_turn_complete() {
        let raw = r#"{"type":"agent-turn-complete","thread-id":"t","cwd":"/src/web","last-assistant-message":"Done."}"#;
        let event = from_codex_notify(raw, 1).unwrap();
        assert_eq!(
            (event.source.as_str(), event.kind, event.project.as_str()),
            ("codex", EventKind::Done, "web")
        );
        assert!(from_codex_notify("not json", 1).is_none());
        assert!(from_codex_notify(r#"{"type":"other"}"#, 1).is_none());
    }

    #[test]
    fn long_messages_are_shortened() {
        let long = "word ".repeat(100);
        let event = from_claude_hook(
            &json!({"hook_event_name": "Stop", "last_assistant_message": long}),
            0,
        )
        .unwrap();
        assert_eq!(event.message.chars().count(), MESSAGE_CHARS);
        assert!(event.message.ends_with('…'));
    }
}
