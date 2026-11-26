//! Buildings a pool floods (BLD-1): a count per reach and level, by the flood rule of
//! [`super::flood`], so the weir search can price them.
//!
//! # The rule
//!
//! A building is a point (its centroid). It stands in the cell of the [`FloodField`] that holds
//! the point. It is FLOODED by a pool at level `L` over `[start_m, end_m)` when:
//!
//! 1. its cell is land a pool can flood (outside the river polygon, with a DEM value, inside the
//!    line's ends) and that land is flooded at `L` by the field's rule: the cell's fill level is
//!    below `L`, its nearest chainage is in the reach, and it is within the flood distance;
//! 2. the water at the building is at least `min_depth_m` deep: `L - ground >= min_depth_m`,
//!    where `ground` is the DEM at the building's point (the cell's ground when the DEM has none
//!    there). A puddle does not count.
//!
//! Both are one number per building, the lowest level that floods it, so the count over a reach
//! at a level is a prefix sum like the flooded area ([`BuildingTable`]): built once in
//! `O(B log S + (S + 1) L)` for `B` buildings, `S` breaks (sites) and `L` levels, then each
//! (site, site, level) the search asks for is two reads. Indexing the field's cells to find each
//! building's cell is `O(C)` for `C` cells, once.

use super::flood::{FloodCell, FloodField};
use crate::geom::Pt;
use crate::types::Refusal;
use std::cmp::Ordering;
use std::collections::HashMap;

/// A building as the count reads it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BuildingAt {
    /// Chainage of its cell's nearest point of the line; NaN when it is in no cell of land.
    pub chainage_m: f64,
    /// The lowest level that floods it by the rule above; +inf when none can.
    pub floods_above_m: f64,
    /// The level must also be at or above this one (ground + least depth).
    pub depth_level_m: f64,
    /// The ground at the building, m.
    pub ground_m: f64,
}

impl BuildingAt {
    /// Flooded by a pool at `level_m` (its reach not checked).
    pub fn flooded_at(&self, level_m: f64) -> bool {
        self.floods_above_m < level_m && level_m >= self.depth_level_m - 1e-9
    }
}

/// The buildings on a flood field.
#[derive(Debug, Clone, PartialEq)]
pub struct BuildingField {
    pub min_depth_m: f64,
    /// By the caller's order.
    pub buildings: Vec<BuildingAt>,
}

fn is_land(c: &FloodCell) -> bool {
    !c.wet && !c.beyond_end && c.ground_m.is_finite()
}

impl BuildingField {
    /// Place each building (`points`, project CRS, with the DEM's `ground` at each; NaN: none) on
    /// `field`.
    pub fn new(
        field: &FloodField,
        points: &[Pt],
        ground: &[f64],
        min_depth_m: f64,
    ) -> Result<BuildingField, Refusal> {
        if !(min_depth_m.is_finite() && min_depth_m >= 0.0) {
            return Err(
                "the least depth that floods a building must be a number of metres >= 0".into(),
            );
        }
        if ground.len() != points.len() {
            return Err("one ground level per building is needed".into());
        }
        let key = |r: i32, c: i32| ((r as u32 as u64) << 32) | (c as u32 as u64);
        let index: HashMap<u64, u32> = field
            .cells
            .iter()
            .enumerate()
            .map(|(i, c)| (key(c.row, c.col), i as u32))
            .collect();
        let buildings = points
            .iter()
            .zip(ground)
            .map(|(p, &g)| {
                let col = libm::rint((p[0] - field.x0) / field.cell_m) as i32;
                let row = libm::rint((field.y0 - p[1]) / field.cell_m) as i32;
                let cell = index
                    .get(&key(row, col))
                    .map(|&i| &field.cells[i as usize])
                    .filter(|c| is_land(c));
                match cell {
                    Some(c) => {
                        let ground_m = if g.is_finite() { g } else { c.ground_m };
                        BuildingAt {
                            chainage_m: c.chainage_m,
                            floods_above_m: c.fill_m,
                            depth_level_m: ground_m + min_depth_m,
                            ground_m,
                        }
                    }
                    None => BuildingAt {
                        chainage_m: f64::NAN,
                        floods_above_m: f64::INFINITY,
                        depth_level_m: f64::INFINITY,
                        ground_m: g,
                    },
                }
            })
            .collect();
        Ok(BuildingField {
            min_depth_m,
            buildings,
        })
    }

    /// The buildings a pool at `level_m` floods over `[start_m, end_m)`, by index; `only` keeps
    /// those it says yes to.
    pub fn flooded(
        &self,
        start_m: f64,
        end_m: f64,
        level_m: f64,
        only: &dyn Fn(usize) -> bool,
    ) -> Vec<usize> {
        self.buildings
            .iter()
            .enumerate()
            .filter(|(i, b)| {
                b.chainage_m >= start_m && b.chainage_m < end_m && b.flooded_at(level_m) && only(*i)
            })
            .map(|(i, _)| i)
            .collect()
    }
}

