//! The optimise stage (spec 0002): the weir cascade search sets the weir sites and pool levels, and
//! the pool lattice refines the design line (spec 0017 ticket 6: one design line per run). Its line
//! is stored as `results.design_line`; once it is there, it is the design line the later stages read. The calls are those of the `optimise_pools` example:
//! `build_level_table`, `search_cascade` with an earthworks reach cost, then `optimise_pools`.
//!
//! A design with no DEM has no ground to cut: the stage then skips the cascade and searches the whole
//! line on plan area, the m2 of channel footprint outside the river polygon (`PLAN_AREA`), and says so.
//!
//! The baseline is the alignment's design line, else the river centreline. Chainages are
//! metres along it. Weights are settings, not prices: 1.0 / 1.0 minimises volume.

use super::session::{DesignError, ENGINE_VERSION, Session, Stage, StageStatus};
use crate::types::TerrainSampler;
use owd_core::alignment::radius_check::{default_check_chord, min_radius_on_chord, radius_ok};
use owd_core::cut_model::CUT_MODEL;
use owd_core::geom::{Pt, linear};
use owd_core::optimise::cascade::{
    CascadeConfig, SectionOptions, build_level_table, cascade_levels, earthworks_reach_cost,
    search_cascade,
};
use owd_core::optimise::lattice::{
    HalfWidthSpec, LatticeConfig, PoolOptions, SectionSpec, optimise_pools,
};
use owd_core::volumes::sections::HalfWidth;
use serde_json::{Value, json};

/// What `optimised_on` says when the stage ran with no DEM: the search cost was the plan area of
/// the channel's footprint outside the river polygon (m2, times the land weight), plus the length
/// cost, and not a volume.
pub const PLAN_AREA: &str = "plan_area";

/// The same, in plain words: in the lattice's details and among the design line's warnings.
pub const PLAN_AREA_NOTE: &str = "the design has no DEM, so the design line was optimised on plan \
    area (the m2 of channel outside the river polygon), not on volume: no weirs or pool levels were \
    chosen. Import a DEM and run the optimise stage again to optimise on volume";

/// The id and the method of the lattice's design line.
const LATTICE: &str = "lattice";

/// The settings of the optimise stage, typed. A key the stage does not know is refused by name.
#[derive(Debug, Clone, PartialEq)]
pub struct OptimiseSettings {
    /// With no candidate sites given: a site every this many metres of design line.
    pub site_step_m: f64,
    /// Candidate weir chainages along the design line, in metres; `None`: every `site_step_m`.
    pub candidate_sites_m: Option<Vec<f64>>,
    /// The lifts a weir may have, in metres.
    pub lifts_m: Vec<f64>,
    pub terminal_min_level_m: f64,
    pub min_spacing_m: f64,
    /// The water level below the first weir; `None`: the water level at the line's start.
    pub base_level_m: Option<f64>,
    pub k_best: i64,
    /// The weight of dredging in the cost.
    pub wet_weight: f64,
    /// The weight of dry cut in the cost.
    pub dry_weight: f64,
    /// The cost of one metre of design line (sailing distance), in the units of the cut cost (one
    /// m3 of cut at weight 1 costs 1). The pool lattice pays it for every metre of its line and of
    /// the baseline it compares with. 0 (the default) is off; then it is not written into the
    /// document, so a design run without it reads, hashes and saves as before.
    pub length_weight: f64,
    /// The cost of one m2 of land taken, in m3 of cut per m2: the pool lattice pays it for every m2
    /// of the channel's footprint at the water surface that lies outside the river polygon, cut or
    /// no cut, on its line and on the baseline it compares with. A value equal to the channel depth
    /// prices land like a cut of the full depth. 0 (the default) is off; then it is not written into
    /// the document, so a design run without it reads, hashes and saves as before.
    pub land_weight: f64,
    pub structure_fixed: f64,
    pub structure_per_lift: f64,
    pub structure_power: f64,
    pub cut_model: String,
    pub station_m: f64,
    pub maxw_m: f64,
    pub half_width_m: Option<f64>,
    pub side_slope: f64,
    pub rake_half_m: f64,
    pub rake_step_m: f64,
    pub ds_step_m: f64,
    pub pitch_m: f64,
    pub n_heading: i64,
    pub radius_multiples: Vec<f64>,
    pub corridor_half_m: f64,
    /// Hold the pool search to the corridor: no point of the lattice's line may lie further than
    /// `corridor_half_m` from the baseline (the alignment's design line, else the river centreline).
    /// False (the default) is the search as it was, where the corridor only feeds the heuristic;
    /// then it is not written into the document.
    pub corridor_limit: bool,
    pub cell_m: f64,
    pub check_chord_m: Option<f64>,
}

