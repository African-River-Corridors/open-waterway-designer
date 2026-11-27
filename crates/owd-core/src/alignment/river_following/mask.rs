//! The river polygon grown or shrunk by a margin, for many point tests: "is the line's centre in
//! the river?" (spec OPT-4, 3.4 and 4.4).

use crate::geom::overlay::PolygonIndex;
use crate::geom::{Polygon, Pt};

/// The river polygon with a margin: `+` grows it, `-` shrinks it.
#[derive(Debug, Clone)]
pub struct RiverMask {
    index: PolygonIndex,
    margin: f64,
    /// The rings' segments in square cells.
    cell: f64,
    origin: Pt,
    nx: usize,
    ny: usize,
    /// The segments of each cell: cell `i` holds `segs[idx[start[i]..start[i + 1]]]`.
    segs: Vec<[Pt; 2]>,
    start: Vec<u32>,
    idx: Vec<u32>,
    /// Each cell's answer when it is the same everywhere in it: `1` in, `0` out, `2` ask exactly
    /// (a ring passes within the margin of it).
    state: Vec<u8>,
}

impl RiverMask {
    /// The mask of `polys` with `margin` metres.
    pub fn new(polys: &[Polygon], margin: f64) -> RiverMask {
        let segs: Vec<[Pt; 2]> = polys
            .iter()
            .flat_map(|p| core::iter::once(&p.exterior).chain(&p.holes))
            .flat_map(|r| r.windows(2).map(|w| [w[0], w[1]]))
            .collect();
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for s in &segs {
            for q in s {
                for k in 0..2 {
                    lo[k] = lo[k].min(q[k]);
                    hi[k] = hi[k].max(q[k]);
                }
            }
        }
        // Cells of at least four margins and 25 m, and larger on a long river, so that there are at
        // most about four million of them.
        let area = ((hi[0] - lo[0]).max(1.0)) * ((hi[1] - lo[1]).max(1.0));
        let cell = (margin.abs() * 4.0).max(25.0).max((area / 4.0e6).sqrt());
        let pad = margin.abs() + cell;
        let origin = [lo[0] - pad, lo[1] - pad];
        let span = [hi[0] - lo[0] + 2.0 * pad, hi[1] - lo[1] + 2.0 * pad];
        let (nx, ny) = if segs.is_empty() {
            (1, 1)
        } else {
            (
                ((span[0] / cell).ceil() as usize).max(1),
                ((span[1] / cell).ceil() as usize).max(1),
            )
        };
        let at = |v: f64, o: f64, n: usize| (((v - o) / cell).floor().max(0.0) as usize).min(n - 1);
        let span_of = |s: &[Pt; 2]| {
            let (x0, x1) = (s[0][0].min(s[1][0]), s[0][0].max(s[1][0]));
            let (y0, y1) = (s[0][1].min(s[1][1]), s[0][1].max(s[1][1]));
            (
                at(x0, origin[0], nx)..=at(x1, origin[0], nx),
                at(y0, origin[1], ny)..=at(y1, origin[1], ny),
            )
        };
        // Count, then fill (compressed rows: no vector per cell).
        let mut count = vec![0u32; nx * ny + 1];
        for s in &segs {
            let (xs, ys) = span_of(s);
            for ix in xs {
                for iy in ys.clone() {
                    count[iy * nx + ix + 1] += 1;
                }
            }
        }
        for i in 1..count.len() {
            count[i] += count[i - 1];
        }
        let start = count.clone();
        let mut fill = count;
        let mut idx = vec![0u32; *start.last().unwrap_or(&0) as usize];
        for (k, s) in segs.iter().enumerate() {
            let (xs, ys) = span_of(s);
            for ix in xs {
                for iy in ys.clone() {
                    let c = iy * nx + ix;
                    idx[fill[c] as usize] = k as u32;
                    fill[c] += 1;
                }
            }
        }
        let mut m = RiverMask {
            index: PolygonIndex::new(polys),
            margin,
            cell,
            origin,
            nx,
            ny,
            segs,
            start,
            idx,
            state: Vec::new(),
        };
        // A cell no ring comes within half its diagonal and the margin of its centre answers the
        // same everywhere: its centre's answer. The centres' answers come row by row from where the
        // rings cross the row (even-odd), the rest are asked point by point.
        let reach = 0.5 * cell * core::f64::consts::SQRT_2 + margin.abs() + 1e-6;
        let mut cross: Vec<Vec<f64>> = vec![Vec::new(); ny];
        for sg in &m.segs {
            let (a, b) = (sg[0], sg[1]);
            if a[1] == b[1] {
                continue;
            }
            let (ylo, yhi) = (a[1].min(b[1]), a[1].max(b[1]));
            let r0 = (((ylo - origin[1]) / cell - 0.5).ceil().max(0.0)) as usize;
            let r1 = (((yhi - origin[1]) / cell - 0.5).floor()) as i64;
            for r in r0..=(r1.max(-1) as usize).min(ny.saturating_sub(1)) {
                if r1 < 0 {
                    break;
                }
                let y = origin[1] + (r as f64 + 0.5) * cell;
                // Half-open in y, as the even-odd rule counts a vertex once.
                if y < ylo || y >= yhi {
                    continue;
                }
                cross[r].push(a[0] + (y - a[1]) * (b[0] - a[0]) / (b[1] - a[1]));
            }
        }
        let mut state = vec![0u8; nx * ny];
        for (iy, xs) in cross.iter_mut().enumerate() {
            xs.sort_by(f64::total_cmp);
            let mut k = 0;
            for ix in 0..nx {
                let cx = origin[0] + (ix as f64 + 0.5) * cell;
                while k < xs.len() && xs[k] <= cx {
                    k += 1;
                }
                let inside = k % 2 == 1;
                let c = [cx, origin[1] + (iy as f64 + 0.5) * cell];
                state[iy * nx + ix] =
                    if m.start[iy * nx + ix + 1] > m.start[iy * nx + ix] || m.near_ring(c, reach) {
                        2
                    } else {
                        // Far from every ring: in the polygon with the margin exactly when inside it.
                        u8::from(inside)
                    };
            }
        }
        m.state = state;
        m
    }

