//! Local usage statistics for the weekly summary ("Wrapped"). Only counts and
//! totals are kept; no prompts or messages.

use std::collections::BTreeMap;
use std::io::{Read, Seek, Write};

use chrono::{Datelike, Duration, Local, NaiveDate};
use serde::{Deserialize, Serialize};

use super::sessions::TaskSummary;
use crate::paths;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DayStats {
    pub claude_tasks: u32,
    pub codex_tasks: u32,
    pub cost_usd: f64,
    pub lines_added: i64,
    pub lines_removed: i64,
    pub busy_seconds: i64,
    pub models: BTreeMap<String, u32>,
    pub projects: BTreeMap<String, u32>,
}

/// Date (YYYY-MM-DD, local time) → totals.
pub type Stats = BTreeMap<String, DayStats>;

pub fn record(
    stats: &mut Stats,
    date: NaiveDate,
    source: &str,
    model: Option<&str>,
    project: &str,
    task: &TaskSummary,
) {
    let day = stats
        .entry(date.format("%Y-%m-%d").to_string())
        .or_default();
    if source == "codex" {
        day.codex_tasks += 1;
    } else {
        day.claude_tasks += 1;
    }
    day.cost_usd += task.cost_usd.unwrap_or(0.0);
    day.lines_added += task.lines_added.unwrap_or(0);
    day.lines_removed += task.lines_removed.unwrap_or(0);
    day.busy_seconds += task.duration_secs;
    if let Some(model) = model.filter(|m| !m.is_empty()) {
        *day.models.entry(model.to_owned()).or_default() += 1;
    }
    if !project.is_empty() {
        *day.projects.entry(project.to_owned()).or_default() += 1;
    }
    // Keep roughly a quarter of history.
    let cutoff = (date - Duration::days(92)).format("%Y-%m-%d").to_string();
    stats.retain(|d, _| d.as_str() >= cutoff.as_str());
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeekSummary {
    pub from: String,
    pub to: String,
    pub claude_tasks: u32,
    pub codex_tasks: u32,
    pub cost_usd: f64,
    pub lines_added: i64,
    pub lines_removed: i64,
    pub busy_hours: f64,
    pub top_model: Option<String>,
    pub top_project: Option<String>,
    /// Weekday number, Monday = 1.
    pub busiest_weekday: Option<u32>,
    /// Tasks per day, oldest first (7 entries).
    pub daily_tasks: Vec<u32>,
}

pub fn week_summary(stats: &Stats, today: NaiveDate) -> WeekSummary {
    let mut summary = WeekSummary {
        from: (today - Duration::days(6)).format("%Y-%m-%d").to_string(),
        to: today.format("%Y-%m-%d").to_string(),
        ..WeekSummary::default()
    };
    let mut models: BTreeMap<&str, u32> = BTreeMap::new();
    let mut projects: BTreeMap<&str, u32> = BTreeMap::new();
    let mut busiest: Option<(u32, u32)> = None;
    let mut busy_seconds = 0;
    for offset in (0..7).rev() {
        let date = today - Duration::days(offset);
        let day = stats.get(&date.format("%Y-%m-%d").to_string());
        let tasks = day.map(|d| d.claude_tasks + d.codex_tasks).unwrap_or(0);
        summary.daily_tasks.push(tasks);
        let Some(day) = day else { continue };
        summary.claude_tasks += day.claude_tasks;
        summary.codex_tasks += day.codex_tasks;
        summary.cost_usd += day.cost_usd;
        summary.lines_added += day.lines_added;
        summary.lines_removed += day.lines_removed;
        busy_seconds += day.busy_seconds;
        for (m, n) in &day.models {
            *models.entry(m).or_default() += n;
        }
        for (p, n) in &day.projects {
            *projects.entry(p).or_default() += n;
        }
        if tasks > 0 && busiest.is_none_or(|(_, best)| tasks > best) {
            busiest = Some((date.weekday().number_from_monday(), tasks));
        }
    }
    let top = |map: BTreeMap<&str, u32>| {
        map.into_iter()
            .max_by_key(|(_, n)| *n)
            .map(|(k, _)| k.to_owned())
    };
    summary.top_model = top(models);
    summary.top_project = top(projects);
    summary.busiest_weekday = busiest.map(|(d, _)| d);
    summary.busy_hours = (busy_seconds as f64 / 3600.0 * 10.0).round() / 10.0;
    summary
}

pub fn load() -> Stats {
    std::fs::read(paths::stats())
        .ok()
        .and_then(|d| serde_json::from_slice(&d).ok())
        .unwrap_or_default()
}

pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// Records a finished task under an exclusive lock.
pub fn record_now(
    source: &str,
    model: Option<&str>,
    project: &str,
    task: &TaskSummary,
) -> std::io::Result<()> {
    let path = paths::stats();
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
    let mut stats: Stats = serde_json::from_slice(&data).unwrap_or_default();
    record(&mut stats, today(), source, model, project, task);
    let bytes = serde_json::to_vec(&stats)?;
    file.set_len(0)?;
    file.rewind()?;
    file.write_all(&bytes)?;
    file.unlock()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(cost: f64, added: i64, secs: i64) -> TaskSummary {
        TaskSummary {
            cost_usd: Some(cost),
            duration_secs: secs,
            lines_added: Some(added),
            lines_removed: Some(1),
        }
    }

    #[test]
    fn week_summary_aggregates_last_seven_days() {
        let today = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap(); // Wednesday
        let mut stats = Stats::new();
        record(
            &mut stats,
            today,
            "claude",
            Some("Opus"),
            "my-app",
            &task(0.5, 100, 1800),
        );
        record(
            &mut stats,
            today,
            "claude",
            Some("Opus"),
            "my-app",
            &task(0.25, 20, 1800),
        );
        record(
            &mut stats,
            today - Duration::days(1),
            "codex",
            Some("GPT"),
            "api",
            &task(0.0, 0, 600),
        );
        record(
            &mut stats,
            today - Duration::days(10),
            "claude",
            Some("Sonnet"),
            "old",
            &task(9.0, 9, 9),
        );
        let week = week_summary(&stats, today);
        assert_eq!((week.claude_tasks, week.codex_tasks), (2, 1));
        assert!((week.cost_usd - 0.75).abs() < 1e-9);
        assert_eq!(week.lines_added, 120);
        assert_eq!(week.top_model.as_deref(), Some("Opus"));
        assert_eq!(week.top_project.as_deref(), Some("my-app"));
        assert_eq!(week.busiest_weekday, Some(3));
        assert_eq!(week.daily_tasks, vec![0, 0, 0, 0, 0, 1, 2]);
        assert_eq!(week.busy_hours, 1.2);
        assert_eq!(week.from, "2026-09-24");
    }

    #[test]
    fn old_history_is_pruned() {
        let today = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();
        let mut stats = Stats::new();
        record(
            &mut stats,
            today - Duration::days(200),
            "claude",
            None,
            "",
            &task(1.0, 1, 1),
        );
        record(&mut stats, today, "claude", None, "", &task(1.0, 1, 1));
        assert_eq!(stats.len(), 1);
    }
}