impl Default for OptimiseSettings {
    fn default() -> Self {
        OptimiseSettings {
            site_step_m: 500.0,
            candidate_sites_m: None,
            lifts_m: vec![0.5, 1.0],
            terminal_min_level_m: 0.5,
            min_spacing_m: 0.0,
            base_level_m: None,
            k_best: 5,
            wet_weight: 1.0,
            dry_weight: 1.0,
            length_weight: 0.0,
            land_weight: 0.0,
            structure_fixed: 1.0e5,
            structure_per_lift: 2.0e5,
            structure_power: 1.0,
            cut_model: CUT_MODEL.to_string(),
            station_m: 25.0,
            maxw_m: 200.0,
            half_width_m: None,
            side_slope: 3.0,
            rake_half_m: 250.0,
            rake_step_m: 4.0,
            ds_step_m: 8.0,
            pitch_m: 12.0,
            n_heading: 16,
            radius_multiples: vec![1.0, 1.5, 3.0],
            corridor_half_m: 350.0,
            corridor_limit: false,
            cell_m: 2.0,
            check_chord_m: None,
        }
    }
}

impl OptimiseSettings {
    /// The settings as the stage writes them into the document.
    pub fn to_value(&self) -> Value {
        let mut v = json!({
            "site_step_m": self.site_step_m,
            "candidate_sites_m": self.candidate_sites_m,
            "lifts_m": self.lifts_m,
            "terminal_min_level_m": self.terminal_min_level_m,
            "min_spacing_m": self.min_spacing_m,
            "base_level_m": self.base_level_m,
            "k_best": self.k_best,
            "wet_weight": self.wet_weight,
            "dry_weight": self.dry_weight,
            "structure_fixed": self.structure_fixed,
            "structure_per_lift": self.structure_per_lift,
            "structure_power": self.structure_power,
            "cut_model": self.cut_model,
            "station_m": self.station_m,
            "maxw_m": self.maxw_m,
            "half_width_m": self.half_width_m,
            "side_slope": self.side_slope,
            "rake_half_m": self.rake_half_m,
            "rake_step_m": self.rake_step_m,
            "ds_step_m": self.ds_step_m,
            "pitch_m": self.pitch_m,
            "n_heading": self.n_heading,
            "radius_multiples": self.radius_multiples,
            "corridor_half_m": self.corridor_half_m,
            "cell_m": self.cell_m,
            "check_chord_m": self.check_chord_m,
        });
        // Written only when it is on: a design that never set it keeps the same settings, and so
        // the same dependency hash, as before the setting existed. On, it is part of the hash, so a
        // change makes the optimise stage stale.
        if self.length_weight != 0.0 {
            v["length_weight"] = json!(self.length_weight);
        }
        if self.land_weight != 0.0 {
            v["land_weight"] = json!(self.land_weight);
        }
        if self.corridor_limit {
            v["corridor_limit"] = json!(true);
        }
        v
    }

    /// Lay the settings of `source` (an object, or null for none) over these. A key the stage does
    /// not know, or a value of the wrong kind, is refused by name.
    fn apply(&self, source: &Value) -> Result<OptimiseSettings, DesignError> {
        let mut out = self.clone();
        let Some(o) = source.as_object() else {
            return Ok(out);
        };
        let f = |k: &str| num(source, k);
        let list = |k: &str| nums(source, k);
        let maybe = |k: &str| match &o[k] {
            Value::Null => Ok(None),
            v => v.as_f64().map(Some).ok_or_else(|| {
                DesignError::Input(format!("the setting `{k}` must be a number or null"))
            }),
        };
        let whole = |k: &str| {
            o[k].as_i64().ok_or_else(|| {
                DesignError::Input(format!("the setting `{k}` must be a whole number"))
            })
        };
        for (k, v) in o {
            match k.as_str() {
                "site_step_m" => out.site_step_m = f(k)?,
                "candidate_sites_m" => {
                    out.candidate_sites_m = match v {
                        Value::Null => None,
                        _ => Some(list(k)?),
                    }
                }
                "lifts_m" => out.lifts_m = list(k)?,
                "terminal_min_level_m" => out.terminal_min_level_m = f(k)?,
                "min_spacing_m" => out.min_spacing_m = f(k)?,
                "base_level_m" => out.base_level_m = maybe(k)?,
                "k_best" => out.k_best = whole(k)?,
                "wet_weight" => out.wet_weight = f(k)?,
                "dry_weight" => out.dry_weight = f(k)?,
                "length_weight" => {
                    let w = f(k)?;
                    if !(w.is_finite() && w >= 0.0) {
                        return Err(DesignError::Input(
                            "the setting `length_weight` must be a number >= 0 (a cost per metre \
                             of design line; 0 is off)"
                                .into(),
                        ));
                    }
                    out.length_weight = w;
                }
                "land_weight" => {
                    let w = f(k)?;
                    if !(w.is_finite() && w >= 0.0) {
                        return Err(DesignError::Input(
                            "the setting `land_weight` must be a number >= 0 (a cost per m2 of \
                             channel footprint outside the river polygon, in m3 of cut per m2; 0 \
                             is off)"
                                .into(),
                        ));
                    }
                    out.land_weight = w;
                }
                "structure_fixed" => out.structure_fixed = f(k)?,
                "structure_per_lift" => out.structure_per_lift = f(k)?,
                "structure_power" => out.structure_power = f(k)?,
                "cut_model" => {
                    out.cut_model = v
                        .as_str()
                        .ok_or_else(|| {
                            DesignError::Input("the setting `cut_model` must be text".into())
                        })?
                        .to_string()
                }
                "station_m" => out.station_m = f(k)?,
                "maxw_m" => out.maxw_m = f(k)?,
                "half_width_m" => out.half_width_m = maybe(k)?,
                "side_slope" => out.side_slope = f(k)?,
                "rake_half_m" => out.rake_half_m = f(k)?,
                "rake_step_m" => out.rake_step_m = f(k)?,
                "ds_step_m" => out.ds_step_m = f(k)?,
                "pitch_m" => out.pitch_m = f(k)?,
                "n_heading" => out.n_heading = whole(k)?,
                "radius_multiples" => out.radius_multiples = list(k)?,
                "corridor_half_m" => out.corridor_half_m = f(k)?,
                "corridor_limit" => {
                    out.corridor_limit = v.as_bool().ok_or_else(|| {
                        DesignError::Input(
                            "the setting `corridor_limit` must be true or false (true holds the \
                             pool search inside the corridor)"
                                .into(),
                        )
                    })?
                }
                "cell_m" => out.cell_m = f(k)?,
                "check_chord_m" => out.check_chord_m = maybe(k)?,
                other => {
                    return Err(DesignError::Input(format!(
                        "the optimise stage has no setting `{other}`"
                    )));
                }
            }
        }
        Ok(out)
    }
}

