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
    cells: Vec<Vec<[Pt; 2]>>,
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
        let cell = (margin.abs() * 4.0).max(25.0);
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
        let mut cells = vec![Vec::new(); nx * ny];
        let at = |v: f64, o: f64, n: usize| (((v - o) / cell).floor().max(0.0) as usize).min(n - 1);
        for s in &segs {
            let (x0, x1) = (s[0][0].min(s[1][0]), s[0][0].max(s[1][0]));
            let (y0, y1) = (s[0][1].min(s[1][1]), s[0][1].max(s[1][1]));
            for ix in at(x0, origin[0], nx)..=at(x1, origin[0], nx) {
                for iy in at(y0, origin[1], ny)..=at(y1, origin[1], ny) {
                    cells[iy * nx + ix].push(*s);
                }
            }
        }
        RiverMask {
            index: PolygonIndex::new(polys),
            margin,
            cell,
            origin,
            nx,
            ny,
            cells,
        }
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
                for s in &self.cells[iy * self.nx + ix] {
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
        let inside = self.index.contains_xy(p);
        if self.margin < 0.0 {
            inside && !self.near_ring(p, -self.margin)
        } else {
            inside || self.near_ring(p, self.margin)
        }
    }
}
