//! A window's ground (OPT-5): the terrain sampled once onto a grid over the window's corridor and as
//! far beyond it as a section reaches, with the river and the corridor read through polygon
//! indexes (one test per cell, not one walk of every vertex).

use crate::geom::Polygon;
use crate::geom::overlay::PolygonIndex;
use crate::optimise::lattice::TerrainGrid;
use crate::types::{Refusal, TerrainSampler};

/// `(xmin, ymin, xmax, ymax)` of the polygons' outer rings.
pub fn bounds(polys: &[Polygon]) -> (f64, f64, f64, f64) {
    let mut b = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for p in polys {
        for q in &p.exterior {
            b.0 = b.0.min(q[0]);
            b.1 = b.1.min(q[1]);
            b.2 = b.2.max(q[0]);
            b.3 = b.3.max(q[1]);
        }
    }
    b
}

/// The grid of a window: cells of `cell_m` over the corridor's bounds grown by `pad_m` (the reach
/// of a section beyond the corridor, so a side slope at the corridor's edge reads ground, never the
/// free "unknown" off the grid). `wet` is the river polygon at the cell centre; `corridor` the
/// corridor there; `valid` known ground inside the corridor.
pub fn window_grid(
    terrain: TerrainSampler,
    river: &PolygonIndex,
    corridor: &[Polygon],
    pad_m: f64,
    cell_m: f64,
) -> Result<TerrainGrid, Refusal> {
    if corridor.is_empty() {
        return Err("the window's corridor is empty".into());
    }
    let (mut xmin, mut ymin, mut xmax, mut ymax) = bounds(corridor);
    xmin -= pad_m;
    ymin -= pad_m;
    xmax += pad_m;
    ymax += pad_m;
    let w = ((xmax - xmin) / cell_m).ceil() as usize + 1;
    let h = ((ymax - ymin) / cell_m).ceil() as usize + 1;
    let inside = PolygonIndex::new(corridor);
    let mut ground = Vec::with_capacity(h);
    let mut wet = Vec::with_capacity(h);
    let mut valid = Vec::with_capacity(h);
    let mut corr = Vec::with_capacity(h);
    // A row at a time: one terrain call per row keeps the samples a sampler holds at once small.
    let xs: Vec<f64> = (0..w).map(|i| xmin + i as f64 * cell_m).collect();
    for r in 0..h {
        let y = ymax - r as f64 * cell_m;
        let ys = vec![y; w];
        let z = crate::types::grid::sample_grid(terrain, &xs, &ys, cell_m);
        if z.len() != w {
            return Err(format!(
                "the terrain gave {} heights for {w} points",
                z.len()
            ));
        }
        let mut wet_row = Vec::with_capacity(w);
        let mut valid_row = Vec::with_capacity(w);
        let mut corr_row = Vec::with_capacity(w);
        for (c, &x) in xs.iter().enumerate() {
            let p = [x, y];
            wet_row.push(river.contains_xy(p));
            let inc = inside.contains_xy(p);
            corr_row.push(inc);
            valid_row.push(inc && z[c].is_finite());
        }
        ground.push(z);
        wet.push(wet_row);
        valid.push(valid_row);
        corr.push(corr_row);
    }
    Ok(TerrainGrid {
        ground,
        wet,
        valid,
        corridor: corr,
        x0: xmin,
        y0: ymax,
        cell_m,
    })
}

/// A boolean mask on the cells of a grid: the permitted ground of a river-following line (OPT-5
/// 3.16), read at the nearest cell.
#[derive(Debug, Clone, PartialEq)]
pub struct CellMask {
    pub cells: Vec<Vec<bool>>,
    pub x0: f64,
    pub y0: f64,
    pub cell_m: f64,
}

impl CellMask {
    /// The mask of `polys` on the cells of `grid`.
    pub fn on(grid: &TerrainGrid, polys: &[Polygon]) -> Self {
        let idx = PolygonIndex::new(polys);
        let cells = (0..grid.ground.len())
            .map(|r| {
                (0..grid.ground[r].len())
                    .map(|c| {
                        idx.contains_xy([
                            grid.x0 + c as f64 * grid.cell_m,
                            grid.y0 - r as f64 * grid.cell_m,
                        ])
                    })
                    .collect()
            })
            .collect();
        Self {
            cells,
            x0: grid.x0,
            y0: grid.y0,
            cell_m: grid.cell_m,
        }
    }

    /// Whether the nearest cell is in the mask; false off the grid.
    pub fn at(&self, x: f64, y: f64) -> bool {
        let r = libm::rint((self.y0 - y) / self.cell_m);
        let c = libm::rint((x - self.x0) / self.cell_m);
        if r < 0.0 || c < 0.0 {
            return false;
        }
        self.cells
            .get(r as usize)
            .and_then(|row| row.get(c as usize))
            .copied()
            .unwrap_or(false)
    }
}
