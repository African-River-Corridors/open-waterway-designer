//! Flooded land: the ground a pool drowns outside the river polygon, and an estimate of bank-full.
//!
//! The weir cascade prices cut and structures. A higher pool always cuts less, so with nothing
//! else in the cost it takes the largest lift offered at every weir. This module gives it the
//! other side of the trade: the land a pool at a level floods, and how far the pool stands above
//! the banks.
//!
//! # The rule (one rule, for the search, the report and the map)
//!
//! Ground is sampled on a square grid of `cell_m` cells ([`FloodField`]). A cell is LAND when its
//! centre is outside the river polygon (an island in a hole of the polygon is land). A land cell
//! is FLOODED by a pool at level `L` over the reach `[start_m, end_m)` when all of these hold:
//!
//! 1. its ground is below `L`;
//! 2. it is linked to the river through cells that are below `L` or in the river polygon, each
//!    sharing an edge with the next (four neighbours). A hollow with a rim at or above `L` all
//!    round it stays dry. The cells the line itself passes over link like the river, whatever
//!    their ground (the channel is cut there), as in the map's pool water;
//! 3. the nearest point of the line (the cascade's chainage axis) to its centre has a chainage in
//!    `[start_m, end_m)`;
//! 4. its centre is no more than `max_distance_m` from the line.
//!
//! Rules 1 and 2 are one number per cell, its FILL LEVEL: the lowest water level that reaches it
//! from the river (a priority flood from the river cells). A cell floods when `fill < L`.
//!
//! **Reach ends.** A cell belongs to the reach that holds its nearest chainage. The link of rule
//! 2 may run through ground of a neighbouring reach: no bund is assumed at the flanks of a weir.
//! So the flooded area of a reach is a sum over its cells, and the area of two reaches side by
//! side at one level is the area of the two as one. Ground beyond the two ends of the line (it
//! projects off the end) is in no reach and is never counted.
//!
//! The map's pool water (`owd-format`, `design/pool_water.rs`) holds the same three tests (in the
//! pool's reach by the nearest point of the line, below the level, joined to the line or the
//! river) on the DEM's own cells. It differs in one place: there the water of a pool does not
//! pass the line across a weir, so its link must lie inside the pool's own reach. Here the link
//! may leave the reach, which is what lets one table serve every (site, site, level) of the
//! search. The two agree except for low ground near a weir that is joined to the river only on
//! the far side of it; this rule counts that ground and the map's does not.
//!
//! **The grid edge.** The field ends at `max_distance_m` from the line, and where the DEM has no
//! value. Ground beyond it is not counted, so the area is a lower bound when the water reaches
//! that edge. [`PoolFlood::reaches_limit`] says when it does: a flooded cell touches a cell past
//! the distance limit or a cell with no DEM value.
//!
//! # Bank-full, an estimate
//!
//! [`BankProfile`] estimates bank-full along the line, in windows of [`BANK_WINDOW_M`] of
//! chainage. BANK CELLS are land cells that touch a river cell (eight neighbours): the ground just
//! outside the polygon, one cell wide. On each side of the line, the bank level of a window is the
//! [`BANK_PERCENTILE`]th percentile (linear between ranks) of its bank cells' ground, where the
//! side has at least [`BANK_MIN_CELLS`] of them. Bank-full of the window is the LOWER of the two
//! sides (water leaves the river over the lower bank first). A window with no bank cells takes
//! its level from its neighbours, linear in chainage. It is an estimate from the DEM, not a
//! surveyed bank line.
//!
//! The DEM RIVER LEVEL of a window is the median DEM value of its river cells (a DEM reads the
//! water surface over a river on the day it was flown, or a bed level if it was dry).

use crate::geom::{Polygon, Pt};
use crate::types::{Refusal, TerrainSampler};
use ovcd_core::pyfmt::general;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

/// The length of chainage of one bank-full window, in metres.
pub const BANK_WINDOW_M: f64 = 500.0;
/// The percentile of the bank cells' ground taken as the bank level of one side.
pub const BANK_PERCENTILE: f64 = 25.0;
/// A side needs at least this many bank cells in a window to give a level.
pub const BANK_MIN_CELLS: usize = 3;
/// The plain words for the estimate, for reports.
pub const BANK_FULL_ESTIMATE: &str = "an estimate from the DEM: in each 500 m of line, the 25th \
    percentile of the ground in the cells that touch the river polygon, on each side of the line, \
    and the lower of the two sides";

/// The most cells a field may hold. A field that would be larger is refused with the cell size
/// that fits.
pub const MAX_FIELD_CELLS: f64 = 2.0e7;

const M2_PER_HA: f64 = 10_000.0;

/// One cell of the field.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FloodCell {
    pub row: i32,
    pub col: i32,
    /// Chainage of the nearest point of the line.
    pub chainage_m: f64,
    /// Distance from the cell centre to the line.
    pub distance_m: f64,
    /// DEM value at the cell centre; NaN where the DEM has none.
    pub ground_m: f64,
    /// The centre is inside the river polygon.
    pub wet: bool,
    /// The lowest water level that reaches the cell from the river; -inf for a river cell, +inf
    /// for a cell no water can reach inside the field.
    pub fill_m: f64,
    /// The cell touches the edge of the field: a neighbour is past the distance limit or has no
    /// DEM value.
    pub at_limit: bool,
    /// +1 left of the line (looking up-chainage), -1 right, 0 on it.
    pub side: i8,
    /// The cell projects beyond an end of the line: in no reach, never counted.
    pub beyond_end: bool,
}

