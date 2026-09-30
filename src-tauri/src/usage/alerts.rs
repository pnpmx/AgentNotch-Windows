//! Limit alerts and pace projection. Pure logic: fed with usage snapshots,
//! it reports thresholds crossed, resets, and when 100% would be reached at
//! the current pace.

use std::collections::HashMap;

use serde::Serialize;

use super::parser::UsageSnapshot;

pub const THRESHOLDS: [u8; 2] = [80, 95];
/// Only recent samples describe the current pace.
const WINDOW_SECONDS: i64 = 45 * 60;
const MIN_SPAN_SECONDS: i64 = 5 * 60;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LimitAlert {
    Threshold {
        window_id: String,
        percent: u8,
    },
    Reset {
        window_id: String,
    },
    /// A window that had run out (95%+) is available again at its reset time.
    Available {
        window_id: String,
    },
}

#[derive(Default)]
struct Track {
    samples: Vec<(i64, f64)>,
    resets_at: Option<i64>,
    notified: u8,
    exhausted: bool,
}

#[derive(Default)]
pub struct Tracker {
    tracks: HashMap<String, Track>,
}

impl Tracker {
    pub fn update(&mut self, snapshot: &UsageSnapshot) -> Vec<LimitAlert> {
        let now = snapshot.fetched_at;
        let mut alerts = Vec::new();
        for window in &snapshot.windows {
            let track = self.tracks.entry(window.id.clone()).or_default();
            let last = track.samples.last().map(|(_, p)| *p);
            let new_period = match (track.resets_at, window.resets_at) {
                (Some(old), Some(new)) => new > old + 60,
                _ => false,
            } || last.is_some_and(|p| window.used_percent + 15.0 < p);
            if new_period {
                if track.notified > 0 || last.is_some_and(|p| p >= 50.0) {
                    alerts.push(LimitAlert::Reset {
                        window_id: window.id.clone(),
                    });
                }
                *track = Track::default();
            }
            track.resets_at = window.resets_at;
            if window.used_percent >= 95.0 {
                track.exhausted = true;
            }
            if track.samples.last().map(|(t, _)| *t) != Some(now) {
                track.samples.push((now, window.used_percent));
            }
            track.samples.retain(|(t, _)| now - t <= WINDOW_SECONDS);
            // Warn once per level; a first reading already above a level
            // counts as crossing it.
            if let Some(level) = THRESHOLDS
                .iter()
                .rev()
                .find(|l| window.used_percent >= f64::from(**l) && track.notified < **l)
            {
                track.notified = *level;
                alerts.push(LimitAlert::Threshold {
                    window_id: window.id.clone(),
                    percent: *level,
                });
            }
        }
        alerts
    }

    /// Windows that were exhausted and whose reset time has now passed. Each
    /// fires once; the next reading starts a fresh period.
    pub fn due_available(&mut self, now: i64) -> Vec<LimitAlert> {
        self.tracks
            .iter_mut()
            .filter(|(_, t)| t.exhausted && t.resets_at.is_some_and(|r| r <= now))
            .map(|(id, t)| {
                t.exhausted = false;
                LimitAlert::Available {
                    window_id: id.clone(),
                }
            })
            .collect()
    }

    /// Unix seconds when the window would reach 100% at the recent pace,
    /// if that happens before it resets.
    pub fn projection(&self, window_id: &str) -> Option<i64> {
        let track = self.tracks.get(window_id)?;
        let (eta, _) = project(&track.samples)?;
        match track.resets_at {
            Some(reset) if eta >= reset => None,
            _ => Some(eta),
        }
    }

    pub fn projections(&self) -> HashMap<String, i64> {
        self.tracks
            .keys()
            .filter_map(|id| self.projection(id).map(|eta| (id.clone(), eta)))
            .collect()
    }
}