fn num(v: &Value, key: &str) -> Result<f64, DesignError> {
    v[key]
        .as_f64()
        .ok_or_else(|| DesignError::Input(format!("the setting `{key}` must be a number")))
}

fn nums(v: &Value, key: &str) -> Result<Vec<f64>, DesignError> {
    v[key]
        .as_array()
        .and_then(|a| a.iter().map(Value::as_f64).collect::<Option<Vec<f64>>>())
        .ok_or_else(|| DesignError::Input(format!("the setting `{key}` must be a list of numbers")))
}

fn refuse(m: String) -> DesignError {
    DesignError::Input(m)
}

impl Session {
    /// The optimise settings: the defaults, then the document's, then `overrides`. A key the stage
    /// does not know, in the document or from the caller, is refused by name.
    fn optimise_settings(&self, overrides: &Value) -> Result<OptimiseSettings, DesignError> {
        OptimiseSettings::default()
            .apply(&self.doc["stages"]["optimise"]["settings"])?
            .apply(overrides)
    }

    /// Set optimise settings in the document: the defaults, then what the document holds, then
    /// `settings`, through the same reading as `owd optimise` (its flags, such as `--length-weight`,
    /// are overrides of the run). Nothing runs; a fresh optimise stage whose settings change shows as
    /// stale. The app shell and the web session call this. A key the stage does not know, or a bad
    /// value, is refused by name and changes nothing.
    pub fn set_optimise_settings(&mut self, settings: &Value) -> Result<(), DesignError> {
        let merged = self.optimise_settings(settings)?;
        self.doc["stages"]["optimise"]["settings"] = merged.to_value();
        Ok(())
    }

