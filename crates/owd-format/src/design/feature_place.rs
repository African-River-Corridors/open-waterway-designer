//! Where a building or a place stands against the design (BLD-1): its distance and chainage on the
//! design line, and its place across the channel's cut, read from the channel's own stations (the
//! volumes stage's, when they are fresh: see `channel.rs`).
//!
//! # The cut test
//!
//! A station's section runs square to the line at its heading. A point belongs to the station
//! whose section it is nearest along the line (`|along| <= half the station spacing`, or the
//! nearest one of the stations within reach). Its offset is measured across, left negative, as
//! the channel's offsets are. The cut's footprint at that station runs from the left daylight
//! offset to the right one; a side with no daylight (no cut there, or no ground read) runs to the
//! water's edge. A building is TAKEN when its footprint reaches inside: the building is a disc of
//! its own area about its point (the footprint itself is not kept), so it is taken when its offset
//! is within `radius` of the footprint. It is NEAR when it is not taken and within `near_m` of the
//! footprint. A point further than the cut's widest reach plus `near_m` along no section is off the
//! channel.

use super::channel::Channel;
use owd_core::geom::Pt;
use std::collections::HashMap;

/// The nearest point of a polyline: `(distance, chainage along it)`.
pub fn line_distance(line: &[Pt], p: Pt) -> (f64, f64) {
    let mut best = (f64::INFINITY, f64::NAN);
    let mut run = 0.0;
    for w in line.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let l2 = dx * dx + dy * dy;
        let t = if l2 > 0.0 {
            (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let d = libm::hypot(p[0] - (a[0] + t * dx), p[1] - (a[1] + t * dy));
        if d < best.0 {
            best = (d, run + t * l2.sqrt());
        }
        run += l2.sqrt();
    }
    best
}

/// A point's place across the cut.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CutPlace {
    /// The chainage of the station whose section the point is on.
    pub chainage_m: f64,
    /// The point's offset across the line, left negative.
    pub offset_m: f64,
    /// The cut's footprint at that station: from `left_m` to `right_m` (left negative).
    pub left_m: f64,
    pub right_m: f64,
    /// How far the point is outside the footprint, metres; 0 inside it.
    pub outside_m: f64,
    /// The footprint is the cut's daylight lines (the volumes' own) on both sides, not the water's
    /// edge.
    pub daylight: bool,
}

/// The channel's stations, bucketed for a quick search.
pub struct CutIndex<'a> {
    channel: &'a Channel,
    cell: f64,
    buckets: HashMap<(i64, i64), Vec<u32>>,
    /// The widest the footprint reaches from the line anywhere.
    reach_m: f64,
    spacing_m: f64,
}

impl<'a> CutIndex<'a> {
    pub fn new(channel: &'a Channel) -> CutIndex<'a> {
        let st = &channel.stations;
        let spacing_m = if st.len() > 1 {
            ((st[st.len() - 1].chainage_m - st[0].chainage_m) / (st.len() - 1) as f64).max(1.0)
        } else {
            25.0
        };
        let mut reach_m: f64 = 0.0;
        for i in 0..st.len() {
            let (l, r) = Self::limits(channel, i);
            reach_m = reach_m.max(-l).max(r);
        }
        let cell = (reach_m + spacing_m).max(50.0);
        let mut buckets: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
        for (i, s) in st.iter().enumerate() {
            let k = ((s.x / cell).floor() as i64, (s.y / cell).floor() as i64);
            buckets.entry(k).or_default().push(i as u32);
        }
        CutIndex {
            channel,
            cell,
            buckets,
            reach_m,
            spacing_m,
        }
    }

    fn limits(channel: &Channel, i: usize) -> (f64, f64) {
        let s = &channel.stations[i];
        let edge = s.water_edge_m(channel.wet_slope).max(s.half_width_m);
        (
            s.daylight_left_m.unwrap_or(-edge).min(-s.half_width_m),
            s.daylight_right_m.unwrap_or(edge).max(s.half_width_m),
        )
    }

    /// The place of `p` across the cut, or `None` when it is on no station's section within
    /// `extra_m` beyond the footprint.
    pub fn locate(&self, p: Pt, extra_m: f64) -> Option<CutPlace> {
        let reach = self.reach_m + extra_m + self.spacing_m;
        let r = (reach / self.cell).ceil() as i64;
        let (cx, cy) = (
            (p[0] / self.cell).floor() as i64,
            (p[1] / self.cell).floor() as i64,
        );
        let mut best: Option<(f64, usize, f64)> = None;
        for gx in cx - r..=cx + r {
            for gy in cy - r..=cy + r {
                let Some(list) = self.buckets.get(&(gx, gy)) else {
                    continue;
                };
                for &i in list {
                    let s = &self.channel.stations[i as usize];
                    let (dx, dy) = (p[0] - s.x, p[1] - s.y);
                    let (c, n) = (libm::cos(s.heading_rad), libm::sin(s.heading_rad));
                    let along = dx * c + dy * n;
                    let off = -dx * n + dy * c;
                    if along.abs() > self.spacing_m {
                        continue;
                    }
                    // The station whose section is nearest along the line; a tie to the nearer one.
                    let key = along.abs() + 1e-9 * off.abs();
                    if best.is_none_or(|b| key < b.0) {
                        best = Some((key, i as usize, off));
                    }
                }
            }
        }
        let (_, i, off) = best?;
        let s = &self.channel.stations[i];
        let (left, right) = Self::limits(self.channel, i);
        let outside = if off < left {
            left - off
        } else if off > right {
            off - right
        } else {
            0.0
        };
        if outside > extra_m {
            return None;
        }
        Some(CutPlace {
            chainage_m: s.chainage_m,
            offset_m: off,
            left_m: left,
            right_m: right,
            outside_m: outside,
            daylight: s.daylight_left_m.is_some() && s.daylight_right_m.is_some(),
        })
    }
}

/// A building's relation to the cut, by the rule at the top of this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CutRelation {
    Taken,
    Near,
}

