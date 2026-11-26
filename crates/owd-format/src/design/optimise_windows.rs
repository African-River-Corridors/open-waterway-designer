//! The optimise stage's window method (OPT-5): `method: "windows"` runs the window optimiser on
//! the alignment's design line (or a given line), instead of the weir cascade and the pool lattice.
//!
//! It shares the stage with the pool lattice. The settings name the method (`method`, written only
//! when it is not `lattice`) and hold the window optimiser's own settings under `windows`, written
//! only with the window method, so a design that never chose it reads, hashes and saves as before.
//! The objective's weights (`wet_weight`, `dry_weight`, `land_weight`, `length_weight`) are the
//! stage's own, shared by both methods. The section is the volumes stage's (its cut model, station
//! step, reach, sampling, slope and half-width), so the search prices what the volumes report counts.
//!
//! The window method does not run the cascade. The water levels are those the volumes stage reads:
//! the pool levels of an earlier, fresh run of the pool lattice when there is one (its weir sites are
//! then forced seams, and its pools, profile and cascade are carried into this run's results
//! unchanged), else the design's water level. Its line is stored as `results.design_line` with the
//! method `windows`, as the lattice's line is, and its records as `results.windows`.

use super::design_line::{self, DesignLine};
use super::optimise::{OptimiseSettings, PLAN_AREA, PLAN_AREA_NOTE};
use super::session::{DesignError, ENGINE_VERSION, Session, Stage, StageStatus};
use crate::types::TerrainSampler;
use owd_core::alignment::radius_check::default_check_chord;
use owd_core::cut_model::WIDEN_RMAX;
use owd_core::geom::overlay::PolygonIndex;
use owd_core::geom::{Pt, linear};
use owd_core::optimise::lattice::LatticeConfig;
use owd_core::optimise::window::polish::PolishSettings;
use owd_core::optimise::window::run::{
    Adopted, WindowConfig, WindowInputs, WindowRecord, WindowsResult, geometric_radii,
    optimise_windows,
};
use owd_core::optimise::window::scorer::{HalfWidthRule, Measure, SectionSettings, Weights};
use owd_core::optimise::window::seams::Seam;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The method name the stage writes for the window optimiser.
pub const WINDOWS: &str = "windows";

/// The window optimiser's settings (OPT-5 3.3). A setting left `null` takes its default from the
/// sizing (R_min, the vessel's length, the bottom width), as the spec gives it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct WindowSettings {
    /// The preferred window length (m).
    pub window_target_m: f64,
    /// The shortest window between free seams (m).
    pub window_min_m: f64,
    /// The longest window (m).
    pub window_max_m: f64,
    /// Seam candidates every this many metres of the line.
    pub seam_step_m: f64,
    /// A free seam stands where the line is within this of the river centreline (m); null: a
    /// quarter of the bottom width, at least 10 m.
    pub seam_max_offset_m: Option<f64>,
    /// The chords the line's radius is read on at a seam (m); null: R_min / 6, / 3, x 2/3, x 4/3.
    pub seam_scales_m: Option<Vec<f64>>,
    /// Search windows over the first pass's seams as well.
    pub second_pass: bool,
    pub pitch_m: f64,
    pub n_heading: i64,
    /// `geometric`, `multiples` or `list`.
    pub radii_mode: String,
    pub n_radii: i64,
    pub radius_max_factor: f64,
    pub radius_multiples: Vec<f64>,
    /// The radii of `list` mode (m); R_min is added and any under R_min dropped.
    pub radii_m: Option<Vec<f64>>,
    pub nominal_arc_m: f64,
    /// null: two pitches.
    pub straight_len_m: Option<f64>,
    /// The straight between opposite curves (m); null: one vessel length.
    pub min_straight_m: Option<f64>,
    pub corridor_half_m: f64,
    pub corridor_limit: bool,
    pub cell_m: f64,
    /// `auto`, `on` or `off`: the line may not leave the river except on its recorded cut-throughs.
    pub stay_in_river: String,
    pub in_river_margin_m: f64,
    /// null: two bottom widths.
    pub cut_through_room_m: Option<f64>,
    pub heuristic: String,
    pub max_expand: i64,
    pub max_states: i64,
    pub window_time_s: f64,
    pub polish: bool,
    /// null: R_min / 3.6, / 3, / 2.4, / 1.8 and / 1.44.
    pub polish_spacings_m: Option<Vec<f64>>,
    /// null: half the bottom width, at least 15 m.
    pub polish_max_shift_m: Option<f64>,
    pub polish_station_m: f64,
    /// null: the larger of 20 m and 6 % of R_min.
    pub polish_radius_margin_m: Option<f64>,
    pub adopt_threshold: f64,
    pub turn_limit_deg: f64,
    pub length_ratio_limit: f64,
    pub search_rake_step_m: f64,
    pub search_ds_m: f64,
    /// Windows searched at once; null: every core (always 1 in the browser).
    pub threads: Option<i64>,
    /// The memory the windows in flight may hold at once (MB).
    pub memory_budget_mb: f64,
}

