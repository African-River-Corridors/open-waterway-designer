//! The optimise stage (spec 0002): the weir cascade search sets the weir sites and pool levels, and
//! the pool lattice adds a design line option. The calls are those of the `optimise_pools` example:
//! `build_level_table`, `search_cascade` with an earthworks reach cost, then `optimise_pools`.
//!
//! The baseline is the rolling-circle option's line, else the river centreline. Chainages are
//! metres along it. Weights are settings, not prices: 1.0 / 1.0 minimises volume.

use super::alignment::Terrain;
use super::geojson::all_positions;
use super::session::{DesignError, ENGINE_VERSION, Session, Stage, StageStatus};
use owd_core::alignment::radius_check::{default_check_chord, min_radius_on_chord, radius_ok};
use owd_core::cut_model::CUT_MODEL;
use owd_core::geom::{Polygon, Pt, linear};
use owd_core::optimise::cascade::{
    CascadeConfig, SectionOptions, build_level_table, cascade_levels, earthworks_reach_cost,
    search_cascade,
};
use owd_core::optimise::lattice::{
    HalfWidthSpec, LatticeConfig, PoolOptions, SectionSpec, optimise_pools,
};
use owd_core::volumes::sections::HalfWidth;
use serde_json::{Value, json};

const OPTION_ID: &str = "lattice";

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
            cell_m: 2.0,
            check_chord_m: None,
        }
    }
}

impl OptimiseSettings {
    /// The settings as the stage writes them into the document.
    pub fn to_value(&self) -> Value {
        json!({
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
        })
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

    /// Run the optimise stage: choose weir sites and pool levels, then add the lattice design line
    /// option (replacing the one an earlier run made). `terrain` is the DEM in the project CRS.
    pub fn run_optimise(
        &mut self,
        overrides: &Value,
        terrain: Option<Terrain>,
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
        let Some(dem_path) = self.dem_path().map(str::to_string) else {
            return Err(DesignError::Input(
                "the optimise stage needs a DEM: import one first (`owd import --dem FILE`)".into(),
            ));
        };
        let terrain = terrain.ok_or_else(|| {
            DesignError::Input(format!(
                "the DEM `{dem_path}` is missing: the optimise stage cannot run without it"
            ))
        })?;

        let typed = self.optimise_settings(overrides)?;
        let settings = typed.to_value();
        let shape = self.sized_shape()?;
        let (depth, r_min) = (shape.design_depth_m, shape.min_bend_radius_m);
        // The bottom half-width: the sizing's, unless the settings fix one.
        let half_width = match &settings["half_width_m"] {
            Value::Null => shape.bottom_width_straight_m / 2.0,
            v => v.as_f64().filter(|h| *h > 0.0).ok_or_else(|| {
                DesignError::Input(
                    "the setting `half_width_m` must be a positive number of metres or null".into(),
                )
            })?,
        };

        let baseline: Vec<Pt> = self.doc["options"]
            .as_array()
            .and_then(|o| o.iter().find(|o| o["method"] == "rolling_circle"))
            .map(|o| all_positions(&o["line"]))
            .map_or_else(|| self.base_line(), Ok)?;
        let length = linear::length(&baseline);
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

        let polygon = &self.doc["inputs"]["river_polygon"];
        if polygon.is_null() {
            return Err(DesignError::Input(
                "the optimise stage needs the river polygon: import it first".into(),
            ));
        }
        let ring: Vec<Pt> = polygon["coordinates"][0]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|p| Some([p[0].as_f64()?, p[1].as_f64()?]))
                    .collect()
            })
            .unwrap_or_default();
        let river = [Polygon {
            exterior: ring,
            holes: vec![],
        }];

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
        let (wet, dry) = (num(&settings, "wet_weight")?, num(&settings, "dry_weight")?);
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
        let pools = best.pools();

        // The pool lattice.
        let mut spec = SectionSpec::new(HalfWidthSpec::Fixed(half_width));
        spec.side_slope_h_per_v = num(&settings, "side_slope")?;
        spec.dry_weight = dry;
        spec.wet_weight = wet;
        spec.rake_half_m = num(&settings, "rake_half_m")?;
        spec.rake_step_m = num(&settings, "rake_step_m")?;
        spec.ds_step_m = num(&settings, "ds_step_m")?;
        let mut lattice = LatticeConfig::new(r_min);
        lattice.pitch_m = num(&settings, "pitch_m")?;
        lattice.n_heading = typed.n_heading;
        lattice.radius_multiples = nums(&settings, "radius_multiples")?;
        let lattice = lattice.validated().map_err(refuse)?;
        let out = optimise_pools(
            &baseline,
            terrain,
            &river,
            &pools,
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
        let option = json!({
            "id": OPTION_ID, "name": "Pool lattice",
            "method": "lattice", "settings": settings,
            "line": {"type": "LineString", "coordinates": out.line},
            "checks": {
                "r_min_m": r_min,
                "radius_ok": radius_ok(measured, r_min),
                "measured_min_radius_m": measured,
                "radius_check_chord_m": chord,
            },
            "details": {
                "n_replaced": out.n_replaced(),
                "pools": out.pools.iter().map(|p| json!({
                    "start_m": p.start_m, "end_m": p.end_m, "water_level_m": p.water_level_m,
                    "converged": p.result.converged, "cost": p.result.cost,
                    "baseline_cost": p.baseline_cost,
                    "measured_min_radius_m": num_or_null(p.result.measured_min_radius_m),
                    "radius_check_chord_m": p.result.radius_check_chord_m,
                    "radius_ok": p.result.radius_ok,
                })).collect::<Vec<_>>(),
            },
        });
        let options = self.doc["options"]
            .as_array_mut()
            .expect("options is a list");
        match options.iter().position(|o| o["method"] == "lattice") {
            Some(i) => options[i] = option,
            None => options.push(option),
        }

        let profile_points = self.pool_profile(&baseline, &pools)?;
        let st = &mut self.doc["stages"]["optimise"];
        st["settings"] = settings;
        st["results"] = json!({
            "baseline": {"type": "LineString", "coordinates": baseline},
            "base_level_m": base_level,
            "profile": profile_points,
            "pools": pools.iter().map(|&(a, b, l)| json!([a, b, l])).collect::<Vec<_>>(),
            "cascade": {
                "total_cost": best.total_cost,
                "reach_cost": best.reach_cost(),
                "structure_cost": best.structure_cost(),
                "weirs": best.weirs.iter().map(|w| json!({
                    "site_id": w.site_id, "chainage_m": w.chainage_m,
                    "pool_level_m": w.pool_level_m, "lift_m": w.lift_m,
                    "structure_cost": w.structure_cost,
                })).collect::<Vec<_>>(),
            },
            "option": OPTION_ID,
        });
        st["engine_version"] = json!(ENGINE_VERSION);
        let h = self.dependency_hash(Stage::Optimise);
        self.doc["stages"]["optimise"]["depends_on"] = json!(h);
        Ok(())
    }
}

/// A finite number, or null (JSON has no NaN).
fn num_or_null(v: f64) -> Value {
    if v.is_finite() { json!(v) } else { Value::Null }
}