/// What a pool floods.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PoolFlood {
    /// Flooded land outside the river polygon, m2.
    pub area_m2: f64,
    pub cells: usize,
    /// The water reaches the edge of the field, so `area_m2` is a lower bound.
    pub reaches_limit: bool,
}

impl PoolFlood {
    pub fn area_ha(&self) -> f64 {
        self.area_m2 / M2_PER_HA
    }
}

/// The ground beside a line, with each cell's fill level. Build it once with
/// [`FloodField::build`]; every pool and level is then a count.
#[derive(Debug, Clone, PartialEq)]
pub struct FloodField {
    pub cell_m: f64,
    pub max_distance_m: f64,
    /// Centre of the cell at row 0, column 0. Rows grow southward.
    pub x0: f64,
    pub y0: f64,
    /// Length of the line, m.
    pub length_m: f64,
    /// Every cell, sorted by chainage.
    pub cells: Vec<FloodCell>,
}

#[derive(Clone, Copy, PartialEq)]
struct HeapItem {
    key: f64,
    idx: u32,
    seg: u32,
}

impl Eq for HeapItem {}

impl Ord for HeapItem {
    fn cmp(&self, other: &Self) -> Ordering {
        // A min-heap on `key`; ties by index so the order is fixed.
        other
            .key
            .partial_cmp(&self.key)
            .unwrap_or(Ordering::Equal)
            .then_with(|| other.idx.cmp(&self.idx))
    }
}

impl PartialOrd for HeapItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn cell_key(row: i32, col: i32) -> u64 {
    ((row as u32 as u64) << 32) | (col as u32 as u64)
}

/// `(distance, t along the segment clamped to 0..1, raw t, side)` of `p` against `a -> b`.
fn seg_dist(p: Pt, a: Pt, b: Pt) -> (f64, f64, f64, i8) {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    let raw = if l2 > 0.0 {
        ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2
    } else {
        0.0
    };
    let t = raw.clamp(0.0, 1.0);
    let (qx, qy) = (a[0] + t * dx, a[1] + t * dy);
    let d = libm::hypot(p[0] - qx, p[1] - qy);
    let cross = dx * (p[1] - a[1]) - dy * (p[0] - a[0]);
    let side = if cross > 0.0 {
        1
    } else if cross < 0.0 {
        -1
    } else {
        0
    };
    (d, t, raw, side)
}

const N4: [(i32, i32); 4] = [(-1, 0), (1, 0), (0, -1), (0, 1)];
const N8: [(i32, i32); 8] = [
    (-1, 0),
    (1, 0),
    (0, -1),
    (0, 1),
    (-1, -1),
    (-1, 1),
    (1, -1),
    (1, 1),
];