    /// Run the optimise stage: choose weir sites and pool levels, then refine the design line in
    /// the pools with the lattice (replacing the line an earlier run made). `terrain` is the DEM in the project CRS.
    pub fn run_optimise(
        &mut self,
        overrides: &Value,
        terrain: Option<TerrainSampler>,
    ) -> Result<(), DesignError> {
        // The settings first, before the terrain or any work: an unknown key changes nothing.
        self.optimise_settings(overrides)?;
        match self.stage_status(Stage::Sizing) {
            StageStatus::Stale => {
                return Err(DesignError::StaleUpstream {
                    stage: Stage::Optimise,
                    run_first: Stage::Sizing,
                });
            }
            StageStatus::NotRun => {
                return Err(DesignError::Input(
                    "the sizing stage has not been run: choose a ruleset and a vessel class first"
                        .into(),
                ));
            }
            StageStatus::Fresh => {}
        }
        if self.stage_status(Stage::Alignment) == StageStatus::Stale {
            return Err(DesignError::StaleUpstream {
                stage: Stage::Optimise,
                run_first: Stage::Alignment,
            });
        }
        // A design that names a DEM is optimised on volume, and cannot run while the file is
        // missing. A design with no DEM at all is optimised on plan area (see `PLAN_AREA`): there is
        // no ground to cut, so no cascade, and the line is searched for the least channel footprint
        // outside the river polygon.
        let terrain: Option<TerrainSampler> = match (self.dem_path().map(str::to_string), terrain) {
            (Some(_), Some(t)) => Some(t),
            (Some(dem_path), None) => {
                return Err(DesignError::Input(format!(
                    "the DEM `{dem_path}` is missing: the optimise stage cannot run without it"
                )));
            }
            (None, _) => None,
        };
        let plan_area = terrain.is_none();

        let typed = self.optimise_settings(overrides)?;
        let settings = typed.to_value();
        let size = self.channel_size()?;
        let (depth, r_min) = (size.design_depth_m, size.min_bend_radius_m);
        // The bottom half-width: the sizing's, unless the settings fix one.
        let half_width = match &settings["half_width_m"] {
            Value::Null => size.bottom_width_straight_m / 2.0,
            v => v.as_f64().filter(|h| *h > 0.0).ok_or_else(|| {
                DesignError::Input(
                    "the setting `half_width_m` must be a positive number of metres or null".into(),
                )
            })?,
        };

        let baseline: Vec<Pt> = match super::design_line::alignment_line_of(&self.doc) {
            Some(l) => l.line.0,
            None => self.base_line()?,
        };
        let length = linear::length(&baseline);
        let river = self.river_polygons().ok_or_else(|| {
            DesignError::Input("the optimise stage needs the river polygon: import it first".into())
        })?;

        let (wet, dry) = (num(&settings, "wet_weight")?, num(&settings, "dry_weight")?);
        // On plan area there is no ground: a flat sampler well below the bed, so nothing is cut.
        let no_ground = move |xs: &[f64], _: &[f64]| vec![-(depth + 1000.0); xs.len()];
        let (terrain, base_level, pools, cascade): (TerrainSampler, Option<f64>, Vec<_>, Value) =
            if let Some(terrain) = terrain {
                // The water level below the first weir: a setting, else the document's profile at the start
                // of the baseline (the profile of the inputs; the stage's own pool levels come after).
                let centre = self.river_centreline()?;
                let base_level = match settings["base_level_m"].as_f64() {
                    Some(b) => b,
                    None => {
                        let profile = self
                            .input_water_profile()
                            .ok_or_else(|| self.needs_water_profile(Stage::Optimise))?;
                        self.water_level_along(&profile, &baseline, &centre)(0.0)
                    }
                };

                // The cascade search.
                let sites: Vec<f64> = match &settings["candidate_sites_m"] {
                    Value::Null => {
                        let step = num(&settings, "site_step_m")?;
                        if !(step.is_finite() && step > 0.0) {
                            return Err(DesignError::Input(
                                "the setting `site_step_m` must be a positive number".into(),
                            ));
                        }
                        (1..)
                            .map(|k| k as f64 * step)
                            .take_while(|c| *c < length)
                            .collect()
                    }
                    _ => nums(&settings, "candidate_sites_m")?,
                };
                let mut cfg = CascadeConfig::new(
                    sites,
                    nums(&settings, "lifts_m")?,
                    length,
                    num(&settings, "terminal_min_level_m")?,
                );
                cfg.base_level_m = base_level;
                cfg.min_spacing_m = num(&settings, "min_spacing_m")?;
                cfg.k_best = typed.k_best;
                let cfg = cfg.validated().map_err(refuse)?;
                let table = build_level_table(
                    &baseline,
                    terrain,
                    &river,
                    &cascade_levels(&cfg),
                    depth,
                    &HalfWidth::Fixed(half_width),
                    settings["cut_model"].as_str(),
                    Some(num(&settings, "station_m")?),
                    Some(num(&settings, "maxw_m")?),
                    None,
                    SectionOptions::default(),
                )
                .map_err(refuse)?;
                let (fixed, coeff, power) = (
                    num(&settings, "structure_fixed")?,
                    num(&settings, "structure_per_lift")?,
                    num(&settings, "structure_power")?,
                );
                let reach = earthworks_reach_cost(&table, wet, dry);
                let structure = move |lift: f64| fixed + coeff * libm::pow(lift, power);
                let search = search_cascade(&cfg, &reach, &structure).map_err(refuse)?;
                let Some(best) = search.best() else {
                    return Err(DesignError::Input(
                    "no cascade reaches the terminal level: raise the lifts or the candidate sites"
                        .into(),
                ));
                };
                let cascade = json!({
                    "total_cost": best.total_cost,
                    "reach_cost": best.reach_cost(),
                    "structure_cost": best.structure_cost(),
                    "weirs": best.weirs.iter().map(|w| json!({
                        "site_id": w.site_id, "chainage_m": w.chainage_m,
                        "pool_level_m": w.pool_level_m, "lift_m": w.lift_m,
                        "structure_cost": w.structure_cost,
                    })).collect::<Vec<_>>(),
                });
                (terrain, Some(base_level), best.pools(), cascade)
            } else {
                // No weirs are chosen and no pool level is set: the water stays the inputs'.
                (
                    &no_ground,
                    None,
                    Vec::new(),
                    json!({"total_cost": null, "reach_cost": null, "structure_cost": null, "weirs": []}),
                )
            };
        // What the lattice searches: the cascade's pools, or on plan area the whole line as one
        // stretch (cut into pieces like any long pool) at a water level of 0, which no cost reads.
        let stretches: Vec<(f64, f64, f64)> = if plan_area {
            vec![(0.0, length, 0.0)]
        } else {
            pools.clone()
        };
        // The land weight in force: the setting, or on plan area 1 when the setting is 0 (the cost
        // is then the m2 of footprint outside the river polygon, plus the length cost).
        let land_weight = if plan_area && typed.land_weight == 0.0 {
            1.0
        } else {
            typed.land_weight
        };

        // The pool lattice.
        let mut spec = SectionSpec::new(HalfWidthSpec::Fixed(half_width));
        spec.side_slope_h_per_v = num(&settings, "side_slope")?;
        spec.dry_weight = dry;
        spec.wet_weight = wet;
        spec.rake_half_m = num(&settings, "rake_half_m")?;
        spec.rake_step_m = num(&settings, "rake_step_m")?;
        spec.ds_step_m = num(&settings, "ds_step_m")?;
        // The length cost is the lattice's only. The cascade prices reaches by their cut along the
        // baseline: every cascade covers the whole baseline, so a cost per metre would add the same
        // weight x length to each and change no choice of weirs. It stays out of the cascade, whose
        // numbers are then the same with the weight on or off.
        spec.length_weight = typed.length_weight;
        // The land cost is the lattice's only as well: the cascade prices every candidate reach
        // along the same baseline, whose land take does not depend on the pool level in this
        // measure. Its footprint is the channel's plan width at the water surface: the bottom width
        // plus the side slopes (`side_slope`) up through the design depth.
        spec.land_weight = land_weight;
        spec.land_depth_m = depth;
        let mut lattice = LatticeConfig::new(r_min);
        lattice.pitch_m = num(&settings, "pitch_m")?;
        lattice.n_heading = typed.n_heading;
        lattice.radius_multiples = nums(&settings, "radius_multiples")?;
        lattice.corridor_limit = typed.corridor_limit;
        let lattice = lattice.validated().map_err(refuse)?;
        let out = optimise_pools(
            &baseline,
            terrain,
            &river,
            &stretches,
            depth,
            &spec,
            &lattice,
            None,
            PoolOptions {
                corridor_half_m: num(&settings, "corridor_half_m")?,
                cell_m: num(&settings, "cell_m")?,
            },
            true,
        )
        .map_err(refuse)?;

        let chord = match &settings["check_chord_m"] {
            Value::Null => default_check_chord(r_min),
            v => v.as_f64().ok_or_else(|| {
                DesignError::Input("the setting `check_chord_m` must be a number or null".into())
            })?,
        };
        let measured =
            min_radius_on_chord(&out.line, chord).map_err(|e| DesignError::Input(e.message))?;
        let checks = json!({
            "r_min_m": r_min,
            "radius_ok": radius_ok(measured, r_min),
            "measured_min_radius_m": measured,
            "radius_check_chord_m": chord,
            // A design line must never contain a loop (issue #569): a failed check, as R_min.
            "no_self_crossing": {
                "ok": out.self_crossing_m.is_none(),
                "first_crossing_m": out.self_crossing_m,
            },
        });
        let mut details = json!({
            "n_replaced": out.n_replaced(),
            "pools": out.pools.iter().map(|p| json!({
                "start_m": p.start_m, "end_m": p.end_m, "water_level_m": p.water_level_m,
                "converged": p.result.converged, "cost": p.result.cost,
                "used": p.used,
                "kept_baseline_because": p.kept_baseline_because,
                "baseline_cost": p.baseline_cost,
                "measured_min_radius_m": num_or_null(p.result.measured_min_radius_m),
                "radius_check_chord_m": p.result.radius_check_chord_m,
                "radius_ok": p.result.radius_ok,
                "goal_closed": p.result.goal_closed,
                "expansions": p.result.expansions,
                "hit_cap": p.result.hit_cap,
                "end_moved_from_m": p.end_moved_from_m,
                "start_moved_from_m": p.start_moved_from_m,
                "piece": p.piece.map(|c| json!({
                    "index": c.index, "count": c.count,
                    "pool_start_m": c.pool_start_m, "pool_end_m": c.pool_end_m,
                })),
            })).collect::<Vec<_>>(),
            "joins": out.joins.iter().map(|j| json!({
                "at_m": j.at_m,
                "position_gap_m": num_or_null(j.position_gap_m),
                "heading_gap_deg": num_or_null(j.heading_gap_rad.to_degrees()),
                "measured_min_radius_m": num_or_null(j.measured_min_radius_m),
                "radius_check_chord_m": out.radius_check_chord_m,
                "radius_ok": j.radius_ok,
            })).collect::<Vec<_>>(),
            "self_crossing_m": out.self_crossing_m,
        });
        // With a length weight on, the details say so, and give each pool's line length and its
        // part of the cost, so the cut part (cost - length_cost) can be read apart. Off, nothing is
        // added: the details are the same as before the setting existed.
        if typed.length_weight != 0.0 {
            details["length_weight"] = json!(typed.length_weight);
            if let Some(ps) = details["pools"].as_array_mut() {
                for (d, p) in ps.iter_mut().zip(&out.pools) {
                    let len: f64 = p.result.prim_seq.iter().map(|q| q.2).sum();
                    if p.result.converged {
                        d["line_length_m"] = json!(len);
                        d["length_cost"] = json!(typed.length_weight * len);
                    }
                }
            }
        }
        // With a land weight on, the same: the footprint's width, and each pool's land area and
        // land cost (its line's, and its baseline's for comparison). The cut part of a pool's cost
        // is cost - length_cost - land_cost.
        if land_weight != 0.0 {
            details["land_weight"] = json!(land_weight);
            details["land_footprint_width_m"] =
                json!(2.0 * (half_width + spec.side_slope_h_per_v * depth));
            if let Some(ps) = details["pools"].as_array_mut() {
                for (d, p) in ps.iter_mut().zip(&out.pools) {
                    if let Some(a) = p.result.land_area_m2 {
                        d["land_area_m2"] = json!(a);
                        d["land_cost"] = json!(land_weight * a);
                    }
                    if let Some(a) = p.baseline_land_area_m2 {
                        d["baseline_land_area_m2"] = json!(a);
                    }
                }
            }
        }
        // On plan area the details say so in plain words, and no pool carries a water level: the
        // level the search ran at is a placeholder that no cost read.
        if plan_area {
            details["optimised_on"] = json!(PLAN_AREA);
            details["optimised_on_note"] = json!(PLAN_AREA_NOTE);
            details["land_weight_defaulted"] = json!(typed.land_weight == 0.0);
            if let Some(ps) = details["pools"].as_array_mut() {
                for d in ps.iter_mut() {
                    d["water_level_m"] = Value::Null;
                }
            }
        }
        // With the corridor limit on, the details say so, with the half-width the line was held to.
        if typed.corridor_limit {
            details["corridor_limit"] = json!(true);
            details["corridor_half_m"] = json!(typed.corridor_half_m);
        }
        let river_geometry = self
            .river_geometry()
            .map_err(super::rule_sets::stage_error)?;
        let mut refined = super::design_line::finish(
            LATTICE,
            "Pool lattice",
            LATTICE,
            settings.clone(),
            out.line.clone(),
            checks,
            details,
            &river_geometry,
        );
        // The lattice does not steer by the constraints yet (spec 0017 ticket 16, line refinement):
        // its line reports what its footprint enters and fails `no_go_clear` when it enters a no-go
        // area or hard-exclusion ground, as every design line does (ticket 13).
        if let Some(c) = self.constraints()?.filter(|c| !c.is_empty()) {
            let raster = match self.stage_multiplier() {
                Ok(r) => r,
                Err(path) => {
                    return Err(refuse(format!(
                        "OPTIMISE_NO_MULTIPLIER: the cut-rate multiplier raster `{path}` is not \
                         there to read: put it beside the design, or set it again"
                    )));
                }
            };
            let epsg = self.multiplier_epsg();
            let weight = match &raster {
                Some(r) => Some((
                    super::constraints::multiplier_sampler(r, epsg)
                        .map_err(super::rule_sets::stage_error)?,
                    super::constraints::multiplier_cell_m(r, epsg)
                        .map_err(super::rule_sets::stage_error)?,
                )),
                None => None,
            };
            let m = weight
                .as_ref()
                .map(|(w, cell_m)| super::constraints::Multiplier {
                    at: w as TerrainSampler,
                    cell_m: *cell_m,
                });
            super::design_line::mark_constraints(
                &mut refined,
                &self.channel_size()?,
                &c,
                m.as_ref(),
            );
        }
        // The design line's own sanity check, one warning for each pool (or piece) whose search
        // hit its limit (issue #559), and one for each pool join that does not hold R_min (issue #6).
        if let Some(w) = refined.sanity["warnings"].as_array_mut() {
            if plan_area {
                w.push(json!(PLAN_AREA_NOTE));
            }
            for p in out.capped_pools() {
                w.push(json!(capped_warning(p, lattice.max_expand)));
            }
            for j in out.failed_joins() {
                w.push(json!(join_warning(j, r_min, out.radius_check_chord_m)));
            }
            // A pool that could not close on its end pose (issue #565): kept as the baseline, or
            // closed on a moved search end. A loop in the line is already a warning of the line's
            // own sanity check, with its chainage.
            for p in out.unclosed_pools() {
                w.push(json!(unclosed_warning(p)));
            }
            for p in out.moved_boundaries() {
                w.push(json!(moved_warning(p)));
            }
            // A pool with no line inside the corridor keeps its baseline (`corridor_limit`).
            for p in out.no_line_in_corridor_pools() {
                w.push(json!(corridor_warning(p, typed.corridor_half_m)));
            }
            // A pool whose line crossed itself or the rest of the line keeps its baseline (#569).
            for p in out.looped_pools() {
                w.push(json!(looped_warning(p)));
            }
        }

        let profile_points = self.pool_profile(&baseline, &pools)?;
        let st = &mut self.doc["stages"]["optimise"];
        st["settings"] = settings;
        st["results"] = json!({
            "baseline": {"type": "LineString", "coordinates": baseline},
            "base_level_m": base_level,
            "profile": profile_points,
            "pools": pools.iter().map(|&(a, b, l)| json!([a, b, l])).collect::<Vec<_>>(),
            "cascade": cascade,
            "design_line": refined,
        });
        // Written on plan area only, so a run on volume keeps the bytes it had.
        if plan_area {
            st["results"]["optimised_on"] = json!(PLAN_AREA);
        }
        st["engine_version"] = json!(ENGINE_VERSION);
        let h = self.dependency_hash(Stage::Optimise);
        self.doc["stages"]["optimise"]["depends_on"] = json!(h);
        Ok(())
    }
}