/// Buildings flooded for every (break, level), counted ones only: a prefix sum like
/// [`super::flood::FloodTable`]. It gives exactly what [`BuildingField::flooded`] counts.
#[derive(Debug, Clone, PartialEq)]
pub struct BuildingTable {
    breaks_m: Vec<f64>,
    levels_m: Vec<f64>,
    /// `cum[b][l]`: counted buildings with chainage below `breaks_m[b]` flooded at `levels_m[l]`.
    cum: Vec<Vec<u32>>,
}

impl BuildingTable {
    /// `counted(i)`: building `i` is costed (its confidence and area pass the filters).
    pub fn new(
        field: &BuildingField,
        counted: &dyn Fn(usize) -> bool,
        breaks_m: &[f64],
        levels_m: &[f64],
    ) -> BuildingTable {
        let sorted = |v: &[f64]| {
            let mut v: Vec<f64> = v.iter().copied().filter(|x| x.is_finite()).collect();
            v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
            v.dedup();
            v
        };
        let breaks = sorted(breaks_m);
        let levels = sorted(levels_m);
        let n_l = levels.len();
        let mut cum = vec![vec![0u32; n_l]; breaks.len() + 1];
        for (i, b) in field.buildings.iter().enumerate() {
            if !b.chainage_m.is_finite() || !counted(i) {
                continue;
            }
            // The first level that floods it: above the fill level, and deep enough.
            let l = levels
                .partition_point(|&v| v <= b.floods_above_m)
                .max(levels.partition_point(|&v| v < b.depth_level_m - 1e-9));
            if l >= n_l {
                continue;
            }
            let s = breaks.partition_point(|&x| x <= b.chainage_m);
            cum[s][l] += 1;
        }
        for row in &mut cum {
            for l in 1..n_l {
                row[l] += row[l - 1];
            }
        }
        for s in 1..cum.len() {
            let (head, tail) = cum.split_at_mut(s);
            for l in 0..n_l {
                tail[0][l] += head[s - 1][l];
            }
        }
        BuildingTable {
            breaks_m: breaks,
            levels_m: levels,
            cum,
        }
    }

    fn find(v: &[f64], x: f64) -> Option<usize> {
        let i = v.partition_point(|&b| b < x - 1e-6);
        (i < v.len() && (v[i] - x).abs() <= 1e-6).then_some(i)
    }

    /// Counted buildings flooded over `[start_m, end_m)` at `level_m`; `None` when an end or the
    /// level is not in the table.
    pub fn count(&self, start_m: f64, end_m: f64, level_m: f64) -> Option<u32> {
        let a = Self::find(&self.breaks_m, start_m)?;
        let b = Self::find(&self.breaks_m, end_m)?;
        let l = Self::find(&self.levels_m, level_m)?;
        if b <= a {
            return Some(0);
        }
        Some(self.cum[b][l] - self.cum[a][l])
    }
}

/// The building term of the cascade's reach cost.
#[derive(Clone, Copy)]
pub struct BuildingTerms<'a> {
    pub field: &'a BuildingField,
    pub table: &'a BuildingTable,
    pub counted: &'a dyn Fn(usize) -> bool,
    /// Money per counted building flooded.
    pub cost_per_building: f64,
}

impl BuildingTerms<'_> {
    /// Counted buildings flooded by the pool.
    pub fn count(&self, start_m: f64, end_m: f64, level_m: f64) -> usize {
        match self.table.count(start_m, end_m, level_m) {
            Some(n) => n as usize,
            None => self
                .field
                .flooded(start_m, end_m, level_m, self.counted)
                .len(),
        }
    }

    pub fn cost(&self, start_m: f64, end_m: f64, level_m: f64) -> f64 {
        self.cost_per_building * self.count(start_m, end_m, level_m) as f64
    }
}

/// `reach_cost = inner(start, end, level) + cost per building x buildings flooded`; an infinite
/// inner cost (a pool over the bank limit) stays infinite.
pub fn with_building_cost<'a>(
    inner: &'a dyn Fn(f64, f64, f64) -> Result<f64, Refusal>,
    terms: BuildingTerms<'a>,
) -> impl Fn(f64, f64, f64) -> Result<f64, Refusal> + 'a {
    move |a, b, level| {
        let c = inner(a, b, level)?;
        if !c.is_finite() {
            return Ok(c);
        }
        Ok(c + terms.cost(a, b, level))
    }
}
