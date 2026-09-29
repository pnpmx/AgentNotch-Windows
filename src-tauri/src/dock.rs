//! Edge-docking geometry. Pure functions in physical pixels so they can be
//! unit-tested without a window system.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

impl Edge {
    pub fn is_vertical(self) -> bool {
        matches!(self, Edge::Left | Edge::Right)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

/// Logical sizes. The collapsed tab lies along the edge; the expanded panel
/// has a fixed size and grows away from the edge.
pub const TAB_THICKNESS: f64 = 50.0;
pub const TAB_LENGTH: f64 = 176.0;
pub const PANEL_WIDTH: f64 = 340.0;
pub const PANEL_HEIGHT: f64 = 420.0;

pub fn logical_size(edge: Edge, expanded: bool) -> (f64, f64) {
    match (expanded, edge.is_vertical()) {
        (true, _) => (PANEL_WIDTH, PANEL_HEIGHT),
        (false, true) => (TAB_THICKNESS, TAB_LENGTH),
        (false, false) => (TAB_LENGTH, TAB_THICKNESS),
    }
}

pub fn physical_size(edge: Edge, expanded: bool, scale: f64) -> (i32, i32) {
    let (w, h) = logical_size(edge, expanded);
    ((w * scale).round() as i32, (h * scale).round() as i32)
}

/// The edge closest to a point, measured against the work area.
pub fn nearest_edge(area: Rect, px: i32, py: i32) -> Edge {
    let distances = [
        (Edge::Left, (px - area.x).abs()),
        (Edge::Right, (area.x + area.w - px).abs()),
        (Edge::Top, (py - area.y).abs()),
        (Edge::Bottom, (area.y + area.h - py).abs()),
    ];
    distances
        .iter()
        .min_by_key(|(_, d)| *d)
        .map(|(e, _)| *e)
        .unwrap_or(Edge::Right)
}

/// Position of a point along an edge as a 0...1 fraction, so a dock position
/// survives resolution and monitor changes.
pub fn offset_along(area: Rect, edge: Edge, px: i32, py: i32) -> f64 {
    let (start, len, p) = if edge.is_vertical() {
        (area.y, area.h, py)
    } else {
        (area.x, area.w, px)
    };
    if len <= 0 {
        return 0.5;
    }
    ((p - start) as f64 / len as f64).clamp(0.0, 1.0)
}

/// Top-left position for a window of `w`×`h` docked to `edge`, centred on
/// `offset` and clamped inside the work area.
pub fn place(area: Rect, edge: Edge, offset: f64, w: i32, h: i32) -> (i32, i32) {
    let offset = if offset.is_finite() {
        offset.clamp(0.0, 1.0)
    } else {
        0.5
    };
    let clamp = |v: i32, lo: i32, hi: i32| if hi < lo { lo } else { v.clamp(lo, hi) };
    match edge {
        Edge::Left | Edge::Right => {
            let centre = area.y + (offset * area.h as f64).round() as i32;
            let y = clamp(centre - h / 2, area.y, area.y + area.h - h);
            let x = if edge == Edge::Left {
                area.x
            } else {
                area.x + area.w - w
            };
            (x, y)
        }
        Edge::Top | Edge::Bottom => {
            let centre = area.x + (offset * area.w as f64).round() as i32;
            let x = clamp(centre - w / 2, area.x, area.x + area.w - w);
            let y = if edge == Edge::Top {
                area.y
            } else {
                area.y + area.h - h
            };
            (x, y)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect {
        x: 0,
        y: 0,
        w: 1920,
        h: 1040,
    };

    #[test]
    fn nearest_edge_picks_closest_side() {
        assert_eq!(nearest_edge(AREA, 10, 500), Edge::Left);
        assert_eq!(nearest_edge(AREA, 1900, 500), Edge::Right);
        assert_eq!(nearest_edge(AREA, 900, 5), Edge::Top);
        assert_eq!(nearest_edge(AREA, 900, 1030), Edge::Bottom);
    }

    #[test]
    fn place_clamps_inside_work_area() {
        let (w, h) = physical_size(Edge::Right, false, 1.0);
        assert_eq!(place(AREA, Edge::Right, 0.0, w, h), (1920 - w, 0));
        assert_eq!(place(AREA, Edge::Right, 1.0, w, h), (1920 - w, 1040 - h));
        assert_eq!(place(AREA, Edge::Left, 0.5, w, h), (0, 520 - h / 2));
        let (w, h) = physical_size(Edge::Bottom, false, 1.0);
        assert_eq!(
            place(AREA, Edge::Bottom, 0.5, w, h),
            (960 - w / 2, 1040 - h)
        );
    }

    #[test]
    fn expanded_panel_grows_away_from_edge() {
        let (w, h) = physical_size(Edge::Right, true, 1.5);
        let (x, y) = place(AREA, Edge::Right, 0.5, w, h);
        assert_eq!(x + w, 1920);
        assert!(y >= 0 && y + h <= 1040);
    }

    #[test]
    fn secondary_monitor_with_negative_origin() {
        let area = Rect {
            x: -1280,
            y: -200,
            w: 1280,
            h: 1024,
        };
        assert_eq!(nearest_edge(area, -1270, 300), Edge::Left);
        let (x, _) = place(area, Edge::Left, 0.5, 50, 176);
        assert_eq!(x, -1280);
        assert!((offset_along(area, Edge::Left, -1270, 312) - 0.5).abs() < 0.01);
    }

    #[test]
    fn invalid_offset_is_centred() {
        assert_eq!(place(AREA, Edge::Top, f64::NAN, 100, 50), (910, 0));
    }

    #[test]
    fn window_larger_than_area_stays_at_origin() {
        let tiny = Rect {
            x: 0,
            y: 0,
            w: 200,
            h: 200,
        };
        assert_eq!(place(tiny, Edge::Left, 0.5, 340, 420), (0, 0));
    }
}