/// The sanity warning for a pool join that does not hold R_min.
fn join_warning(j: &owd_core::optimise::lattice::PoolJoin, r_min: f64, chord: f64) -> String {
    let mut w = format!(
        "the pool lattice's join at {:.0} m reads a radius of {:.0} m on a {:.0} m chord against R_min {:.0} m",
        j.at_m, j.measured_min_radius_m, chord, r_min
    );
    if j.position_gap_m > 1e-6 || j.heading_gap_rad > 1e-6 {
        w.push_str(&format!(
            " (the pool's line ends {:.1} m and {:.1}° from where the next piece starts)",
            j.position_gap_m,
            j.heading_gap_rad.to_degrees()
        ));
    }
    w
}

/// The sanity warning for a pool (or a piece of one) whose search hit its expansion limit, so it
/// keeps the baseline unsearched.
fn capped_warning(p: &owd_core::optimise::lattice::PoolAlignment, max_expand: i64) -> String {
    let what = match p.piece {
        Some(c) => format!(
            "piece {} of {} of the pool from {:.0} m to {:.0} m ({:.0} m to {:.0} m)",
            c.index + 1,
            c.count,
            c.pool_start_m,
            c.pool_end_m,
            p.start_m,
            p.end_m
        ),
        None => format!("the pool from {:.0} m to {:.0} m", p.start_m, p.end_m),
    };
    format!(
        "{what} was not searched: the search hit its limit of {max_expand} expansions, so it keeps \
         the baseline"
    )
}