impl FloodField {
    /// Sample the ground within `max_distance_m` of `line` on `cell_m` cells and work out each
    /// cell's fill level. `line` is the cascade's chainage axis; `river_polygon` marks the river.
    pub fn build(
        line: &[Pt],
        terrain: TerrainSampler,
        river_polygon: &[Polygon],
        max_distance_m: f64,
        cell_m: f64,
    ) -> Result<FloodField, Refusal> {
        if !(cell_m.is_finite() && cell_m > 0.0) {
            return Err("the flood cell size must be a positive number of metres".into());
        }
        if !(max_distance_m.is_finite() && max_distance_m > 0.0) {
            return Err("the flood distance must be a positive number of metres".into());
        }
        // The line with its chainage, without points closer than half a cell to the one before
        // (they add work and no detail the grid can hold). Chainage stays that of the full line.
        let mut pts: Vec<Pt> = Vec::new();
        let mut cum: Vec<f64> = Vec::new();
        let mut run = 0.0;
        for (i, p) in line.iter().enumerate() {
            if i > 0 {
                run += libm::hypot(p[0] - line[i - 1][0], p[1] - line[i - 1][1]);
            }
            let keep = match pts.last() {
                None => true,
                Some(q) => {
                    libm::hypot(p[0] - q[0], p[1] - q[1]) >= 0.5 * cell_m || i == line.len() - 1
                }
            };
            if keep {
                if i == line.len() - 1
                    && let Some(q) = pts.last()
                    && pts.len() > 1
                    && libm::hypot(p[0] - q[0], p[1] - q[1]) < 1e-9
                {
                    // The last point sits on the one kept before it: move that one's chainage.
                    *cum.last_mut().unwrap() = run;
                    continue;
                }
                pts.push(*p);
                cum.push(run);
            }
        }
        if pts.len() < 2 || run <= 0.0 {
            return Err("the flood field needs a line with a length".into());
        }
        let length_m = run;
        let est = length_m * 2.0 * max_distance_m / (cell_m * cell_m);
        if est > MAX_FIELD_CELLS {
            let fit = libm::sqrt(length_m * 2.0 * max_distance_m / MAX_FIELD_CELLS);
            return Err(format!(
                "the flood field would hold about {:.0} million cells ({:.0} km of line, {} m each \
                 side, {} m cells). Use flood cells of at least {:.0} m (`flood_cell_m`), or a \
                 smaller flood distance (`flood_distance_m`).",
                est / 1.0e6,
                length_m / 1000.0,
                general(max_distance_m),
                general(cell_m),
                fit.ceil()
            ));
        }
        let (mut xmin, mut ymax) = (f64::INFINITY, f64::NEG_INFINITY);
        for p in &pts {
            xmin = xmin.min(p[0]);
            ymax = ymax.max(p[1]);
        }
        let x0 = xmin - max_distance_m;
        let y0 = ymax + max_distance_m;
        let centre = |row: i32, col: i32| -> Pt {
            [x0 + f64::from(col) * cell_m, y0 - f64::from(row) * cell_m]
        };
        let n_seg = pts.len() - 1;

        // 1. Nearest point of the line for every cell within the distance: a wave from the cells
        //    the line passes over, each cell measured exactly against the segment its neighbour
        //    was nearest to and the two segments beside it.
        struct Near {
            row: i32,
            col: i32,
            d: f64,
            seg: u32,
            on_line: bool,
        }
        let mut index: HashMap<u64, u32> = HashMap::new();
        let mut near: Vec<Near> = Vec::new();
        let mut heap: BinaryHeap<HeapItem> = BinaryHeap::new();
        let offer = |row: i32,
                     col: i32,
                     seg: usize,
                     on_line: bool,
                     index: &mut HashMap<u64, u32>,
                     near: &mut Vec<Near>,
                     heap: &mut BinaryHeap<HeapItem>| {
            let p = centre(row, col);
            let lo = seg.saturating_sub(1);
            let hi = (seg + 1).min(n_seg - 1);
            let mut best = (f64::INFINITY, seg);
            for s in lo..=hi {
                let d = seg_dist(p, pts[s], pts[s + 1]).0;
                if d < best.0 {
                    best = (d, s);
                }
            }
            if best.0 > max_distance_m {
                return;
            }
            let k = cell_key(row, col);
            match index.get(&k) {
                Some(&i) => {
                    let n = &mut near[i as usize];
                    n.on_line |= on_line;
                    if best.0 < n.d {
                        n.d = best.0;
                        n.seg = best.1 as u32;
                        heap.push(HeapItem {
                            key: best.0,
                            idx: i,
                            seg: best.1 as u32,
                        });
                    }
                }
                None => {
                    let i = near.len() as u32;
                    index.insert(k, i);
                    near.push(Near {
                        row,
                        col,
                        d: best.0,
                        seg: best.1 as u32,
                        on_line,
                    });
                    heap.push(HeapItem {
                        key: best.0,
                        idx: i,
                        seg: best.1 as u32,
                    });
                }
            }
        };
        for s in 0..n_seg {
            let (a, b) = (pts[s], pts[s + 1]);
            let len = libm::hypot(b[0] - a[0], b[1] - a[1]);
            let steps = (len / (0.5 * cell_m)).ceil().max(1.0) as usize;
            for k in 0..=steps {
                let t = k as f64 / steps as f64;
                let (x, y) = (a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1]));
                let col = libm::rint((x - x0) / cell_m) as i32;
                let row = libm::rint((y0 - y) / cell_m) as i32;
                offer(row, col, s, true, &mut index, &mut near, &mut heap);
            }
        }
        while let Some(item) = heap.pop() {
            let (row, col) = {
                let n = &near[item.idx as usize];
                if item.key > n.d || item.seg != n.seg {
                    continue;
                }
                (n.row, n.col)
            };
            for (dr, dc) in N8 {
                offer(
                    row + dr,
                    col + dc,
                    item.seg as usize,
                    false,
                    &mut index,
                    &mut near,
                    &mut heap,
                );
            }
        }

        // 2. The cells, with ground from the DEM.
        let n = near.len();
        let mut cells: Vec<FloodCell> = Vec::with_capacity(n);
        let mut on_line: Vec<bool> = Vec::with_capacity(n);
        for q in &near {
            let s = q.seg as usize;
            let p = centre(q.row, q.col);
            let (d, t, raw, side) = seg_dist(p, pts[s], pts[s + 1]);
            let beyond = (s == 0 && raw < 0.0) || (s == n_seg - 1 && raw > 1.0);
            cells.push(FloodCell {
                row: q.row,
                col: q.col,
                chainage_m: cum[s] + t * (cum[s + 1] - cum[s]),
                distance_m: d,
                ground_m: f64::NAN,
                wet: false,
                fill_m: f64::INFINITY,
                at_limit: false,
                side,
                beyond_end: beyond,
            });
            on_line.push(q.on_line);
        }
        drop(near);
        const CHUNK: usize = 1 << 16;
        let mut xs = Vec::with_capacity(CHUNK);
        let mut ys = Vec::with_capacity(CHUNK);
        let mut at = 0;
        while at < n {
            let end = (at + CHUNK).min(n);
            xs.clear();
            ys.clear();
            for c in &cells[at..end] {
                let p = centre(c.row, c.col);
                xs.push(p[0]);
                ys.push(p[1]);
            }
            let z = terrain(&xs, &ys);
            if z.len() != end - at {
                return Err(format!(
                    "the terrain gave {} levels for {} points",
                    z.len(),
                    end - at
                ));
            }
            for (c, zc) in cells[at..end].iter_mut().zip(z) {
                c.ground_m = if zc.is_finite() { zc } else { f64::NAN };
            }
            at = end;
        }

        // 3. The river cells: every ring of the polygon crossed with each row of cell centres
        //    (even-odd, so a hole is land).
        let mut crossings: HashMap<i32, Vec<f64>> = HashMap::new();
        let mut ring = |r: &[Pt]| {
            let m = r.len();
            if m < 3 {
                return;
            }
            for i in 0..m {
                let (a, b) = (r[i], r[(i + 1) % m]);
                if a[1] == b[1] {
                    continue;
                }
                let (lo, hi) = if a[1] < b[1] { (a, b) } else { (b, a) };
                // Rows whose centre y is in [lo.y, hi.y).
                let r_first = ((y0 - hi[1]) / cell_m).floor() as i64 + 1;
                let r_last = ((y0 - lo[1]) / cell_m).floor() as i64;
                for row in r_first..=r_last {
                    let y = y0 - row as f64 * cell_m;
                    if !(y >= lo[1] && y < hi[1]) {
                        continue;
                    }
                    let x = lo[0] + (y - lo[1]) / (hi[1] - lo[1]) * (hi[0] - lo[0]);
                    crossings.entry(row as i32).or_default().push(x);
                }
            }
        };
        for poly in river_polygon {
            ring(&poly.exterior);
            for h in &poly.holes {
                ring(h);
            }
        }
        for (row, xs) in &mut crossings {
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
            for pair in xs.as_chunks::<2>().0 {
                let c_first = ((pair[0] - x0) / cell_m).ceil() as i64;
                let c_last = ((pair[1] - x0) / cell_m).ceil() as i64 - 1;
                for col in c_first..=c_last {
                    if let Some(&i) = index.get(&cell_key(*row, col as i32)) {
                        cells[i as usize].wet = true;
                    }
                }
            }
        }

        // 4. The edge of the field, and the fill level of every cell (a priority flood from the
        //    river cells; the cells under the line start at their own ground).
        let open = |c: &FloodCell| !c.beyond_end && (c.wet || c.ground_m.is_finite());
        for i in 0..n {
            let (row, col) = (cells[i].row, cells[i].col);
            let mut edge = false;
            for (dr, dc) in N4 {
                match index.get(&cell_key(row + dr, col + dc)) {
                    None => edge = true,
                    Some(&j) => {
                        let o = &cells[j as usize];
                        if !o.beyond_end && !o.wet && !o.ground_m.is_finite() {
                            edge = true;
                        }
                    }
                }
            }
            cells[i].at_limit = edge;
        }
        let mut heap: BinaryHeap<HeapItem> = BinaryHeap::new();
        for i in 0..n {
            if !open(&cells[i]) {
                continue;
            }
            // A cell under the line joins the pool whatever its ground (the channel is cut
            // there), but it is flooded land only when its own ground is below the level.
            cells[i].fill_m = if cells[i].wet {
                f64::NEG_INFINITY
            } else if on_line[i] {
                cells[i].ground_m
            } else {
                continue;
            };
            heap.push(HeapItem {
                key: f64::NEG_INFINITY,
                idx: i as u32,
                seg: 0,
            });
        }
        while let Some(item) = heap.pop() {
            let i = item.idx as usize;
            if item.key > cells[i].fill_m {
                continue;
            }
            let (row, col) = (cells[i].row, cells[i].col);
            for (dr, dc) in N4 {
                let Some(&j) = index.get(&cell_key(row + dr, col + dc)) else {
                    continue;
                };
                let j = j as usize;
                if !open(&cells[j]) || cells[j].wet {
                    continue;
                }
                let cand = item.key.max(cells[j].ground_m);
                if cand < cells[j].fill_m {
                    cells[j].fill_m = cand;
                    heap.push(HeapItem {
                        key: cand,
                        idx: j as u32,
                        seg: 0,
                    });
                }
            }
        }
        cells.sort_by(|a, b| {
            a.chainage_m
                .partial_cmp(&b.chainage_m)
                .unwrap_or(Ordering::Equal)
                .then(a.row.cmp(&b.row))
                .then(a.col.cmp(&b.col))
        });
        Ok(FloodField {
            cell_m,
            max_distance_m,
            x0,
            y0,
            length_m,
            cells,
        })
    }

    /// The centre of a cell.
    pub fn centre(&self, c: &FloodCell) -> Pt {
        [
            self.x0 + f64::from(c.col) * self.cell_m,
            self.y0 - f64::from(c.row) * self.cell_m,
        ]
    }

    /// The cells with `start_m <= chainage < end_m`.
    fn range(&self, start_m: f64, end_m: f64) -> &[FloodCell] {
        let i0 = self.cells.partition_point(|c| c.chainage_m < start_m);
        let i1 = self.cells.partition_point(|c| c.chainage_m < end_m);
        &self.cells[i0..i1.max(i0)]
    }

    /// A cell is land a pool can flood: outside the polygon, with a DEM value, inside the line's
    /// ends.
    fn is_land(c: &FloodCell) -> bool {
        !c.wet && !c.beyond_end && c.ground_m.is_finite()
    }

    /// The land a pool at `level_m` floods over `[start_m, end_m)`: the rule at the top of this
    /// module.
    pub fn flooded(&self, start_m: f64, end_m: f64, level_m: f64) -> PoolFlood {
        let mut cells = 0usize;
        let mut reaches_limit = false;
        for c in self.range(start_m, end_m) {
            if Self::is_land(c) && c.fill_m < level_m {
                cells += 1;
                reaches_limit |= c.at_limit;
            }
        }
        PoolFlood {
            area_m2: cells as f64 * self.cell_m * self.cell_m,
            cells,
            reaches_limit,
        }
    }

    /// The centres of the flooded land cells of a pool, for a map layer. Each is a square of
    /// `cell_m`. Pass `with_river = true` to add the river cells of the reach (the whole of the
    /// pool's water).
    pub fn flooded_cells(
        &self,
        start_m: f64,
        end_m: f64,
        level_m: f64,
        with_river: bool,
    ) -> Vec<Pt> {
        self.range(start_m, end_m)
            .iter()
            .filter(|c| {
                (Self::is_land(c) && c.fill_m < level_m) || (with_river && c.wet && !c.beyond_end)
            })
            .map(|c| self.centre(c))
            .collect()
    }
}

