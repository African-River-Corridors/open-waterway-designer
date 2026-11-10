//! The stations a pair of banks is read at (spec 0013): arc length, resampling, normals, the
//! segment index, and the offsets and widths along a line. Moved here from
//! `owd_format::design::midline` (ticket B3a) so the bank checks of the core can read banks;
//! `midline` re-exports every name.
//!
//! The geometry is planar, in project metres. Square roots go through `libm`, so native and wasm give
//! the same bits.
//!
//! Label: property

type Pt = [f64; 2];

/// The passes of the mid-line.
pub const MIDLINE_PASSES: usize = 3;

/// The least a mid-line station may move in a pass, in metres (it may move one river width).
pub const MIN_MOVE_M: f64 = 200.0;

/// The farthest a station looks for a bank along its normal, each way, in metres (the bank search
/// of the mask reaches 1.5 km).
pub const MAX_RAY_M: f64 = 2_000.0;

pub fn dist(a: Pt, b: Pt) -> f64 {
    libm::hypot(b[0] - a[0], b[1] - a[1])
}

/// Arc length at each vertex.
pub fn arc(line: &[Pt]) -> Vec<f64> {
    let mut s = Vec::with_capacity(line.len());
    s.push(0.0);
    for w in line.windows(2) {
        s.push(s[s.len() - 1] + dist(w[0], w[1]));
    }
    s
}

/// The point at arc length `at` (clamped), given the arc lengths `s` of `line`.
pub fn point_at(line: &[Pt], s: &[f64], at: f64) -> Pt {
    if line.len() == 1 {
        return line[0];
    }
    let at = at.clamp(0.0, s[s.len() - 1]);
    let i = s
        .partition_point(|v| *v <= at)
        .saturating_sub(1)
        .min(line.len() - 2);
    let seg = s[i + 1] - s[i];
    let f = if seg > 0.0 { (at - s[i]) / seg } else { 0.0 };
    [
        line[i][0] + f * (line[i + 1][0] - line[i][0]),
        line[i][1] + f * (line[i + 1][1] - line[i][1]),
    ]
}

/// Evenly spaced stations along a line, about `step` apart, both ends included.
pub fn resample(line: &[Pt], step: f64) -> Vec<Pt> {
    if line.len() < 2 {
        return line.to_vec();
    }
    let s = arc(line);
    let len = s[s.len() - 1];
    let n = ((len / step.max(1e-6)).round() as usize).max(1);
    (0..=n)
        .map(|k| point_at(line, &s, len * k as f64 / n as f64))
        .collect()
}

/// The half-length of line, each side of a station, whose direction gives the station's normal. Long
/// enough that the normals of a line still being moved, or of rough banks, do not swing.
pub const TANGENT_HALF_M: f64 = 100.0;

/// Unit left normals of a line, from the chord between the points `half_m` of arc either side.
pub fn arc_normals(line: &[Pt], half_m: f64) -> Vec<Pt> {
    let s = arc(line);
    line.iter()
        .zip(&s)
        .map(|(_, at)| {
            let (a, b) = (
                point_at(line, &s, at - half_m),
                point_at(line, &s, at + half_m),
            );
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let l = libm::hypot(dx, dy).max(1e-12);
            [-dy / l, dx / l]
        })
        .collect()
}

/// Unit left normals of a line, from the direction over `k` stations either side.
pub fn normals(line: &[Pt], k: usize) -> Vec<Pt> {
    let n = line.len();
    (0..n)
        .map(|i| {
            let (a, b) = (line[i.saturating_sub(k)], line[(i + k).min(n - 1)]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let l = libm::hypot(dx, dy).max(1e-12);
            [-dy / l, dx / l]
        })
        .collect()
}

/// A grid of the segments of a line, so a ray tests only the segments near it.
pub struct SegIndex<'a> {
    line: &'a [Pt],
    x0: f64,
    y0: f64,
    cell: f64,
    nx: usize,
    ny: usize,
    cells: Vec<Vec<u32>>,
}

/// A bank segment longer than this many times the bank's median segment is a gap (issue #373)...
pub const GAP_FACTOR: f64 = 20.0;
/// ...and at least this long (m). Provisional. The gaps of the whole Sassandra run were 1.75 to
/// 11 km; a fault the checks repair (a bank that spikes out or dives across a river up to 500 m
/// wide) has edges up to about 1.4 km, and stays a bank. A bank drawn by hand with even long
/// segments has no gap at all (the median rule).
pub const GAP_MIN_M: f64 = 1500.0;

/// The length over which a segment of `line` is a gap: two vertices far apart joined by a straight
/// chord, which may cut across land and past the other bank.
pub fn gap_length(line: &[Pt]) -> f64 {
    let mut seg: Vec<f64> = line.windows(2).map(|w| dist(w[0], w[1])).collect();
    if seg.is_empty() {
        return f64::INFINITY;
    }
    seg.sort_by(f64::total_cmp);
    (GAP_FACTOR * seg[seg.len() / 2]).max(GAP_MIN_M)
}