/// The building's relation to the cut, given its area (`NaN`: a point).
pub fn cut_relation(place: &CutPlace, area_m2: f64, near_m: f64) -> Option<CutRelation> {
    let radius = if area_m2.is_finite() && area_m2 > 0.0 {
        (area_m2 / std::f64::consts::PI).sqrt()
    } else {
        0.0
    };
    if place.outside_m <= radius {
        Some(CutRelation::Taken)
    } else if place.outside_m - radius <= near_m {
        Some(CutRelation::Near)
    } else {
        None
    }
}

/// The finest cell of the grid the severed-island test is worked out on, metres.
pub const ISLAND_CELL_M: f64 = 10.0;
/// The most cells one cut-through's grid may hold; a larger one takes coarser cells.
const ISLAND_MAX_CELLS: f64 = 4.0e6;

/// Which of `points` stand on land a cut-through severs from the rest (a meander island).
///
/// # The rule
///
/// A CUT-THROUGH is a run of the design line outside the river polygon. Around each, on a grid of
/// [`ISLAND_CELL_M`] cells over the run's box widened by four times its length (at least 1 km), a cell
/// is WATER when its centre is in the river polygon; with the cut, also when it is within
/// `half_m` of the design line (the channel's half-width at the water surface). LAND cells join
/// their four neighbours. Land that reaches the edge of the grid is the MAINLAND. A point is
/// SEVERED when its cell is mainland without the cut and not with it: the cut and the bypassed loop
/// of the river close round it. Land the loop encloses beyond the grid's edge reads as mainland,
/// so a very wide loop is missed, never invented.
pub fn severed(
    line: &[Pt],
    river: &[owd_core::geom::Polygon],
    half_m: f64,
    points: &[Pt],
) -> Vec<bool> {
    let mut out = vec![false; points.len()];
    if line.len() < 2 || river.is_empty() {
        return out;
    }
    let index = owd_core::geom::overlay::PolygonIndex::new(river);
    let in_river = |p: Pt| index.contains_xy(p);
    // The line sampled every cell, with each sample's place in or out of the river.
    let mut samples: Vec<(Pt, bool)> = Vec::new();
    for w in line.windows(2) {
        let d = libm::hypot(w[1][0] - w[0][0], w[1][1] - w[0][1]);
        let n = (d / ISLAND_CELL_M).ceil().max(1.0) as usize;
        for k in 0..n {
            let t = k as f64 / n as f64;
            let p = [
                w[0][0] + t * (w[1][0] - w[0][0]),
                w[0][1] + t * (w[1][1] - w[0][1]),
            ];
            samples.push((p, !in_river(p)));
        }
    }
    let mut i = 0;
    while i < samples.len() {
        if !samples[i].1 {
            i += 1;
            continue;
        }
        let j0 = i;
        while i < samples.len() && samples[i].1 {
            i += 1;
        }
        let run: Vec<Pt> = samples[j0..i].iter().map(|s| s.0).collect();
        if run.len() < 3 {
            continue;
        }
        island_run(&run, line, &index, half_m, points, &mut out);
    }
    out
}