/// Flooded area for every (break, level): a prefix sum, so a reach between two breaks at a table
/// level is two reads. Build it on the cascade's start, candidate sites and terminal, and on
/// `cascade_levels`. It gives exactly [`FloodField::flooded`]'s area.
#[derive(Debug, Clone, PartialEq)]
pub struct FloodTable {
    breaks_m: Vec<f64>,
    levels_m: Vec<f64>,
    /// `cum[b][l]`: flooded cells with chainage below `breaks_m[b]` at `levels_m[l]`.
    cum: Vec<Vec<u32>>,
    cell_area_m2: f64,
}

impl FloodTable {
    pub fn new(field: &FloodField, breaks_m: &[f64], levels_m: &[f64]) -> FloodTable {
        let sorted = |v: &[f64]| {
            let mut v: Vec<f64> = v.iter().copied().filter(|x| x.is_finite()).collect();
            v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
            v.dedup();
            v
        };
        let breaks = sorted(breaks_m);
        let levels = sorted(levels_m);
        let n_l = levels.len();
        // hist[s][l]: cells of slice s (chainage in [breaks[s-1], breaks[s])) that first flood at
        // level index l. Slice 0 is below the first break.
        let mut cum = vec![vec![0u32; n_l]; breaks.len() + 1];
        for c in &field.cells {
            if !FloodField::is_land(c) {
                continue;
            }
            // The first level strictly above the fill level.
            let l = levels.partition_point(|&v| v <= c.fill_m);
            if l >= n_l {
                continue;
            }
            // The slice: the number of breaks at or below the chainage.
            let s = breaks.partition_point(|&b| b <= c.chainage_m);
            cum[s][l] += 1;
        }
        for row in &mut cum {
            for l in 1..n_l {
                row[l] += row[l - 1];
            }
        }
        // cum[b] = cells below breaks[b] = slices 0..=b.
        for s in 1..cum.len() {
            let (head, tail) = cum.split_at_mut(s);
            for l in 0..n_l {
                tail[0][l] += head[s - 1][l];
            }
        }
        FloodTable {
            breaks_m: breaks,
            levels_m: levels,
            cum,
            cell_area_m2: field.cell_m * field.cell_m,
        }
    }