/// How a pool (or a piece of one) is named in a warning.
fn pool_name(p: &owd_core::optimise::lattice::PoolAlignment) -> String {
    let start = p.start_moved_from_m.unwrap_or(p.start_m);
    let end = p.end_moved_from_m.unwrap_or(p.end_m);
    match p.piece {
        Some(c) => format!(
            "piece {} of {} of the pool from {:.0} m to {:.0} m ({start:.0} m to {end:.0} m)",
            c.index + 1,
            c.count,
            c.pool_start_m,
            c.pool_end_m,
        ),
        None => format!("the pool from {start:.0} m to {end:.0} m"),
    }
}

/// The sanity warning for a pool whose search could not close on its end pose at any end tried, and
/// whose line would have broken its join, so it keeps the baseline (issue #565).
fn unclosed_warning(p: &owd_core::optimise::lattice::PoolAlignment) -> String {
    let inside = if p.kept_baseline_because
        == Some(owd_core::optimise::lattice::UNCLOSED_IN_CORRIDOR_REASON)
    {
        " inside the corridor"
    } else {
        ""
    };
    format!(
        "{} keeps the baseline: its search could not end on the pool's end pose{inside}, even with \
         the end moved, and its line would have joined the next piece {:.1} m and {:.1}° off, \
         breaking R_min or crossing itself",
        pool_name(p),
        p.result.end_err_m,
        p.result.end_heading_err_rad.to_degrees()
    )
}