fn island_run(
    run: &[Pt],
    line: &[Pt],
    river: &owd_core::geom::overlay::PolygonIndex,
    half_m: f64,
    points: &[Pt],
    out: &mut [bool],
) {
    let (mut x0, mut y0, mut x1, mut y1) = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for p in run {
        x0 = x0.min(p[0]);
        y0 = y0.min(p[1]);
        x1 = x1.max(p[0]);
        y1 = y1.max(p[1]);
    }
    let len = run.len() as f64 * ISLAND_CELL_M;
    let pad = (4.0 * len).max(1000.0);
    let (x0, y0, x1, y1) = (x0 - pad, y0 - pad, x1 + pad, y1 + pad);
    let mut cell = ISLAND_CELL_M;
    while (x1 - x0) * (y1 - y0) / (cell * cell) > ISLAND_MAX_CELLS {
        cell *= 2.0;
    }
    let w = ((x1 - x0) / cell).ceil() as usize;
    let h = ((y1 - y0) / cell).ceil() as usize;
    let centre = |c: usize, r: usize| [x0 + (c as f64 + 0.5) * cell, y0 + (r as f64 + 0.5) * cell];
    let mut water = vec![false; w * h];
    for r in 0..h {
        for c in 0..w {
            water[r * w + c] = river.contains_xy(centre(c, r));
        }
    }
    let mut cut = water.clone();
    // The channel: every cell within `half_m` of the line (the whole line inside the box).
    let reach = (half_m / cell).ceil() as i64 + 1;
    for seg in line.windows(2) {
        let (a, b) = (seg[0], seg[1]);
        let d = libm::hypot(b[0] - a[0], b[1] - a[1]);
        let n = (d / (cell / 2.0)).ceil().max(1.0) as usize;
        for k in 0..=n {
            let t = k as f64 / n as f64;
            let p = [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])];
            if p[0] < x0 - half_m || p[0] > x1 + half_m || p[1] < y0 - half_m || p[1] > y1 + half_m
            {
                continue;
            }
            let (pc, pr) = (((p[0] - x0) / cell) as i64, ((p[1] - y0) / cell) as i64);
            for dr in -reach..=reach {
                for dc in -reach..=reach {
                    let (cc, rr) = (pc + dc, pr + dr);
                    if cc < 0 || rr < 0 || cc >= w as i64 || rr >= h as i64 {
                        continue;
                    }
                    let q = centre(cc as usize, rr as usize);
                    if libm::hypot(q[0] - p[0], q[1] - p[1]) <= half_m {
                        cut[rr as usize * w + cc as usize] = true;
                    }
                }
            }
        }
    }
    let mainland = |water: &[bool]| -> Vec<bool> {
        let mut seen = vec![false; w * h];
        let mut stack: Vec<usize> = Vec::new();
        for r in 0..h {
            for c in 0..w {
                if (r == 0 || c == 0 || r == h - 1 || c == w - 1) && !water[r * w + c] {
                    seen[r * w + c] = true;
                    stack.push(r * w + c);
                }
            }
        }
        while let Some(i) = stack.pop() {
            let (r, c) = (i / w, i % w);
            let mut go = |j: usize| {
                if !water[j] && !seen[j] {
                    seen[j] = true;
                    stack.push(j);
                }
            };
            if r > 0 {
                go(i - w);
            }
            if r + 1 < h {
                go(i + w);
            }
            if c > 0 {
                go(i - 1);
            }
            if c + 1 < w {
                go(i + 1);
            }
        }
        seen
    };
    let before = mainland(&water);
    let after = mainland(&cut);
    for (k, p) in points.iter().enumerate() {
        if p[0] < x0 || p[0] >= x1 || p[1] < y0 || p[1] >= y1 {
            continue;
        }
        let i = ((p[1] - y0) / cell) as usize * w + ((p[0] - x0) / cell) as usize;
        if i < w * h && !cut[i] && before[i] && !after[i] {
            out[k] = true;
        }
    }
}