    fn find(v: &[f64], x: f64) -> Option<usize> {
        let i = v.partition_point(|&b| b < x - 1e-6);
        (i < v.len() && (v[i] - x).abs() <= 1e-6).then_some(i)
    }

    /// Flooded land in m2 over `[start_m, end_m)` at `level_m`; `None` when an end or the level is
    /// not in the table (read the field then: a value is never taken from a neighbour).
    pub fn area_m2(&self, start_m: f64, end_m: f64, level_m: f64) -> Option<f64> {
        let a = Self::find(&self.breaks_m, start_m)?;
        let b = Self::find(&self.breaks_m, end_m)?;
        let l = Self::find(&self.levels_m, level_m)?;
        if b <= a {
            return Some(0.0);
        }
        // cum[k] holds slices 0..=k, that is cells below breaks[k]... slice k is
        // [breaks[k-1], breaks[k]), so cells below breaks[k] are cum[k].
        Some(f64::from(self.cum[b][l] - self.cum[a][l]) * self.cell_area_m2)
    }
}

/// Bank-full and the DEM river level along the line, in windows of [`BANK_WINDOW_M`]. See the top
/// of this module for the definition. An estimate.
#[derive(Debug, Clone, PartialEq)]
pub struct BankProfile {
    pub window_m: f64,
    /// Bank-full per window; NaN in every window when the field has no bank cells at all.
    pub bank_m: Vec<f64>,
    /// The windows whose bank-full was measured (the others are filled from their neighbours).
    pub measured: Vec<bool>,
    /// Median DEM level of the river cells per window; filled like `bank_m`.
    pub river_m: Vec<f64>,
    /// `mins[k][i]`: the lowest `bank_m` over `2^k` windows from `i`.
    mins: Vec<Vec<f64>>,
}

