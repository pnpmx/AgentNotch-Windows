//! Port of the macOS app's UsageParser. Labels are not stored: the UI derives
//! them from stable ids so they follow the interface language.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UsageSource {
    Codex,
    Claude,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    pub id: String,
    /// Raw limit name for windows without a known duration (Codex buckets).
    pub name: String,
    pub used_percent: f64,
    /// Unix seconds.
    pub resets_at: Option<i64>,
    pub duration_minutes: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSnapshot {
    pub source: UsageSource,
    pub windows: Vec<UsageWindow>,
    /// Unix seconds.
    pub fetched_at: i64,
    pub plan: Option<String>,
    pub origin: String,
}

/// Live Claude Code session details from the status line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub model: String,
    pub effort: Option<String>,
    pub cost_usd: Option<f64>,
    pub context_percent: Option<f64>,
    pub project: String,
    /// Unix seconds.
    pub updated_at: i64,
}

pub fn parse_claude_session(root: &Value, now: i64) -> Option<SessionInfo> {
    let model = root.get("model")?;
    let name = model
        .get("display_name")
        .or_else(|| model.get("id"))
        .and_then(Value::as_str)?
        .to_owned();
    let project = root
        .get("workspace")
        .and_then(|w| w.get("project_dir").or_else(|| w.get("current_dir")))
        .or_else(|| root.get("cwd"))
        .and_then(Value::as_str)
        .map(|p| {
            p.trim_end_matches(['/', '\\'])
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or("")
                .to_owned()
        })
        .unwrap_or_default();
    Some(SessionInfo {
        model: name,
        effort: root
            .get("effort")
            .and_then(|e| e.get("level"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        cost_usd: number(root.get("cost").and_then(|c| c.get("total_cost_usd")))
            .filter(|c| *c >= 0.0),
        context_percent: number(
            root.get("context_window")
                .and_then(|c| c.get("used_percentage")),
        )
        .map(clamp_percent),
        project,
        updated_at: now,
    })
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ParseError {
    #[error("malformed payload")]
    Malformed,
    #[error("missing rate limits")]
    MissingRateLimits,
}

fn number(value: Option<&Value>) -> Option<f64> {
    value.and_then(Value::as_f64).filter(|v| v.is_finite())
}

fn epoch(value: Option<&Value>) -> Option<i64> {
    number(value)
        .filter(|s| *s > 0.0 && *s <= 253_402_300_799.0)
        .map(|s| s as i64)
}

fn clamp_percent(v: f64) -> f64 {
    v.clamp(0.0, 100.0)
}

pub fn parse_codex_response(root: &Value, now: i64) -> Result<UsageSnapshot, ParseError> {
    let result = root
        .get("result")
        .and_then(Value::as_object)
        .ok_or(ParseError::Malformed)?;
    let plan = result
        .get("rateLimits")
        .and_then(|r| r.get("planType"))
        .and_then(Value::as_str)
        .map(str::to_owned);

    let mut windows = Vec::new();
    if let Some(buckets) = result.get("rateLimitsByLimitId").and_then(Value::as_object) {
        let mut keys: Vec<&String> = buckets.keys().collect();
        keys.sort_by(|a, b| match (a.as_str(), b.as_str()) {
            ("codex", "codex") => std::cmp::Ordering::Equal,
            ("codex", _) => std::cmp::Ordering::Less,
            (_, "codex") => std::cmp::Ordering::Greater,
            _ => a.cmp(b),
        });
        for key in keys {
            if let Some(bucket) = buckets[key].as_object() {
                windows.extend(parse_codex_bucket(bucket, key));
            }
        }
    }
    if windows.is_empty() {
        if let Some(bucket) = result.get("rateLimits").and_then(Value::as_object) {
            windows = parse_codex_bucket(bucket, "Codex");
        }
    }
    if windows.is_empty() {
        return Err(ParseError::MissingRateLimits);
    }
    Ok(UsageSnapshot {
        source: UsageSource::Codex,
        windows,
        fetched_at: now,
        plan,
        origin: "Codex CLI".into(),
    })
}

fn parse_codex_bucket(bucket: &serde_json::Map<String, Value>, fallback: &str) -> Vec<UsageWindow> {
    let name = bucket
        .get("limitName")
        .and_then(Value::as_str)
        .or_else(|| bucket.get("limitId").and_then(Value::as_str))
        .unwrap_or(fallback)
        .to_owned();
    ["primary", "secondary"]
        .iter()
        .filter_map(|key| {
            let value = bucket.get(*key)?.as_object()?;
            let used = number(value.get("usedPercent"))?;
            let minutes = number(value.get("windowDurationMins"))
                .filter(|m| *m > 0.0 && *m <= 5_256_000.0)
                .map(|m| m as i64);
            Some(UsageWindow {
                id: format!("codex-{name}-{key}"),
                name: name.clone(),
                used_percent: clamp_percent(used),
                resets_at: epoch(value.get("resetsAt")),
                duration_minutes: minutes,
            })
        })
        .collect()
}

pub fn parse_claude_status_line(root: &Value, now: i64) -> Result<UsageSnapshot, ParseError> {
    let root = root.as_object().ok_or(ParseError::Malformed)?;
    let limits = root
        .get("rate_limits")
        .and_then(Value::as_object)
        .ok_or(ParseError::MissingRateLimits)?;
    let windows: Vec<UsageWindow> = ["five_hour", "seven_day", "spend_limit"]
        .iter()
        .filter_map(|key| {
            let value = limits.get(*key)?.as_object()?;
            let used = number(value.get("used_percentage"))?;
            Some(UsageWindow {
                id: format!("claude-{key}"),
                name: (*key).to_owned(),
                used_percent: clamp_percent(used),
                resets_at: epoch(value.get("resets_at")),
                duration_minutes: None,
            })
        })
        .collect();
    if windows.is_empty() {
        return Err(ParseError::MissingRateLimits);
    }
    Ok(UsageSnapshot {
        source: UsageSource::Claude,
        windows,
        fetched_at: now,
        plan: None,
        origin: "Claude Code".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn codex_buckets_sort_codex_first() {
        let payload = json!({"result": {"rateLimitsByLimitId": {
            "zeta": {"primary": {"usedPercent": 5}},
            "codex": {
                "primary": {"usedPercent": 25, "windowDurationMins": 300, "resetsAt": 1_800_000_000},
                "secondary": {"usedPercent": 40, "windowDurationMins": 10_080, "resetsAt": 1_800_100_000}
            }
        }, "rateLimits": {"planType": "plus"}}});
        let snap = parse_codex_response(&payload, 1_700_000_000).unwrap();
        assert_eq!(snap.windows.len(), 3);
        assert_eq!(snap.windows[0].id, "codex-codex-primary");
        assert_eq!(snap.windows[0].duration_minutes, Some(300));
        assert_eq!(snap.windows[1].used_percent, 40.0);
        assert_eq!(snap.plan.as_deref(), Some("plus"));
    }

    #[test]
    fn codex_empty_buckets_fall_back_and_invalid_numbers_clamp() {
        let payload = json!({"result": {"rateLimitsByLimitId": {}, "rateLimits": {
            "primary": {"usedPercent": 120, "windowDurationMins": -1, "resetsAt": -1}
        }}});
        let snap = parse_codex_response(&payload, 0).unwrap();
        assert_eq!(snap.windows[0].used_percent, 100.0);
        assert_eq!(snap.windows[0].duration_minutes, None);
        assert_eq!(snap.windows[0].resets_at, None);
    }

    #[test]
    fn codex_without_limits_errors() {
        assert_eq!(
            parse_codex_response(&json!({"result": {}}), 0),
            Err(ParseError::MissingRateLimits)
        );
        assert_eq!(
            parse_codex_response(&json!([]), 0),
            Err(ParseError::Malformed)
        );
    }

    #[test]
    fn claude_status_line() {
        let payload = json!({"rate_limits": {
            "five_hour": {"used_percentage": 23.5, "resets_at": 1_800_000_000},
            "seven_day": {"used_percentage": 41.2, "resets_at": 1_800_100_000}
        }});
        let snap = parse_claude_status_line(&payload, 1_700_000_000).unwrap();
        assert_eq!(snap.windows.len(), 2);
        assert_eq!(snap.windows[0].id, "claude-five_hour");
        assert_eq!(snap.windows[1].used_percent, 41.2);
    }

    #[test]
    fn claude_session_details() {
        let payload = json!({
            "model": {"id": "claude-opus-5-5", "display_name": "Opus"},
            "workspace": {"project_dir": "/home/u/my-app"},
            "cost": {"total_cost_usd": 1.4},
            "context_window": {"used_percentage": 62},
            "effort": {"level": "high"}
        });
        let s = parse_claude_session(&payload, 10).unwrap();
        assert_eq!(
            (s.model.as_str(), s.effort.as_deref(), s.project.as_str()),
            ("Opus", Some("high"), "my-app")
        );
        assert_eq!((s.cost_usd, s.context_percent), (Some(1.4), Some(62.0)));
        let early = json!({"model": {"id": "claude-sonnet-5"}, "context_window": {"used_percentage": null}});
        let s = parse_claude_session(&early, 0).unwrap();
        assert_eq!(
            (s.model.as_str(), s.context_percent),
            ("claude-sonnet-5", None)
        );
        assert!(parse_claude_session(&json!({}), 0).is_none());
    }

    #[test]
    fn claude_without_limits_errors() {
        assert_eq!(
            parse_claude_status_line(&json!({"model": {}}), 0),
            Err(ParseError::MissingRateLimits)
        );
    }
}