impl Default for WindowSettings {
    fn default() -> Self {
        WindowSettings {
            window_target_m: 1500.0,
            window_min_m: 800.0,
            window_max_m: 2500.0,
            seam_step_m: 20.0,
            seam_max_offset_m: None,
            seam_scales_m: None,
            second_pass: true,
            pitch_m: 12.0,
            n_heading: 64,
            radii_mode: "geometric".into(),
            n_radii: 12,
            radius_max_factor: 8.0,
            radius_multiples: vec![1.0, 1.5, 3.0],
            radii_m: None,
            nominal_arc_m: 80.0,
            straight_len_m: None,
            min_straight_m: None,
            corridor_half_m: 350.0,
            corridor_limit: true,
            cell_m: 2.0,
            stay_in_river: "auto".into(),
            in_river_margin_m: 0.0,
            cut_through_room_m: None,
            heuristic: "c_min".into(),
            max_expand: 2_000_000,
            max_states: 2_000_000,
            window_time_s: 300.0,
            polish: true,
            polish_spacings_m: None,
            polish_max_shift_m: None,
            polish_station_m: 20.0,
            polish_radius_margin_m: None,
            adopt_threshold: 0.01,
            turn_limit_deg: 1080.0,
            length_ratio_limit: 3.0,
            search_rake_step_m: 2.0,
            search_ds_m: 8.0,
            threads: None,
            memory_budget_mb: 8192.0,
        }
    }
}

fn bad(m: String) -> DesignError {
    DesignError::Input(m)
}

impl WindowSettings {
    /// The settings as the stage writes them.
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    /// Lay the keys of `source` (an object; null for none) over these. An unknown key or a value of
    /// the wrong kind is refused by name.
    pub fn apply(&self, source: &Value) -> Result<WindowSettings, DesignError> {
        let Some(o) = source.as_object() else {
            return Ok(self.clone());
        };
        let mut merged = self.to_value();
        for (k, v) in o {
            if merged.get(k).is_none() {
                return Err(bad(format!(
                    "the window optimiser has no setting `{k}` (the settings under `windows`)"
                )));
            }
            merged[k] = v.clone();
        }
        let out: WindowSettings = serde_json::from_value(merged).map_err(|e| {
            bad(format!(
                "a setting of the window optimiser (under `windows`) is of the wrong kind: {e}"
            ))
        })?;
        out.check()?;
        Ok(out)
    }