/// The `p`th percentile of `v`, linear between ranks (numpy's default). `v` is sorted here.
pub fn percentile(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let pos = (p / 100.0).clamp(0.0, 1.0) * (v.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = (lo + 1).min(v.len() - 1);
    v[lo] + (pos - lo as f64) * (v[hi] - v[lo])
}

/// Fill the NaN entries from their known neighbours, linear in index; the ends hold the nearest
/// known value. All NaN stays all NaN.
fn fill_gaps(v: &mut [f64]) {
    let known: Vec<usize> = (0..v.len()).filter(|&i| v[i].is_finite()).collect();
    if known.is_empty() {
        return;
    }
    for i in 0..v.len() {
        if v[i].is_finite() {
            continue;
        }
        let after = known.partition_point(|&k| k < i);
        v[i] = if after == 0 {
            v[known[0]]
        } else if after == known.len() {
            v[known[after - 1]]
        } else {
            let (a, b) = (known[after - 1], known[after]);
            v[a] + (v[b] - v[a]) * (i - a) as f64 / (b - a) as f64
        };
    }
}

impl BankProfile {
    pub fn new(field: &FloodField) -> BankProfile {
        let w = BANK_WINDOW_M;
        let n_w = ((field.length_m / w).ceil() as usize).max(1);
        let index: HashMap<u64, u32> = field
            .cells
            .iter()
            .enumerate()
            .map(|(i, c)| (cell_key(c.row, c.col), i as u32))
            .collect();
        let mut left: Vec<Vec<f64>> = vec![Vec::new(); n_w];
        let mut right: Vec<Vec<f64>> = vec![Vec::new(); n_w];
        let mut river: Vec<Vec<f64>> = vec![Vec::new(); n_w];
        for c in &field.cells {
            if c.beyond_end || !c.ground_m.is_finite() {
                continue;
            }
            let k = ((c.chainage_m / w).floor() as usize).min(n_w - 1);
            if c.wet {
                river[k].push(c.ground_m);
                continue;
            }
            let touches = N8.iter().any(|(dr, dc)| {
                index
                    .get(&cell_key(c.row + dr, c.col + dc))
                    .is_some_and(|&j| field.cells[j as usize].wet)
            });
            if !touches {
                continue;
            }
            if c.side >= 0 {
                left[k].push(c.ground_m);
            }
            if c.side <= 0 {
                right[k].push(c.ground_m);
            }
        }
        let mut bank = vec![f64::NAN; n_w];
        let mut river_m = vec![f64::NAN; n_w];
        for k in 0..n_w {
            let side = |v: &mut Vec<f64>| {
                if v.len() >= BANK_MIN_CELLS {
                    percentile(v, BANK_PERCENTILE)
                } else {
                    f64::NAN
                }
            };
            let (l, r) = (side(&mut left[k]), side(&mut right[k]));
            bank[k] = match (l.is_finite(), r.is_finite()) {
                (true, true) => l.min(r),
                (true, false) => l,
                (false, true) => r,
                (false, false) => f64::NAN,
            };
            river_m[k] = percentile(&mut river[k], 50.0);
        }
        let measured: Vec<bool> = bank.iter().map(|b| b.is_finite()).collect();
        fill_gaps(&mut bank);
        fill_gaps(&mut river_m);
        let mut mins = vec![bank.clone()];
        let mut span = 1;
        while 2 * span <= n_w {
            let prev = mins.last().unwrap();
            let next: Vec<f64> = (0..=n_w - 2 * span)
                .map(|i| prev[i].min(prev[i + span]))
                .collect();
            mins.push(next);
            span *= 2;
        }
        BankProfile {
            window_m: w,
            bank_m: bank,
            measured,
            river_m,
            mins,
        }
    }

    /// True when the field gave a bank-full level (it had cells that touch the river polygon).
    pub fn known(&self) -> bool {
        self.bank_m.iter().any(|b| b.is_finite())
    }

    fn window(&self, chainage_m: f64) -> usize {
        let k = (chainage_m / self.window_m).floor();
        if k <= 0.0 {
            0
        } else {
            (k as usize).min(self.bank_m.len() - 1)
        }
    }

    /// The windows a reach `[start_m, end_m)` touches, first and last. A reach that ends on a
    /// window's edge does not touch the next window.
    fn windows(&self, start_m: f64, end_m: f64) -> (usize, usize) {
        let a = self.window(start_m);
        let b = self.window((end_m - 1e-6).max(start_m));
        (a, b.max(a))
    }

    /// Bank-full at a chainage (its window's).
    pub fn bank_at(&self, chainage_m: f64) -> f64 {
        self.bank_m[self.window(chainage_m)]
    }

    /// The DEM river level at a chainage (its window's).
    pub fn river_at(&self, chainage_m: f64) -> f64 {
        self.river_m[self.window(chainage_m)]
    }

    /// The lowest bank-full in the reach `[start_m, end_m)`.
    pub fn min_bank(&self, start_m: f64, end_m: f64) -> f64 {
        let (a, b) = self.windows(start_m, end_m);
        let n = b - a + 1;
        let k = (usize::BITS - 1 - n.leading_zeros()) as usize;
        self.mins[k][a].min(self.mins[k][b + 1 - (1 << k)])
    }

    /// The chainage at the middle of the window with the lowest bank-full in the reach.
    pub fn lowest_bank_at(&self, start_m: f64, end_m: f64) -> f64 {
        let (a, b) = self.windows(start_m, end_m);
        let mut best = a;
        for k in a..=b {
            if self.bank_m[k] < self.bank_m[best] {
                best = k;
            }
        }
        (best as f64 + 0.5) * self.window_m
    }

    /// The most a pool at `level_m` stands above bank-full in the reach.
    pub fn max_above_bank(&self, start_m: f64, end_m: f64, level_m: f64) -> f64 {
        level_m - self.min_bank(start_m, end_m)
    }

    /// The pool's heights at its two ends: above bank-full and over the DEM river level. The upper
    /// end is read just inside the reach.
    pub fn at_ends(&self, start_m: f64, end_m: f64, level_m: f64) -> PoolEnds {
        let (a, b) = self.windows(start_m, end_m);
        PoolEnds {
            above_bank_lower_m: level_m - self.bank_m[a],
            above_bank_upper_m: level_m - self.bank_m[b],
            max_above_bank_m: self.max_above_bank(start_m, end_m, level_m),
            depth_over_river_lower_m: level_m - self.river_m[a],
            depth_over_river_upper_m: level_m - self.river_m[b],
        }
    }
}

/// A pool against the banks and the DEM river level. `lower` is the downstream (low chainage) end,
/// just above the weir that holds the pool; `upper` is the upstream end.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PoolEnds {
    pub above_bank_lower_m: f64,
    pub above_bank_upper_m: f64,
    /// The most the pool stands above bank-full anywhere in its reach.
    pub max_above_bank_m: f64,
    pub depth_over_river_lower_m: f64,
    pub depth_over_river_upper_m: f64,
}

