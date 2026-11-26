//! The buildings and the settlement land a pool floods, for the optimise stage (BLD-1): the count
//! table the weir search prices buildings with, and the report of the chosen cascade. The rule is
//! the weir search's own (`owd_core::optimise::buildings`), on the same flood field.
//!
//! The map's pool water (`pool_water.rs`) is a second implementation of "flooded"; near a weir its
//! count of buildings can differ from this one (see `owd_core::optimise::flood`).

use super::features::{BuildingFilters, FeatureInput, FeatureSet, polygon_contains};
use super::session::{DesignError, Session};
use owd_core::geom::Pt;
use owd_core::optimise::buildings::{BuildingField, BuildingTable, BuildingTerms};
use owd_core::optimise::flood::FloodField;
use owd_core::types::TerrainSampler;
use serde_json::{Value, json};
use std::sync::Arc;

/// How many flooded buildings the report lists, so the user can go and look.
pub const FIRST_LISTED: usize = 20;

/// The confidence bands the report counts flooded buildings in.
pub const BANDS: [(&str, f64, f64); 6] = [
    ("below 0.5", f64::NEG_INFINITY, 0.5),
    ("0.5 to 0.6", 0.5, 0.6),
    ("0.6 to 0.7", 0.6, 0.7),
    ("0.7 to 0.8", 0.7, 0.8),
    ("0.8 to 0.9", 0.8, 0.9),
    ("0.9 and over", 0.9, f64::INFINITY),
];

/// The buildings on the flood field, and their count table.
pub struct BuildingGround {
    pub set: Arc<FeatureSet>,
    pub field: BuildingField,
    pub table: BuildingTable,
    pub counted: Vec<bool>,
    pub filters: BuildingFilters,
}

impl BuildingGround {
    pub fn terms<'a>(&'a self, counted: &'a dyn Fn(usize) -> bool, cost: f64) -> BuildingTerms<'a> {
        BuildingTerms {
            field: &self.field,
            table: &self.table,
            counted,
            cost_per_building: cost,
        }
    }
}

impl Session {
    /// The buildings the optimise stage reads: `Ok(None)` when it needs none (no building cost and
    /// no buildings named). Refused, in plain words, when a building cost is set and there are no
    /// buildings to count.
    pub(super) fn stage_buildings(
        &self,
        cost: f64,
    ) -> Result<Option<Arc<FeatureSet>>, DesignError> {
        match (self.features_path(FeatureInput::Buildings), self.features(FeatureInput::Buildings)) {
            (_, Some(set)) => Ok(Some(set.clone())),
            (Some(path), None) if cost != 0.0 => Err(DesignError::Input(format!(
                "a building cost (`building_cost`) needs the buildings file `{path}`, which is \
                 missing or has changed: load it again (`owd import --buildings FILE`), or set the \
                 building cost to 0"
            ))),
            (None, None) if cost != 0.0 => Err(DesignError::Input(
                "a building cost (`building_cost`) needs buildings to count: load a buildings file \
                 (`owd import --buildings FILE`, or Buildings in the web app's Data), or set the \
                 building cost to 0"
                    .into(),
            )),
            _ => Ok(None),
        }
    }
}

/// Place the buildings on the flood field and build their count table on `breaks` and `levels`.
pub fn building_ground(
    set: Arc<FeatureSet>,
    filters: BuildingFilters,
    field: &FloodField,
    terrain: TerrainSampler,
    breaks: &[f64],
    levels: &[f64],
    min_depth_m: f64,
) -> Result<BuildingGround, DesignError> {
    let pts: Vec<Pt> = set.features.iter().map(|f| f.at).collect();
    let (xs, ys): (Vec<f64>, Vec<f64>) = pts.iter().map(|p| (p[0], p[1])).unzip();
    let ground = terrain(&xs, &ys);
    let bf = BuildingField::new(field, &pts, &ground, min_depth_m).map_err(DesignError::Input)?;
    let counted: Vec<bool> = set.features.iter().map(|f| filters.counts(f)).collect();
    let table = BuildingTable::new(&bf, &|i| counted[i], breaks, levels);
    Ok(BuildingGround {
        set,
        field: bf,
        table,
        counted,
        filters,
    })
}