/// The sanity warning for a pool whose search, held to the corridor (`corridor_limit`), found no
/// line inside it, so it keeps the baseline.
fn corridor_warning(p: &owd_core::optimise::lattice::PoolAlignment, half_m: f64) -> String {
    format!(
        "{} keeps the baseline: the search found no line that stays within {half_m:.0} m of the \
         baseline (`corridor_limit` is on). Widen `corridor_half_m`, or turn the limit off",
        pool_name(p)
    )
}

/// The sanity warning for a pool whose line crossed itself (at every search end tried) or crossed
/// the rest of the design line, so it keeps the baseline (issue #569).
fn looped_warning(p: &owd_core::optimise::lattice::PoolAlignment) -> String {
    format!(
        "{} keeps the baseline: {}",
        pool_name(p),
        p.kept_baseline_because.unwrap_or("its line makes a loop")
    )
}

/// The sanity warning for a pool whose search closed only on an end moved along the baseline.
fn moved_warning(p: &owd_core::optimise::lattice::PoolAlignment) -> String {
    let from = p.end_moved_from_m.unwrap_or(p.end_m);
    let (by, way) = if p.end_m >= from {
        (p.end_m - from, "downstream")
    } else {
        (from - p.end_m, "upstream")
    };
    format!(
        "{}: the search could not end on the pool's end pose with a line that does not cross \
         itself, so its search end moved {by:.0} m {way}, to {:.0} m, where it does; the pool's \
         water level and weir are unchanged, so check the line between {:.0} m and {:.0} m",
        pool_name(p),
        p.end_m,
        from.min(p.end_m),
        from.max(p.end_m)
    )
}