/// The flood terms of the cascade's reach cost.
#[derive(Debug, Clone, Copy)]
pub struct FloodTerms<'a> {
    pub field: &'a FloodField,
    pub table: &'a FloodTable,
    pub banks: &'a BankProfile,
    /// Money per hectare of flooded land outside the river polygon.
    pub cost_per_ha: f64,
    /// No pool may stand more than this above bank-full anywhere in its reach.
    pub max_above_bank_m: Option<f64>,
}

impl FloodTerms<'_> {
    /// True when the pool breaks the limit.
    pub fn breaks_limit(&self, start_m: f64, end_m: f64, level_m: f64) -> bool {
        match self.max_above_bank_m {
            // 1e-9: a pool exactly at the limit is allowed.
            Some(limit) => self.banks.max_above_bank(start_m, end_m, level_m) > limit + 1e-9,
            None => false,
        }
    }

    /// Flooded land of the pool, m2.
    pub fn area_m2(&self, start_m: f64, end_m: f64, level_m: f64) -> f64 {
        self.table
            .area_m2(start_m, end_m, level_m)
            .unwrap_or_else(|| self.field.flooded(start_m, end_m, level_m).area_m2)
    }

    /// `cost_per_ha x flooded hectares`; infinite for a pool that breaks the limit (the search
    /// never takes an infinite reach).
    pub fn cost(&self, start_m: f64, end_m: f64, level_m: f64) -> f64 {
        if self.breaks_limit(start_m, end_m, level_m) {
            return f64::INFINITY;
        }
        if self.cost_per_ha == 0.0 {
            return 0.0;
        }
        self.cost_per_ha * self.area_m2(start_m, end_m, level_m) / M2_PER_HA
    }
}

/// `reach_cost = cut(start, end, level) + flood cost`, infinite where the limit is broken.
pub fn with_flood_cost<'a>(
    cut: &'a dyn Fn(f64, f64, f64) -> Result<f64, Refusal>,
    flood: FloodTerms<'a>,
) -> impl Fn(f64, f64, f64) -> Result<f64, Refusal> + 'a {
    move |a, b, level| {
        let f = flood.cost(a, b, level);
        if !f.is_finite() {
            return Ok(f64::INFINITY);
        }
        Ok(cut(a, b, level)? + f)
    }
}