    /// Refuse a value no window optimiser can run with, in plain words.
    pub fn check(&self) -> Result<(), DesignError> {
        let pos = |k: &str, v: f64| {
            if v.is_finite() && v > 0.0 {
                Ok(())
            } else {
                Err(bad(format!(
                    "the window setting `{k}` must be a number above 0"
                )))
            }
        };
        pos("window_target_m", self.window_target_m)?;
        pos("window_min_m", self.window_min_m)?;
        pos("window_max_m", self.window_max_m)?;
        pos("seam_step_m", self.seam_step_m)?;
        pos("pitch_m", self.pitch_m)?;
        pos("nominal_arc_m", self.nominal_arc_m)?;
        pos("corridor_half_m", self.corridor_half_m)?;
        pos("cell_m", self.cell_m)?;
        pos("window_time_s", self.window_time_s)?;
        pos("polish_station_m", self.polish_station_m)?;
        pos("search_rake_step_m", self.search_rake_step_m)?;
        pos("search_ds_m", self.search_ds_m)?;
        pos("memory_budget_mb", self.memory_budget_mb)?;
        pos("radius_max_factor", self.radius_max_factor)?;
        if !(self.window_min_m <= self.window_target_m && self.window_target_m <= self.window_max_m)
        {
            return Err(bad(format!(
                "the windows must run from `window_min_m` up to `window_max_m` with \
                 `window_target_m` between: {} <= {} <= {} does not hold",
                self.window_min_m, self.window_target_m, self.window_max_m
            )));
        }
        if !(self.adopt_threshold >= 0.0 && self.adopt_threshold < 1.0) {
            return Err(bad(
                "the window setting `adopt_threshold` must be at least 0 and below 1 (0.01 is 1 %)"
                    .into(),
            ));
        }
        if !(4..=1024).contains(&self.n_heading) {
            return Err(bad(
                "the window setting `n_heading` must be a whole number from 4 to 1024".into(),
            ));
        }
        if !matches!(self.radii_mode.as_str(), "geometric" | "multiples" | "list") {
            return Err(bad(
                "the window setting `radii_mode` must be `geometric`, `multiples` or `list`".into(),
            ));
        }
        if self.radii_mode == "list" && self.radii_m.as_ref().is_none_or(Vec::is_empty) {
            return Err(bad(
                "with `radii_mode` `list` the window setting `radii_m` must list the radii".into(),
            ));
        }
        if self.n_radii < 1 {
            return Err(bad("the window setting `n_radii` must be at least 1".into()));
        }
        if !matches!(self.stay_in_river.as_str(), "auto" | "on" | "off") {
            return Err(bad(
                "the window setting `stay_in_river` must be `auto`, `on` or `off`".into(),
            ));
        }
        if !matches!(self.heuristic.as_str(), "c_min" | "none") {
            return Err(bad(
                "the window setting `heuristic` must be `c_min` or `none`".into(),
            ));
        }
        if self.max_expand < 1 || self.max_states < 1 {
            return Err(bad(
                "the window settings `max_expand` and `max_states` must be at least 1".into(),
            ));
        }
        if self.threads.is_some_and(|t| t < 1) {
            return Err(bad(
                "the window setting `threads` must be at least 1, or null for every core".into(),
            ));
        }
        Ok(())
    }
}

/// The start-line chainage of a record's stretches (OPT-4's cut-throughs and forced stretches),
/// read from the line's details: `[{from_m, to_m}]`.
fn stretches(details: &Value, key: &str) -> Vec<(f64, f64)> {
    details[key]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| Some((s["from_m"].as_f64()?, s["to_m"].as_f64()?)))
        .filter(|(a, b)| a < b)
        .collect()
}

/// The river-following record's permitted stretches (OPT-4's `permitted_stretches()`: `{id, kind:
/// cut_through|forced, from, to, room_m}`, design-line chainage), read from the line's details, at
/// `permitted_stretches` or `record.permitted_stretches`: `(kind, from, to, room)`.
fn permitted_records(details: &Value) -> Vec<(String, f64, f64, Option<f64>)> {
    let list = if details["permitted_stretches"].is_array() {
        &details["permitted_stretches"]
    } else {
        &details["record"]["permitted_stretches"]
    };
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let from = s["from"].as_f64().or_else(|| s["from_m"].as_f64())?;
            let to = s["to"].as_f64().or_else(|| s["to_m"].as_f64())?;
            (from < to).then(|| {
                (
                    s["kind"].as_str().unwrap_or("cut_through").to_string(),
                    from,
                    to,
                    s["room_m"].as_f64(),
                )
            })
        })
        .collect()
}

/// Where the water level along the starting line steps (a weir between two pools): every place
/// the level table (one value every 2 m) changes by more than 5 cm between neighbours.
fn level_steps(table: &[f64]) -> Vec<f64> {
    table
        .windows(2)
        .enumerate()
        .filter(|(_, w)| (w[1] - w[0]).abs() > 0.05)
        .map(|(i, _)| 2.0 * i as f64 + 1.0)
        .collect()
}

fn num(v: f64) -> Value {
    if v.is_finite() { json!(v) } else { Value::Null }
}