/// A finite number, or null (JSON has no NaN).
fn num_or_null(v: f64) -> Value {
    if v.is_finite() { json!(v) } else { Value::Null }
}

#[cfg(test)]
mod tests {
    //! Label: property (a join's warning names where it is, what it reads, and the gap).
    use super::*;
    use owd_core::optimise::lattice::PoolJoin;

    #[test]
    fn a_join_warning_names_the_join_its_radius_and_its_gap() {
        let mut j = PoolJoin {
            at_m: 3937.4,
            position_gap_m: 4.04,
            heading_gap_rad: 27f64.to_radians(),
            measured_min_radius_m: 158.2,
            radius_ok: false,
        };
        assert_eq!(
            join_warning(&j, 340.0, 340.0 / 3.0),
            "the pool lattice's join at 3937 m reads a radius of 158 m on a 113 m chord against \
             R_min 340 m (the pool's line ends 4.0 m and 27.0° from where the next piece starts)"
        );
        j.position_gap_m = 0.0;
        j.heading_gap_rad = 0.0;
        assert!(join_warning(&j, 340.0, 113.0).ends_with("against R_min 340 m"));
    }

    /// Label: property (a capped pool's warning names the pool, or the piece and its pool).
    #[test]
    fn a_capped_warning_names_the_pool_or_the_piece() {
        use owd_core::optimise::lattice::{
            HalfWidthSpec, LatticeConfig, PoolAlignment, PoolPiece, SectionSpec,
        };
        let flat = |xs: &[f64], _: &[f64]| vec![0.0; xs.len()];
        let line = [[0.0, 0.0], [100.0, 0.0]];
        let poly = owd_core::geom::Polygon {
            exterior: vec![
                [0.0, -5.0],
                [100.0, -5.0],
                [100.0, 5.0],
                [0.0, 5.0],
                [0.0, -5.0],
            ],
            holes: vec![],
        };
        let mut cfg = LatticeConfig::new(50.0);
        cfg.max_expand = 1;
        let cfg = cfg.validated().unwrap();
        let mut p: PoolAlignment = owd_core::optimise::lattice::optimise_pool(
            &line,
            &flat,
            core::slice::from_ref(&poly),
            0.0,
            100.0,
            0.0,
            1.0,
            &SectionSpec::new(HalfWidthSpec::Fixed(2.0)),
            &cfg,
            None,
            owd_core::optimise::lattice::PoolOptions {
                corridor_half_m: 20.0,
                cell_m: 5.0,
            },
        )
        .unwrap();
        assert_eq!(
            capped_warning(&p, 4_000_000),
            "the pool from 0 m to 100 m was not searched: the search hit its limit of 4000000 \
             expansions, so it keeps the baseline"
        );
        p.piece = Some(PoolPiece {
            index: 1,
            count: 3,
            pool_start_m: 0.0,
            pool_end_m: 300.0,
        });
        assert!(
            capped_warning(&p, 9)
                .starts_with("piece 2 of 3 of the pool from 0 m to 300 m (0 m to 100 m) was not")
        );

        // Label: property (a moved or unclosed pool's warning names the pool and the move).
        p.piece = None;
        p.end_m = 80.0;
        p.end_moved_from_m = Some(100.0);
        assert_eq!(
            moved_warning(&p),
            "the pool from 0 m to 100 m: the search could not end on the pool's end pose with a \
             line that does not cross itself, so its search end moved 20 m upstream, to 80 m, where \
             it does; the pool's water level and weir are unchanged, so check the line between 80 m \
             and 100 m"
        );
        p.end_m = 100.0;
        p.end_moved_from_m = None;
        p.result.end_err_m = 7.34;
        p.result.end_heading_err_rad = 14f64.to_radians();
        assert_eq!(
            unclosed_warning(&p),
            "the pool from 0 m to 100 m keeps the baseline: its search could not end on the pool's \
             end pose, even with the end moved, and its line would have joined the next piece \
             7.3 m and 14.0° off, breaking R_min or crossing itself"
        );
    }
}