/// How many buildings each list of the cut report names; the counts are whole.
pub const CONFLICTS_LISTED: usize = 50;

impl super::session::Session {
    /// The buildings and the channel (BLD-1): those whose footprint the cut takes, those within
    /// the near distance of it, and those on land a cut-through severs, by the rules of this module,
    /// against `channel` (the volumes stage's own stations when fresh). A report, not a cost. `None`
    /// with no buildings attached.
    pub fn building_conflicts(&self, channel: &Channel) -> Option<serde_json::Value> {
        use super::features::FeatureInput;
        use serde_json::json;
        let set = self.features(FeatureInput::Buildings)?;
        let filters = self.building_filters();
        let index = CutIndex::new(channel);
        let line: Vec<Pt> = channel.stations.iter().map(|s| [s.x, s.y]).collect();
        let half = {
            let mut e: Vec<f64> = channel
                .stations
                .iter()
                .map(|s| s.water_edge_m(channel.wet_slope).max(s.half_width_m))
                .collect();
            e.sort_by(f64::total_cmp);
            e.get(e.len() / 2).copied().unwrap_or(0.0)
        };
        let pts: Vec<Pt> = set.features.iter().map(|f| f.at).collect();
        let cut_off = match self.river_polygons() {
            Some(river) => severed(&line, &river, half, &pts),
            None => vec![false; pts.len()],
        };
        let mut taken = Vec::new();
        let mut near = Vec::new();
        let mut island = Vec::new();
        let (mut n_taken, mut n_near, mut n_island) = ([0usize; 2], [0usize; 2], [0usize; 2]);
        // Every building in each class, by its number in the file's order (for the map's style).
        let (mut f_taken, mut f_near, mut f_island) = (Vec::new(), Vec::new(), Vec::new());
        for (i, f) in set.features.iter().enumerate() {
            let counted = usize::from(!filters.counts(f));
            let item = |place: Option<&CutPlace>| {
                json!({
                    "id": f.id,
                    "lon_lat": [(f.lon_lat[0] * 1e6).round() / 1e6, (f.lon_lat[1] * 1e6).round() / 1e6],
                    "chainage_m": place.map(|p| p.chainage_m.round()),
                    "outside_m": place.map(|p| (p.outside_m * 10.0).round() / 10.0),
                    "counted": counted == 0,
                })
            };
            match index.locate(f.at, filters.near_cut_m + 200.0) {
                Some(p) => match cut_relation(&p, f.area_m2, filters.near_cut_m) {
                    Some(CutRelation::Taken) => {
                        n_taken[counted] += 1;
                        f_taken.push(i);
                        if taken.len() < CONFLICTS_LISTED {
                            taken.push(item(Some(&p)));
                        }
                    }
                    Some(CutRelation::Near) => {
                        n_near[counted] += 1;
                        f_near.push(i);
                        if near.len() < CONFLICTS_LISTED {
                            near.push(item(Some(&p)));
                        }
                    }
                    None => {}
                },
                None => {}
            }
            if cut_off[i] {
                n_island[counted] += 1;
                f_island.push(i);
                if island.len() < CONFLICTS_LISTED {
                    island.push(item(None));
                }
            }
        }
        Some(json!({
            "taken": n_taken[0], "taken_apart": n_taken[1],
            "near": n_near[0], "near_apart": n_near[1],
            "severed": n_island[0], "severed_apart": n_island[1],
            "near_cut_m": filters.near_cut_m,
            "from_volumes": channel.from_volumes && channel.has_ground,
            "listed": CONFLICTS_LISTED,
            "taken_list": taken, "near_list": near, "severed_list": island,
            "fids": {"taken": f_taken, "near": f_near, "severed": f_island},
            "rule": "taken: the building, as a disc of its own area about its centroid, reaches inside the cut's daylight lines (the water's edge where a side has no cut); near: within near_cut_m of them; severed: on land a cut-through and the bypassed loop of the river close round. A report, not a cost.",
        }))
    }
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;

    #[test]
    fn the_nearest_point_of_a_line_gives_its_distance_and_chainage() {
        let l = [[0.0, 0.0], [100.0, 0.0], [100.0, 100.0]];
        assert_eq!(line_distance(&l, [50.0, 10.0]), (10.0, 50.0));
        assert_eq!(line_distance(&l, [110.0, 60.0]), (10.0, 160.0));
    }
}