/// Least-squares slope over the samples; returns (eta, percent per hour).
fn project(samples: &[(i64, f64)]) -> Option<(i64, f64)> {
    let (first, last) = (samples.first()?, samples.last()?);
    if samples.len() < 3 || last.0 - first.0 < MIN_SPAN_SECONDS || last.1 >= 100.0 {
        return None;
    }
    let n = samples.len() as f64;
    let mean_t = samples
        .iter()
        .map(|(t, _)| (*t - first.0) as f64)
        .sum::<f64>()
        / n;
    let mean_p = samples.iter().map(|(_, p)| *p).sum::<f64>() / n;
    let (mut num, mut den) = (0.0, 0.0);
    for (t, p) in samples {
        let dt = (*t - first.0) as f64 - mean_t;
        num += dt * (p - mean_p);
        den += dt * dt;
    }
    if den == 0.0 {
        return None;
    }
    let per_second = num / den;
    if per_second <= 0.0 {
        return None;
    }
    let eta = last.0 + ((100.0 - last.1) / per_second).round() as i64;
    Some((eta, per_second * 3600.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::parser::{UsageSource, UsageWindow};

    fn snap(at: i64, percent: f64, resets_at: i64) -> UsageSnapshot {
        UsageSnapshot {
            source: UsageSource::Claude,
            windows: vec![UsageWindow {
                id: "claude-five_hour".into(),
                name: "five_hour".into(),
                used_percent: percent,
                resets_at: Some(resets_at),
                duration_minutes: None,
            }],
            fetched_at: at,
            plan: None,
            origin: "Claude Code".into(),
        }
    }

    #[test]
    fn thresholds_fire_once_each() {
        let mut t = Tracker::default();
        assert!(t.update(&snap(0, 50.0, 10_000)).is_empty());
        assert_eq!(
            t.update(&snap(60, 81.0, 10_000)),
            vec![LimitAlert::Threshold {
                window_id: "claude-five_hour".into(),
                percent: 80
            }]
        );
        assert!(t.update(&snap(120, 85.0, 10_000)).is_empty());
        assert_eq!(t.update(&snap(180, 96.0, 10_000)).len(), 1);
        assert!(t.update(&snap(240, 97.0, 10_000)).is_empty());
    }

    #[test]
    fn first_reading_above_95_reports_only_95() {
        let mut t = Tracker::default();
        let alerts = t.update(&snap(0, 97.0, 10_000));
        assert_eq!(
            alerts,
            vec![LimitAlert::Threshold {
                window_id: "claude-five_hour".into(),
                percent: 95
            }]
        );
    }

    #[test]
    fn reset_is_reported_and_thresholds_rearm() {
        let mut t = Tracker::default();
        t.update(&snap(0, 90.0, 10_000));
        let alerts = t.update(&snap(60, 2.0, 28_000));
        assert_eq!(
            alerts,
            vec![LimitAlert::Reset {
                window_id: "claude-five_hour".into()
            }]
        );
        assert_eq!(t.update(&snap(120, 82.0, 28_000)).len(), 1);
    }

    #[test]
    fn exhausted_window_announces_availability_once() {
        let mut t = Tracker::default();
        t.update(&snap(0, 97.0, 5_000));
        assert!(t.due_available(4_999).is_empty());
        assert_eq!(
            t.due_available(5_000),
            vec![LimitAlert::Available {
                window_id: "claude-five_hour".into()
            }]
        );
        assert!(t.due_available(6_000).is_empty());
    }

    #[test]
    fn projection_extrapolates_recent_pace() {
        let mut t = Tracker::default();
        // 10 percentage points per 10 minutes, from 40%.
        for i in 0..=3 {
            t.update(&snap(i * 600, 40.0 + i as f64 * 10.0, 100_000));
        }
        // At 70% after 1800 s; 30 more points take 1800 s.
        assert_eq!(t.projection("claude-five_hour"), Some(3600));
    }

    #[test]
    fn no_projection_when_reset_comes_first_or_flat() {
        let mut t = Tracker::default();
        for i in 0..=3 {
            t.update(&snap(i * 600, 40.0 + i as f64, 2_500));
        }
        assert_eq!(t.projection("claude-five_hour"), None);
        let mut flat = Tracker::default();
        for i in 0..=3 {
            flat.update(&snap(i * 600, 40.0, 100_000));
        }
        assert_eq!(flat.projection("claude-five_hour"), None);
    }
}