fn measure_json(m: &Measure, plan_area: bool) -> Value {
    if plan_area {
        return json!({
            "cost": num(m.cost), "land_m2": num(m.land_m2), "length_m": num(m.length_m),
        });
    }
    json!({
        "cost": num(m.cost), "dredge_m3": num(m.dredge_m3), "excav_m3": num(m.excav_m3),
        "total_m3": num(m.total_m3()), "land_m2": num(m.land_m2), "length_m": num(m.length_m),
        "max_face_h_m": num(m.max_face_h_m), "unknown_ground_pct": num(100.0 * m.unknown_share),
    })
}

fn seam_json(s: &Seam) -> Value {
    json!({
        "at_m": s.at_m, "kind": s.kind.name(),
        "quality_radius_m": num(s.quality_m), "offset_m": num(s.offset_m),
    })
}

/// One window's record, as the details give it (OPT-5 3.12).
fn record_json(w: &WindowRecord, plan_area: bool, hash: &str) -> Value {
    let polish = |p: &Option<owd_core::optimise::window::run::PolishRecord>| {
        p.as_ref().map(|p| {
            json!({
                "accepted": p.accepted, "spacing_m": num(p.spacing_m),
                "cost_by_spacing": p.by_spacing.iter().map(|(s, c)| json!([s, num(*c)])).collect::<Vec<_>>(),
                "spread_pct": num(p.spread_pct), "max_shift_m": num(p.max_shift_m),
                "min_radius_before_m": num(p.min_radius_before_m),
                "min_radius_after_m": num(p.min_radius_after_m),
            })
        })
    };
    json!({
        "id": w.id, "pass": w.pass,
        "start_m": w.start_m, "end_m": w.end_m,
        "start_seam": seam_json(&w.start_seam), "end_seam": seam_json(&w.end_seam),
        "search_cost_start": num(w.search_cost_start),
        "search_cost_found": w.search_cost_found.map(num),
        "full_start": measure_json(&w.full_start, plan_area),
        "full_start_polished": w.full_start_polished.as_ref().map(|m| measure_json(m, plan_area)),
        "full_found": w.full_found.as_ref().map(|m| measure_json(m, plan_area)),
        "gain_pct": w.gain_pct.map(num),
        "adopted": w.adopted.name(),
        "reason": w.reason,
        "expansions": w.expansions, "states": w.states, "seconds": num(w.seconds),
        "polish_found": polish(&w.polish_found),
        "polish_start": polish(&w.polish_start),
        "loop": w.loop_check.map(|l| json!({
            "self_crossing_m": l.self_crossing_m, "cumulative_turn_deg": num(l.turn_deg),
            "length_ratio": num(l.length_ratio),
        })),
        "min_radius_m": num(w.min_radius_m),
        "unknown_ground_pct": num(w.unknown_ground_pct),
        "short_straights": w.short_straights,
        "seam_gap_m": num(w.seam_gap_m),
        "seam_heading_gap_rad": num(w.seam_heading_gap_rad),
        "cut_through_dropped": w.cut_through_dropped,
        "weir_not_on_straight": w.weir_not_on_straight,
        "settings_hash": hash,
    })
}