/// The index of a bank, without its gaps ([`gap_length`]), for the mid-line: a normal that meets
/// only a gap meets no bank, so its station keeps its place. The checks read every segment: with
/// a gap left out, a normal would read through it to the bank beyond.
pub fn bank_index(line: &[Pt], cell: f64) -> SegIndex<'_> {
    SegIndex::with_max_segment(line, cell, gap_length(line))
}

impl<'a> SegIndex<'a> {
    pub fn new(line: &'a [Pt], cell: f64) -> SegIndex<'a> {
        Self::with_max_segment(line, cell, f64::INFINITY)
    }

    /// The index of the segments of `line` no longer than `max_segment`.
    pub fn with_max_segment(line: &'a [Pt], cell: f64, max_segment: f64) -> SegIndex<'a> {
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for p in line {
            for k in 0..2 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        if line.is_empty() {
            lo = [0.0; 2];
            hi = [0.0; 2];
        }
        let nx = ((hi[0] - lo[0]) / cell).floor() as usize + 1;
        let ny = ((hi[1] - lo[1]) / cell).floor() as usize + 1;
        let mut cells = vec![Vec::new(); nx * ny];
        // Each segment goes in exactly the cells it passes through (a grid walk), not in every cell
        // of its box: a long diagonal segment would otherwise fill a whole square of cells.
        for (i, w) in line.windows(2).enumerate() {
            let (a, b) = (w[0], w[1]);
            let len = dist(a, b);
            if len > max_segment {
                continue;
            }
            let d = if len > 0.0 {
                [(b[0] - a[0]) / len, (b[1] - a[1]) / len]
            } else {
                [1.0, 0.0]
            };
            grid_walk([lo[0], lo[1]], cell, nx, ny, a, d, len, |c, _| {
                cells[c].push(i as u32);
                true
            });
        }
        SegIndex {
            line,
            x0: lo[0],
            y0: lo[1],
            cell,
            nx,
            ny,
            cells,
        }
    }

    /// The signed distances along `d` (a unit vector) from `p` to the first crossing with the line
    /// ahead (positive) and behind (negative), within `max_t` either way. Each way walks the grid
    /// out from `p` and stops once a crossing is nearer than the next cell.
    pub fn hits(&self, p: Pt, d: Pt, max_t: f64, buf: &mut Vec<u32>) -> (Option<f64>, Option<f64>) {
        let mut one_way = |sign: f64| -> Option<f64> {
            let dir = [d[0] * sign, d[1] * sign];
            let mut best: Option<f64> = None;
            buf.clear();
            grid_walk(
                [self.x0, self.y0],
                self.cell,
                self.nx,
                self.ny,
                p,
                dir,
                max_t,
                |c, t_in| {
                    if best.is_some_and(|b| b < t_in) {
                        return false;
                    }
                    for &i in &self.cells[c] {
                        let (a, b) = (self.line[i as usize], self.line[i as usize + 1]);
                        let e = [b[0] - a[0], b[1] - a[1]];
                        let den = dir[0] * e[1] - dir[1] * e[0];
                        if den.abs() < 1e-12 {
                            continue;
                        }
                        let w = [a[0] - p[0], a[1] - p[1]];
                        let t = (w[0] * e[1] - w[1] * e[0]) / den;
                        let u = (w[0] * dir[1] - w[1] * dir[0]) / den;
                        if (-1e-9..=1.0 + 1e-9).contains(&u)
                            && (0.0..=max_t).contains(&t)
                            && best.is_none_or(|v| t < v)
                        {
                            best = Some(t);
                        }
                    }
                    true
                },
            );
            best
        };
        let ahead = one_way(1.0);
        let behind = one_way(-1.0).map(|t| -t);
        (ahead, behind)
    }

    /// The signed distance to the nearest crossing either way.
    pub fn nearest_hit(&self, p: Pt, d: Pt, max_t: f64, buf: &mut Vec<u32>) -> Option<f64> {
        match self.hits(p, d, max_t, buf) {
            (Some(a), Some(b)) => Some(if a <= -b { a } else { b }),
            (a, b) => a.or(b),
        }
    }
}

/// Walk the cells of a grid (origin `lo`, square `cell`, `nx` by `ny`) that a ray from `p` along the
/// unit vector `d` passes, up to `max_t`, in order, calling `visit(cell index, distance where the ray
/// enters it)`; `visit` returning false stops the walk. Cells outside the grid are skipped.
#[allow(clippy::too_many_arguments)]
pub fn grid_walk(
    lo: Pt,
    cell: f64,
    nx: usize,
    ny: usize,
    p: Pt,
    d: Pt,
    max_t: f64,
    mut visit: impl FnMut(usize, f64) -> bool,
) {
    let fx = (p[0] - lo[0]) / cell;
    let fy = (p[1] - lo[1]) / cell;
    let (mut ix, mut iy) = (fx.floor() as i64, fy.floor() as i64);
    let step = |v: f64| if v > 0.0 { 1i64 } else { -1 };
    let (sx, sy) = (step(d[0]), step(d[1]));
    let inf = f64::INFINITY;
    let next = |f: f64, i: i64, s: i64, dv: f64| -> f64 {
        if dv.abs() < 1e-15 {
            inf
        } else {
            let edge = if s > 0 { (i + 1) as f64 } else { i as f64 };
            (edge - f) * cell / dv
        }
    };
    let (mut tx, mut ty) = (next(fx, ix, sx, d[0]), next(fy, iy, sy, d[1]));
    let (dx, dy) = (
        if d[0].abs() < 1e-15 {
            inf
        } else {
            cell / d[0].abs()
        },
        if d[1].abs() < 1e-15 {
            inf
        } else {
            cell / d[1].abs()
        },
    );
    let mut t_in = 0.0;
    loop {
        if ix >= 0
            && iy >= 0
            && (ix as usize) < nx
            && (iy as usize) < ny
            && !visit(iy as usize * nx + ix as usize, t_in)
        {
            return;
        }
        if tx < ty {
            t_in = tx;
            tx += dx;
            ix += sx;
        } else {
            t_in = ty;
            ty += dy;
            iy += sy;
        }
        if t_in > max_t {
            return;
        }
    }
}

/// The offsets of the two banks from a station along its left normal `n`: the first crossing of the
/// left bank on the left, and of the right bank on the right. A bank met only on the wrong side gives
/// its nearest crossing there (a negative left or a positive right offset).
pub fn side_hits(
    li: &SegIndex,
    ri: &SegIndex,
    p: Pt,
    n: Pt,
    buf: &mut Vec<u32>,
) -> (Option<f64>, Option<f64>) {
    let (la, lb) = li.hits(p, n, MAX_RAY_M, buf);
    let (ra, rb) = ri.hits(p, n, MAX_RAY_M, buf);
    (la.or(lb), rb.or(ra))
}

/// Per station of `centre`: the signed distances along its left normal to the first crossing of the
/// left bank on the left and of the right bank on the right (see `side_hits`; `None` where the normal
/// meets the bank on neither side within [`MAX_RAY_M`]).
pub fn bank_offsets(centre: &[Pt], left: &[Pt], right: &[Pt]) -> Vec<(Option<f64>, Option<f64>)> {
    let cell = cell_for(left, right);
    let (li, ri) = (bank_index(left, cell), bank_index(right, cell));
    let nrm = arc_normals(centre, tangent_half(centre, &li, &ri));
    let mut buf = Vec::new();
    centre
        .iter()
        .zip(&nrm)
        .map(|(p, n)| side_hits(&li, &ri, *p, *n, &mut buf))
        .collect()
}

/// How far each station of `centre` is from the midpoint of the banks along its normal: half the
/// sum of the two offsets (`None` where a bank is not met).
pub fn midline_offsets(centre: &[Pt], left: &[Pt], right: &[Pt]) -> Vec<Option<f64>> {
    bank_offsets(centre, left, right)
        .into_iter()
        .map(|o| match o {
            (Some(l), Some(r)) => Some(((l + r) / 2.0).abs()),
            _ => None,
        })
        .collect()
}

/// A grid cell about the size of a typical segment, at least 25 m.
pub fn cell_for(left: &[Pt], right: &[Pt]) -> f64 {
    let (a, b) = (arc(left), arc(right));
    let len = a.last().copied().unwrap_or(0.0) + b.last().copied().unwrap_or(0.0);
    let n = (left.len() + right.len()).max(2) as f64;
    (4.0 * len / n).clamp(25.0, 500.0)
}

/// The median width across a line's stations, by the first crossings either side (`None` with none).
pub fn median_width(line: &[Pt], left: &[Pt], right: &[Pt]) -> Option<f64> {
    let cell = cell_for(left, right);
    let (li, ri) = (bank_index(left, cell), bank_index(right, cell));
    median_width_on(line, &li, &ri)
}

pub fn median_width_on(line: &[Pt], li: &SegIndex, ri: &SegIndex) -> Option<f64> {
    let nrm = arc_normals(line, TANGENT_HALF_M);
    let mut buf = Vec::new();
    let mut w: Vec<f64> = line
        .iter()
        .zip(&nrm)
        .filter_map(|(p, n)| match side_hits(li, ri, *p, *n, &mut buf) {
            (Some(l), Some(r)) if l > r => Some(l - r),
            _ => None,
        })
        .collect();
    if w.is_empty() {
        return None;
    }
    w.sort_by(f64::total_cmp);
    Some(w[w.len() / 2])
}

/// The half-length of line that gives a normal: [`TANGENT_HALF_M`], or half the median width if
/// that is more, up to 1 km.
pub fn tangent_half(line: &[Pt], li: &SegIndex, ri: &SegIndex) -> f64 {
    median_width_on(line, li, ri)
        .map_or(TANGENT_HALF_M, |w| (w / 2.0).clamp(TANGENT_HALF_M, 1_000.0))
}