    /// The margin, in metres.
    pub fn margin(&self) -> f64 {
        self.margin
    }

    /// True when `p` is within `r` metres of a ring.
    fn near_ring(&self, p: Pt, r: f64) -> bool {
        if r <= 0.0 {
            return false;
        }
        let at = |v: f64, o: f64, n: usize| -> usize {
            (((v - o) / self.cell).floor().max(0.0) as usize).min(n - 1)
        };
        let (ix0, ix1) = (
            at(p[0] - r, self.origin[0], self.nx),
            at(p[0] + r, self.origin[0], self.nx),
        );
        let (iy0, iy1) = (
            at(p[1] - r, self.origin[1], self.ny),
            at(p[1] + r, self.origin[1], self.ny),
        );
        for iy in iy0..=iy1 {
            for ix in ix0..=ix1 {
                let c = iy * self.nx + ix;
                for &k in &self.idx[self.start[c] as usize..self.start[c + 1] as usize] {
                    let s = &self.segs[k as usize];
                    if crate::geom::linear::point_segment_distance(p, s[0], s[1]) < r {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// True when `p` is in the river with the margin: inside the polygon and at least `-margin`
    /// from its rings (a negative margin), or inside or within `margin` of it (a positive one).
    pub fn contains(&self, p: Pt) -> bool {
        let (fx, fy) = (
            (p[0] - self.origin[0]) / self.cell,
            (p[1] - self.origin[1]) / self.cell,
        );
        if fx >= 0.0 && fy >= 0.0 && (fx as usize) < self.nx && (fy as usize) < self.ny {
            match self.state.get(fy as usize * self.nx + fx as usize) {
                Some(0) => return false,
                Some(1) => return true,
                _ => {}
            }
        } else if self.margin <= 0.0 && !self.state.is_empty() {
            // Beyond the grid, which holds every ring with room: out of the river.
            return false;
        }
        self.exact(p)
    }

    /// [`RiverMask::contains`], asked of the polygon itself.
    fn exact(&self, p: Pt) -> bool {
        let inside = self.index.contains_xy(p);
        if self.margin < 0.0 {
            inside && !self.near_ring(p, -self.margin)
        } else {
            inside || self.near_ring(p, self.margin)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cells' answers are the polygon's own, at points all over it, for both signs of margin.
    #[test]
    fn the_cells_answer_as_the_polygon_does() {
        let centre = crate::alignment::river_following::fixtures::sine(300.0, 2000.0, 6000.0);
        let poly = crate::alignment::river_following::fixtures::polygon(&centre, 120.0);
        for margin in [-3.0, 0.0, 20.0] {
            let m = RiverMask::new(std::slice::from_ref(&poly), margin);
            let mut seed = 7u64;
            let mut next = || {
                seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (seed >> 11) as f64 / (1u64 << 53) as f64
            };
            for _ in 0..20_000 {
                let p = [-200.0 + 6400.0 * next(), -500.0 + 1000.0 * next()];
                assert_eq!(m.contains(p), m.exact(p), "{p:?} at margin {margin}");
            }
        }
    }
}