/// The buildings report of the chosen cascade's pools: the totals (`buildings` of the flood
/// report) and one row a pool (`buildings_flooded`, `building_cost`, `buildings_apart`).
pub fn buildings_report(
    g: &BuildingGround,
    pools: &[(f64, f64, f64)],
    cost: f64,
) -> (Value, Vec<Value>) {
    let mut rows = Vec::new();
    let (mut counted, mut below_conf, mut below_area) = (0usize, 0usize, 0usize);
    let mut bands = [0usize; BANDS.len()];
    let mut no_conf = 0usize;
    let mut first: Vec<Value> = Vec::new();
    for (pool, &(a, b, level)) in pools.iter().enumerate() {
        let all = g.field.flooded(a, b, level, &|_| true);
        let mut n = 0;
        let mut apart = 0;
        for &i in &all {
            let f = &g.set.features[i];
            if g.counted[i] {
                n += 1;
            } else {
                apart += 1;
                if g.filters.below_confidence(f) {
                    below_conf += 1;
                } else {
                    below_area += 1;
                }
            }
            match BANDS
                .iter()
                .position(|(_, lo, hi)| f.confidence >= *lo && f.confidence < *hi)
            {
                Some(k) if f.confidence.is_finite() => bands[k] += 1,
                _ => no_conf += 1,
            }
            if g.counted[i] && first.len() < FIRST_LISTED {
                let at = &g.field.buildings[i];
                first.push(json!({
                    "id": f.id,
                    "lon_lat": [round6(f.lon_lat[0]), round6(f.lon_lat[1])],
                    "pool": pool + 1,
                    "chainage_m": at.chainage_m.round(),
                    "depth_m": ((level - at.ground_m) * 100.0).round() / 100.0,
                    "confidence": if f.confidence.is_finite() { json!(f.confidence) } else { Value::Null },
                }));
            }
        }
        counted += n;
        rows.push(json!({
            "buildings_flooded": n,
            "buildings_apart": apart,
            "building_cost": cost * n as f64,
        }));
    }
    let mut by_band = serde_json::Map::new();
    for (k, (name, _, _)) in BANDS.iter().enumerate() {
        by_band.insert((*name).into(), json!(bands[k]));
    }
    by_band.insert("not given".into(), json!(no_conf));
    let total = json!({
        "cost_per_building": cost,
        "min_depth_m": g.field.min_depth_m,
        "min_confidence": g.filters.min_confidence,
        "min_area_m2": g.filters.min_area_m2,
        "loaded": g.set.features.len(),
        "flooded": counted,
        "flooded_below_confidence": below_conf,
        "flooded_below_area": below_area,
        "by_confidence": by_band,
        "building_cost": cost * counted as f64,
        "first": first,
        "rule": "a building's centroid in land the pool floods by the weir search's rule, with the water there at least min_depth_m deep",
    });
    (total, rows)
}

fn round6(x: f64) -> f64 {
    (x * 1e6).round() / 1e6
}

/// The settlement land each pool floods, hectares, by the weir search's rule: the flooded cells
/// of the field whose centre is in a settlement.
pub fn settlement_hectares(
    set: &FeatureSet,
    field: &FloodField,
    pools: &[(f64, f64, f64)],
) -> Vec<f64> {
    // The settlements by a coarse grid of their bounding boxes, so each cell tests a few.
    const GRID: f64 = 500.0;
    let mut grid: std::collections::HashMap<(i64, i64), Vec<(usize, usize)>> = Default::default();
    for (i, f) in set.features.iter().enumerate() {
        for (j, p) in f.polygons.iter().enumerate() {
            let (mut x0, mut y0, mut x1, mut y1) = (
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            );
            for q in &p.exterior {
                x0 = x0.min(q[0]);
                y0 = y0.min(q[1]);
                x1 = x1.max(q[0]);
                y1 = y1.max(q[1]);
            }
            for gx in (x0 / GRID).floor() as i64..=(x1 / GRID).floor() as i64 {
                for gy in (y0 / GRID).floor() as i64..=(y1 / GRID).floor() as i64 {
                    grid.entry((gx, gy)).or_default().push((i, j));
                }
            }
        }
    }
    let ha = field.cell_m * field.cell_m / 10_000.0;
    pools
        .iter()
        .map(|&(a, b, level)| {
            let n = field
                .flooded_cells(a, b, level, false)
                .into_iter()
                .filter(|c| {
                    grid.get(&((c[0] / GRID).floor() as i64, (c[1] / GRID).floor() as i64))
                        .is_some_and(|list| {
                            list.iter()
                                .any(|&(i, j)| polygon_contains(&set.features[i].polygons[j], *c))
                        })
                })
                .count();
            n as f64 * ha
        })
        .collect()
}