/// Why no cascade reaches the top level under the bank limit: which reach blocks it, and what
/// would unblock it. `sites_m` are the candidate sites inside `(start_m, terminal_m]`, sorted;
/// `levels_m` are `cascade_levels`.
#[allow(clippy::too_many_arguments)]
pub fn explain_blocked(
    banks: &BankProfile,
    limit_m: f64,
    start_m: f64,
    base_level_m: f64,
    sites_m: &[f64],
    lifts_m: &[f64],
    levels_m: &[f64],
    terminal_m: f64,
    terminal_min_level_m: f64,
    min_spacing_m: f64,
) -> String {
    let ok = |a: f64, b: f64, l: f64| banks.max_above_bank(a, b, l) <= limit_m + 1e-9;
    let mut s: Vec<f64> = vec![start_m];
    s.extend(sites_m.iter().copied());
    let n_l = levels_m.len();
    let level_of = |x: f64| levels_m.iter().position(|&v| (v - x).abs() <= 1e-6);
    // reach[j][k]: a weir at site j can hold a pool at level k with every pool below it inside
    // the limit.
    let mut reach = vec![vec![false; n_l]; s.len()];
    if let Some(k) = level_of(base_level_m) {
        reach[0][k] = true;
    }
    // up[k]: the levels one lift above level k.
    let up: Vec<Vec<usize>> = levels_m
        .iter()
        .map(|&v| lifts_m.iter().filter_map(|&l| level_of(v + l)).collect())
        .collect();
    for j in 1..s.len() {
        for i in 0..j {
            if s[j] - s[i] < min_spacing_m {
                continue;
            }
            for k in 0..n_l {
                if !reach[i][k] || !ok(s[i], s[j], levels_m[k]) {
                    continue;
                }
                for &k2 in &up[k] {
                    reach[j][k2] = true;
                }
            }
        }
    }
    let m = |x: f64| format!("{x:.2}");
    let ch = |x: f64| format!("{x:.0}");
    let head = format!(
        "No cascade reaches the top level of {} m with every pool at most {} m above bank-full \
         (`max_above_bank_m`).",
        m(terminal_min_level_m),
        m(limit_m)
    );
    let unblock = |need: f64| {
        format!(
            "To unblock it: give more candidate sites (a smaller `site_step_m`, or more `--sites`) \
             so a weir can stand where the banks are higher; allow smaller lifts (`--lifts`); or \
             raise `max_above_bank_m` to at least {} m.",
            m(need)
        )
    };
    // The next site above site j, or the terminal.
    let next = |j: usize| {
        if j + 1 < s.len() {
            s[j + 1]
        } else {
            terminal_m
        }
    };

    // A. Some weir can hold a pool at the top level, but the pool from it to the terminal breaks
    //    the limit.
    let mut top: Option<(usize, usize, f64)> = None;
    for j in 1..s.len() {
        for k in 0..n_l {
            if reach[j][k] && levels_m[k] >= terminal_min_level_m - 1e-9 {
                let above = banks.max_above_bank(s[j], terminal_m, levels_m[k]);
                if top.is_none_or(|t| above < t.2) {
                    top = Some((j, k, above));
                }
            }
        }
    }
    if let Some((j, k, above)) = top {
        let low = banks.lowest_bank_at(s[j], terminal_m);
        return format!(
            "{head} The reach from {} m to {} m blocks it: the top pool, at {} m, would stand {} m \
             above bank-full there (bank-full is about {} m near {} m). {}",
            ch(s[j]),
            ch(terminal_m),
            m(levels_m[k]),
            m(above),
            m(banks.min_bank(s[j], terminal_m)),
            ch(low),
            unblock(above)
        );
    }
    // B. No weir can hold a pool that high. The highest pool that stays inside the limit over the
    //    shortest reach it could have, and where the top pool would stand least above the banks.
    let mut high: Option<(usize, usize)> = None;
    for (j, row) in reach.iter().enumerate() {
        for (k, &r) in row.iter().enumerate() {
            if r && ok(s[j], next(j), levels_m[k])
                && high.is_none_or(|h| k > h.1 || (k == h.1 && j > h.0))
            {
                high = Some((j, k));
            }
        }
    }
    let min_lift = lifts_m.iter().copied().fold(f64::INFINITY, f64::min);
    if sites_m.is_empty() {
        return format!(
            "{head} There is no candidate site between {} m and {} m, so no weir can lift the \
             water. Give candidate sites (`--sites`, or a smaller `site_step_m`).",
            ch(start_m),
            ch(terminal_m)
        );
    }
    let Some((hj, hk)) = high else {
        // Not even the water below the first weir is inside the limit.
        let above = banks.max_above_bank(start_m, s[1], base_level_m);
        return format!(
            "{head} The reach from {} m to {} m blocks it: the water below the first weir, at the \
             base level {} m, stands {} m above bank-full there (about {} m). A lower base level, \
             or a limit of at least {} m, would unblock it.",
            ch(start_m),
            ch(s[1]),
            m(base_level_m),
            m(above),
            m(banks.min_bank(start_m, s[1])),
            m(above)
        );
    };
    // Where a pool at the top level would stand least above the banks, over the shortest reach it
    // could have.
    let mut best: Option<(usize, f64)> = None;
    for (j, &site) in s.iter().enumerate().skip(1) {
        let above = banks.max_above_bank(site, next(j), terminal_min_level_m);
        if best.is_none_or(|b| above < b.1) {
            best = Some((j, above));
        }
    }
    let (bj, above) = best.unwrap_or((hj, f64::NAN));
    let held = format!(
        "{head} The highest pool the weirs can hold inside the limit is {} m, above the site at {} \
         m.",
        m(levels_m[hk]),
        ch(s[hj])
    );
    if above > limit_m + 1e-9 {
        return format!(
            "{held} The reach from {} m to {} m blocks it: a pool at the top level would stand {} \
             m above bank-full there (about {} m), and at every other site it would stand higher. \
             {}",
            ch(s[bj]),
            ch(next(bj)),
            m(above),
            m(banks.min_bank(s[bj], next(bj))),
            unblock(above)
        );
    }
    // The banks allow the top level somewhere, but the steps cannot get there: the next step up
    // from the highest pool.
    let want = levels_m[hk] + min_lift;
    let step = banks.max_above_bank(s[hj], next(hj), want);
    format!(
        "{held} The next pool up needs at least {} m (the smallest lift is {} m). The reach from \
         {} m to {} m blocks it: that pool would stand {} m above bank-full there (about {} m). \
         The banks allow the top level only from {} m up. {}",
        m(want),
        m(min_lift),
        ch(s[hj]),
        ch(next(hj)),
        m(step),
        m(banks.min_bank(s[hj], next(hj))),
        ch(s[bj]),
        unblock(step)
    )
}