impl Session {
    /// The window method's run of the optimise stage: see the module docs. `typed` are the stage's
    /// settings with `method` `windows`; `terrain` the DEM (with no DEM, plan area).
    #[allow(clippy::too_many_lines)]
    pub(super) fn run_optimise_windows(
        &mut self,
        typed: &OptimiseSettings,
        terrain: Option<TerrainSampler>,
    ) -> Result<(), DesignError> {
        let ws = &typed.windows;
        let plan_area = terrain.is_none();
        let _ = owd_core::types::grid::take_grid_read();
        let size = self.channel_size()?;
        let (depth, r_min, loa, bw) = (
            size.design_depth_m,
            size.min_bend_radius_m,
            size.loa_m,
            size.bottom_width_straight_m,
        );
        // The section is the volumes stage's.
        let vol = self.volumes_settings(&Value::Null)?;
        let model = owd_core::cut_model::normalise_model(&vol.cut_model);
        if model != owd_core::cut_model::CUT_MODEL && model != owd_core::cut_model::CUT_MODEL_B {
            return Err(bad(format!(
                "the volumes stage's cut model `{}` is not one OWD knows: use `a` or `b`",
                vol.cut_model
            )));
        }
        let rule = match vol.half_width_m {
            Some(h) => HalfWidthRule::fixed(h),
            None => {
                let shape = size.shape().map_err(super::rule_sets::stage_error)?.clone();
                let floor = shape.min_bend_radius_m;
                HalfWidthRule::new(bw / 2.0, WIDEN_RMAX, move |r: f64| {
                    shape.bend_widening_m(r.max(floor))
                })
            }
        };
        // The starting line: the alignment's (a given line included), else the river centreline.
        let start = design_line::alignment_line_of(&self.doc);
        let (line, start_details): (Vec<Pt>, Value) = match &start {
            Some(l) => (l.line.0.clone(), l.details.clone()),
            None => (self.base_line()?, Value::Null),
        };
        if line.len() < 2 {
            return Err(bad("the starting line needs at least two points".into()));
        }
        let river = self.river_polygons().ok_or_else(|| {
            bad("the optimise stage needs the river polygon: import it first".into())
        })?;
        let index = PolygonIndex::new(&river);
        let centre = self.river_centreline().ok();
        // The pools of an earlier, fresh run of the pool lattice: their weir sites are forced seams.
        let previous = self.doc["stages"]["optimise"]["results"].clone();
        let carried = previous.is_object()
            && previous["design_line"]["method"] != WINDOWS
            && self.stage_status(Stage::Optimise) == StageStatus::Fresh;
        let weirs_m: Vec<f64> = if carried {
            previous["cascade"]["weirs"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|w| w["chainage_m"].as_f64())
                .collect()
        } else {
            Vec::new()
        };
        // The water level along the starting line, as the volumes stage reads it, in a table every
        // 2 m (the search reads it often, from many threads).
        let len = linear::length(&line);
        let table: Vec<f64> = if plan_area {
            vec![0.0, 0.0]
        } else {
            let (profile, _) = self
                .water_profile()
                .ok_or_else(|| self.needs_water_profile(Stage::Optimise))?;
            let centre = centre
                .clone()
                .ok_or_else(|| bad("the optimise stage needs the river centreline".into()))?;
            let ix = linear::ProjectIndex::new(&centre);
            let n = (len / 2.0).ceil() as usize + 1;
            (0..n)
                .map(|i| {
                    let s = (i as f64 * 2.0).min(len);
                    let p = linear::interpolate(&line, s).unwrap_or([f64::NAN; 2]);
                    profile.level_at(crate::types::Chainage(ix.project(p)))
                })
                .collect()
        };
        let level = |s: f64| -> f64 {
            let u = (s / 2.0).clamp(0.0, (table.len() - 1) as f64);
            let i = (u.floor() as usize).min(table.len() - 2);
            let f = u - i as f64;
            if f == 0.0 {
                table[i]
            } else {
                table[i] * (1.0 - f) + table[i + 1] * f
            }
        };
        // Pools given as a water level (not by an earlier cascade): each step of the level is a
        // weir site, so a window never crosses one.
        let mut weirs_m = weirs_m;
        if !carried && !plan_area {
            weirs_m.extend(level_steps(&table));
        }
        let land_weight = if plan_area && typed.land_weight == 0.0 {
            1.0
        } else {
            typed.land_weight
        };
        // The record of a river-following line (OPT-4): its cut-throughs and forced stretches are
        // forced seams and permitted ground, each with its room.
        let room_default = ws.cut_through_room_m.unwrap_or(2.0 * bw);
        let mut cut_throughs = stretches(&start_details, "cut_throughs");
        let mut forced_stretches = stretches(&start_details, "forced_stretches");
        let mut permitted: Vec<(f64, f64, f64)> = cut_throughs
            .iter()
            .map(|&(a, b)| (a, b, room_default))
            .chain(forced_stretches.iter().map(|&(a, b)| (a, b, 0.5 * bw)))
            .collect();
        let records = permitted_records(&start_details);
        for (kind, a, b, room) in &records {
            if kind == "forced" {
                forced_stretches.push((*a, *b));
                permitted.push((*a, *b, room.unwrap_or(0.5 * bw)));
            } else {
                cut_throughs.push((*a, *b));
                permitted.push((*a, *b, room.unwrap_or(room_default)));
            }
        }
        let method = start.as_ref().map_or("centreline", |l| l.method.as_str());
        // A given line has no record: its own off-river stretches are its permitted ground (not
        // seams), so the mask never forbids what the line already does.
        if method == "given" {
            for c in start.iter().flat_map(|l| l.cut_offs.iter().flatten()) {
                permitted.push((c.from.0, c.to.0, room_default));
            }
        }
        let stay = match ws.stay_in_river.as_str() {
            "on" => true,
            "off" => false,
            _ => {
                matches!(method, "river_following" | "given")
                    || !cut_throughs.is_empty()
                    || !forced_stretches.is_empty()
            }
        };
        // "on" for any line: its own off-river stretches are permitted too.
        if stay && method != "given" && records.is_empty() && cut_throughs.is_empty() {
            for c in start.iter().flat_map(|l| l.cut_offs.iter().flatten()) {
                permitted.push((c.from.0, c.to.0, room_default));
            }
        }
        let cfg = self.window_config(
            ws,
            typed,
            r_min,
            loa,
            bw,
            depth,
            &vol,
            &model,
            land_weight,
            stay,
        )?;
        // With a DEM, the bed model under the river where the design has one: the volumes' ground.
        let plan = (!plan_area)
            .then(|| self.section_plan(&Value::Null))
            .transpose()?;
        let no_ground = move |xs: &[f64], _: &[f64]| vec![-(depth + 1000.0); xs.len()];
        let bedded;
        let ground: TerrainSampler = match (terrain, &plan) {
            (Some(t), Some(p)) => {
                bedded = Session::plan_terrain(p, t);
                &*bedded
            }
            _ => &no_ground,
        };
        #[cfg(not(target_arch = "wasm32"))]
        let clock = {
            let t0 = std::time::Instant::now();
            move || t0.elapsed().as_secs_f64()
        };
        #[cfg(target_arch = "wasm32")]
        let clock = || 0.0;
        let never = || false;
        let inputs = WindowInputs {
            line: &line,
            river: &river,
            river_index: &index,
            centreline: centre.as_deref(),
            terrain: ground,
            plan_area,
            level: &level,
            rule: &rule,
            weirs_m,
            cut_throughs_m: cut_throughs,
            forced_stretches_m: forced_stretches,
            permitted_m: permitted,
            frozen_m: Vec::new(),
            clock: &clock,
            cancel: &never,
        };
        let out = optimise_windows(&inputs, &cfg).map_err(bad)?;
        let settings = typed.to_value();
        let hash = super::hash::hash_value(&settings);
        let refined =
            self.windows_design_line(&out, &settings, &hash, r_min, plan_area, stay, &cfg)?;
        let st = &mut self.doc["stages"]["optimise"];
        let mut results = if carried {
            json!({
                "baseline": previous["baseline"],
                "base_level_m": previous["base_level_m"],
                "profile": previous["profile"],
                "pools": previous["pools"],
                "cascade": previous["cascade"],
            })
        } else {
            json!({
                "baseline": {"type": "LineString", "coordinates": line},
                "base_level_m": Value::Null,
                "profile": [],
                "pools": [],
                "cascade": {"total_cost": null, "reach_cost": null, "structure_cost": null, "weirs": []},
            })
        };
        results["method"] = json!(WINDOWS);
        results["design_line"] = serde_json::to_value(&refined).unwrap_or(Value::Null);
        if plan_area {
            results["optimised_on"] = json!(PLAN_AREA);
        }
        st["settings"] = settings;
        st["results"] = results;
        st["engine_version"] = json!(ENGINE_VERSION);
        let h = self.dependency_hash(Stage::Optimise);
        self.doc["stages"]["optimise"]["depends_on"] = json!(h);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn window_config(
        &self,
        ws: &WindowSettings,
        typed: &OptimiseSettings,
        r_min: f64,
        loa: f64,
        bw: f64,
        depth: f64,
        vol: &super::volumes::VolumesSettings,
        model: &str,
        land_weight: f64,
        stay: bool,
    ) -> Result<WindowConfig, DesignError> {
        let mut cfg = WindowConfig::defaults(r_min, loa, bw, depth);
        let mut lat = LatticeConfig::new(r_min);
        lat.pitch_m = ws.pitch_m;
        lat.n_heading = ws.n_heading;
        lat.radii_m = Some(match ws.radii_mode.as_str() {
            "multiples" => ws.radius_multiples.iter().map(|m| r_min * m).collect(),
            "list" => {
                let mut r: Vec<f64> = ws
                    .radii_m
                    .clone()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|r| *r >= r_min)
                    .collect();
                if !r.contains(&r_min) {
                    r.insert(0, r_min);
                }
                r
            }
            _ => geometric_radii(r_min, ws.n_radii as usize, ws.radius_max_factor),
        });
        lat.nominal_arc_m = Some(ws.nominal_arc_m);
        lat.straight_len_m = ws.straight_len_m;
        lat.min_straight_m = ws.min_straight_m.unwrap_or(loa);
        lat.corridor_limit = ws.corridor_limit;
        lat.heuristic = ws.heuristic.clone();
        lat.max_expand = ws.max_expand;
        lat.max_states = ws.max_states as usize;
        cfg.lattice = lat.validated().map_err(bad)?;
        cfg.window_target_m = ws.window_target_m;
        cfg.window_min_m = ws.window_min_m;
        cfg.window_max_m = ws.window_max_m;
        cfg.seam_step_m = ws.seam_step_m;
        if let Some(o) = ws.seam_max_offset_m {
            cfg.seam_max_offset_m = o;
        }
        if let Some(s) = &ws.seam_scales_m {
            cfg.seam_scales_m = s.clone();
        }
        cfg.second_pass = ws.second_pass;
        cfg.corridor_half_m = ws.corridor_half_m;
        cfg.cell_m = ws.cell_m;
        cfg.stay_in_river = stay;
        cfg.in_river_margin_m = ws.in_river_margin_m;
        if let Some(r) = ws.cut_through_room_m {
            cfg.cut_through_room_m = r;
        }
        cfg.window_time_s = ws.window_time_s;
        cfg.polish = if ws.polish {
            let mut p: PolishSettings = cfg.polish.clone().expect("the defaults polish");
            if let Some(s) = &ws.polish_spacings_m {
                p.spacings_m = s.clone();
            }
            if let Some(m) = ws.polish_max_shift_m {
                p.max_shift_m = m;
            }
            p.station_m = ws.polish_station_m;
            if let Some(m) = ws.polish_radius_margin_m {
                p.radius_margin_m = m;
            }
            Some(p)
        } else {
            None
        };
        cfg.adopt_threshold = ws.adopt_threshold;
        cfg.turn_limit_deg = ws.turn_limit_deg;
        cfg.length_ratio_limit = ws.length_ratio_limit;
        cfg.search_dx_m = ws.search_rake_step_m;
        cfg.search_ds_m = ws.search_ds_m;
        cfg.weights = Weights {
            wet: typed.wet_weight,
            dry: typed.dry_weight,
            land: land_weight,
            length: typed.length_weight,
        };
        cfg.section = SectionSettings {
            model: model.to_string(),
            depth_m: depth,
            wet_slope: vol.wet_slope,
            maxw_m: vol.maxw_m,
            dx_m: vol.dx_m,
            daylight_run_m: None,
        };
        cfg.station_m = vol.station_m;
        cfg.weir_straight_m = typed.weir_straight_m.unwrap_or(2.0 * loa);
        cfg.threads = window_threads(ws.threads);
        cfg.memory_budget_mb = ws.memory_budget_mb;
        Ok(cfg)
    }

    #[allow(clippy::too_many_arguments)]
    fn windows_design_line(
        &self,
        out: &WindowsResult,
        settings: &Value,
        hash: &str,
        r_min: f64,
        plan_area: bool,
        stay: bool,
        cfg: &WindowConfig,
    ) -> Result<DesignLine, DesignError> {
        let chord = default_check_chord(r_min);
        let checks = json!({
            "r_min_m": r_min,
            "radius_ok": out.radius_ok,
            "measured_min_radius_m": num(out.min_radius_m),
            "radius_check_chord_m": chord,
            "no_self_crossing": {
                "ok": out.self_crossing_m.is_none(),
                "first_crossing_m": out.self_crossing_m,
            },
            "min_straight": {
                "required_m": cfg.lattice.min_straight_m,
                "short": out.short_straights.iter().map(|(a, l)| json!({"at_m": a, "straight_m": l})).collect::<Vec<_>>(),
            },
        });
        let adopted = out
            .windows
            .iter()
            .filter(|w| w.adopted != Adopted::None)
            .count();
        let mut details = json!({
            "method": WINDOWS,
            "optimised_on": if plan_area { PLAN_AREA } else { "volume" },
            "n_windows": out.windows.len(),
            "n_adopted": adopted,
            "stay_in_river": stay,
            "windows_in_flight": out.windows_in_flight,
            "seconds": num(out.seconds),
            "seams": out.seams.iter().map(seam_json).collect::<Vec<_>>(),
            "windows": out.windows.iter().map(|w| record_json(w, plan_area, hash)).collect::<Vec<_>>(),
            "totals": {
                "start": measure_json(&out.start_total, plan_area),
                "result": measure_json(&out.result_total, plan_area),
                "saving": num(out.end_to_end_saving),
                "saving_pct": num(if out.start_total.cost > 0.0 { 100.0 * out.end_to_end_saving / out.start_total.cost } else { 0.0 }),
                "window_saving_sum": num(out.window_saving_sum),
                "agree": out.totals_agree,
            },
            "settings_hash": hash,
        });
        if plan_area {
            details["optimised_on_note"] = json!(PLAN_AREA_NOTE);
        }
        if let Some(cell) = owd_core::types::grid::take_grid_read() {
            details["ground_read_cell_m"] = json!(cell);
        }
        let river_geometry = self
            .river_geometry()
            .map_err(super::rule_sets::stage_error)?;
        let mut refined = design_line::finish(
            WINDOWS,
            "Window optimiser",
            WINDOWS,
            settings.clone(),
            out.line.clone(),
            checks,
            details,
            &river_geometry,
        );
        self.mark_optimised_constraints(&mut refined)?;
        if let Some(w) = refined.sanity["warnings"].as_array_mut() {
            if plan_area {
                w.push(json!(PLAN_AREA_NOTE));
            }
            for text in &out.warnings {
                w.push(json!(text));
            }
            for (a, l) in &out.short_straights {
                w.push(json!(format!(
                    "at {a:.0} m the straight between two opposite curves is {l:.0} m, under the \
                     {:.0} m asked",
                    cfg.lattice.min_straight_m
                )));
            }
        }
        Ok(refined)
    }
}

/// The windows searched at once: the setting, else every core; one in the browser.
fn window_threads(setting: Option<i64>) -> usize {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = setting;
        1
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        match setting {
            Some(t) => t.max(1) as usize,
            None => std::thread::available_parallelism().map_or(1, |n| n.get()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Label: property (the river-following record's hand-over shape is read, both keys).
    #[test]
    fn the_permitted_stretches_of_a_record_are_read() {
        let d = json!({"permitted_stretches": [
            {"id": "ct-1", "kind": "cut_through", "from": 100.0, "to": 900.0, "room_m": 60.0},
            {"id": "f-1", "kind": "forced", "from": 2000.0, "to": 2150.0, "room_m": 15.0},
            {"id": "bad", "kind": "forced", "from": 10.0, "to": 5.0},
        ]});
        let r = permitted_records(&d);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0], ("cut_through".to_string(), 100.0, 900.0, Some(60.0)));
        assert_eq!(r[1].0, "forced");
        let nested = json!({"record": d.clone()});
        assert_eq!(permitted_records(&nested), r);
        assert!(permitted_records(&json!({})).is_empty());
    }

    /// Label: property (a step of the water level is a weir site; a sloping level is not).
    #[test]
    fn a_step_of_the_level_is_a_weir_site() {
        let mut t: Vec<f64> = (0..1000).map(|i| 10.0 - 1e-4 * i as f64).collect();
        assert!(level_steps(&t).is_empty());
        for v in t.iter_mut().skip(500) {
            *v -= 1.0;
        }
        assert_eq!(level_steps(&t), vec![999.0]);
    }
}
