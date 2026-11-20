//! The water level (spec 0002, ADR 0006): a profile along the river centreline. A design line's
//! station takes its level by projecting onto the river centreline and reading the profile at that
//! chainage, so two design lines through the same place see the same water.
//!
//! The profile is a table of `(river chainage, level)` points, read linearly between points and held
//! flat beyond the first and last. Two points at one chainage make a step (a weir): at that chainage
//! the later point holds. The optimise stage's pool levels are such a table and, once it has run,
//! they are the profile the volumes stage reads.

use super::hash::hash_value;
use super::session::{DesignError, ENGINE_VERSION, Session, Stage, StageStatus};
use super::water_stage::{self, BeyondPools, DEFAULT_STEP_M, DesignWater, Pool, WaterIn};
use crate::types::{Chainage, Polyline, WaterProfile};
use owd_core::geom::{Pt, linear};
use serde_json::{Value, json};

/// How a design line's station finds its place on the profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WaterSampling {
    /// Project the station onto the river centreline. The shipped way (ADR 0006).
    #[default]
    Projection,
    /// Read the profile at the design line's own chainage, as the frozen Python does. For tests of
    /// the port only: it is not offered by the CLI or the app.
    DesignLineChainage,
}

/// The profile held as `[chainage, level]` pairs in `v`: a pair that is not two numbers is skipped.
fn profile_of(v: &Value) -> WaterProfile {
    let pairs: Vec<[f64; 2]> = v
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| Some([p[0].as_f64()?, p[1].as_f64()?]))
                .collect()
        })
        .unwrap_or_default();
    WaterProfile::from_pairs(&pairs)
}

impl Session {
    /// Read the profile at the stations of design lines by the test-only sampling of the frozen
    /// Python (the design line's own chainage). Not for the CLI or the app.
    #[doc(hidden)]
    pub fn sample_water_at_design_line_chainage(&mut self) {
        self.water_sampling = WaterSampling::DesignLineChainage;
    }

    /// Set the water level as a table of `(river chainage in metres, level in metres)` points. One
    /// point is a constant level. Chainages must not decrease.
    pub fn set_water_profile(&mut self, points: &[[f64; 2]]) -> Result<(), DesignError> {
        if points.is_empty() {
            return Err(DesignError::Input(
                "a water level profile needs at least one point".into(),
            ));
        }
        if points
            .iter()
            .any(|p| !(p[0].is_finite() && p[1].is_finite()))
        {
            return Err(DesignError::Input(
                "every point of the water level profile must be a finite chainage and level".into(),
            ));
        }
        if points.windows(2).any(|w| w[1][0] < w[0][0]) {
            return Err(DesignError::Input(
                "the chainages of the water level profile must not decrease".into(),
            ));
        }
        self.doc["inputs"]["water_level"] = json!({ "points": points });
        Ok(())
    }

    /// Set the water level as pools: rows of `[river chainage in metres, level in metres]`, one
    /// per pool, the chainage where it starts. A level holds from its chainage to the next row; the
    /// first also upstream of its chainage, the last to the end of the river. Nothing is read
    /// between two levels. Stored as the stepped profile the document already has: each row, then
    /// the same level at the next row's chainage. A refused table changes nothing.
    pub fn set_water_pools(&mut self, pools: &[[f64; 2]]) -> Result<(), DesignError> {
        if pools.is_empty() {
            return Err(DesignError::Input(
                "a pool table needs at least one pool".into(),
            ));
        }
        if pools
            .iter()
            .any(|p| !(p[0].is_finite() && p[1].is_finite()))
        {
            return Err(DesignError::Input(
                "every pool needs a finite chainage and level".into(),
            ));
        }
        if let Some(w) = pools.windows(2).find(|w| w[1][0] <= w[0][0]) {
            return Err(DesignError::Input(format!(
                "the chainage of each pool must be above the one before: {} m follows {} m \
                 (two rows at one chainage are a pool with no length)",
                w[1][0], w[0][0]
            )));
        }
        let mut points = Vec::with_capacity(2 * pools.len() - 1);
        for (i, p) in pools.iter().enumerate() {
            points.push(*p);
            if let Some(next) = pools.get(i + 1) {
                points.push([next[0], p[1]]);
            }
        }
        self.set_water_profile(&points)
    }

    /// [`Session::set_water_pools`] from text: lines of `chainage_m,level_m`, read as
    /// `--water-table` reads its file. A blank line and a `#` line are skipped, and a first line
    /// that is not two numbers is a header; any other line that is not two numbers is refused by
    /// its number. The one reader of the text for every surface.
    pub fn set_water_pools_text(&mut self, text: &str) -> Result<(), DesignError> {
        let mut pools = Vec::new();
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let f: Vec<&str> = line.split(',').map(str::trim).collect();
            match (
                f.first().and_then(|x| x.parse::<f64>().ok()),
                f.get(1).and_then(|x| x.parse::<f64>().ok()),
            ) {
                (Some(c), Some(l)) if f.len() == 2 => pools.push([c, l]),
                _ if n == 0 => {} // a header line
                _ => {
                    return Err(DesignError::Input(format!(
                        "the pool table, line {}: expected `chainage_m,level_m`",
                        n + 1
                    )));
                }
            }
        }
        self.set_water_pools(&pools)
    }

    /// A straight fall: `start_m_level` at the river's start, falling `slope` per metre of river
    /// chainage to its end. Needs the river centreline (or both banks) to be imported first.
    pub fn set_water_fall(&mut self, start_level_m: f64, slope: f64) -> Result<(), DesignError> {
        let len = linear::length(&self.base_line()?);
        self.set_water_profile(&[[0.0, start_level_m], [len, start_level_m - slope * len]])
    }

    /// The water level the later stages read: the pool levels the optimise stage set, if it has run,
    /// else the profile of the inputs. The second value says which.
    pub fn water_profile(&self) -> Option<(WaterProfile, &'static str)> {
        let opt = &self.doc["stages"]["optimise"]["results"];
        if !opt.is_null() {
            let p = profile_of(&opt["profile"]);
            if !p.is_empty() {
                return Some((p, "optimise"));
            }
        }
        self.input_water_profile().map(|p| (p, "input"))
    }

    /// The optimise stage's pool levels as pools: its stored profile is each pool's two ends on river
    /// chainage, in order. Empty when it has not run.
    fn optimise_pools(&self) -> Vec<Pool> {
        let opt = &self.doc["stages"]["optimise"]["results"];
        if opt.is_null() {
            return vec![];
        }
        profile_of(&opt["profile"])
            .to_pairs()
            .chunks(2)
            .map(|c| Pool {
                from: Chainage(c[0][0]),
                to: Chainage(c.get(1).map_or(c[0][0], |e| e[0])),
                level_m: c[0][1],
                min_level_m: None,
                max_level_m: None,
            })
            .collect()
    }

    /// What the water stage reads (spec 0017 ticket 8): the river centreline, the design line the
    /// later stages read, the water profile of the inputs and the optimise stage's pools, read as
    /// the volumes stage has always read them (held flat beyond their ends). `None` without a design
    /// line or a river centreline.
    ///
    /// With given structures (ticket 14), the pools are theirs, not the optimise stage's: each is
    /// held flat from its structure to the next, with its minimum and maximum level, and the river
    /// is open below the lowest structure. `None` too when the structures cannot be placed on the
    /// line ([`Session::try_water_input`] says why).
    pub fn water_input(&self) -> Option<WaterIn> {
        self.try_water_input().ok().flatten()
    }

    /// [`Session::water_input`], with the reason when the given structures cannot be placed.
    pub fn try_water_input(&self) -> Result<Option<WaterIn>, DesignError> {
        let Some(design_line) = self.final_line().map(|d| d.line) else {
            return Ok(None);
        };
        let Ok(centreline) = self.river_centreline() else {
            return Ok(None);
        };
        self.water_input_on(design_line, Polyline(centreline))
            .map(Some)
    }

    /// What the water stage reads for `design_line`: the same river, water and pools. The step is
    /// the volumes stage's station step, so each volumes station is a point of the answer.
    pub(super) fn water_input_on(
        &self,
        design_line: Polyline,
        centreline: Polyline,
    ) -> Result<WaterIn, DesignError> {
        let step_m = self
            .volumes_settings(&Value::Null)
            .map_or(DEFAULT_STEP_M, |v| v.station_m);
        let (pools, beyond_pools) =
            match self.structure_plan_on(design_line.clone(), centreline.clone())? {
                Some((_, plan)) => (plan.water_pools(), BeyondPools::OpenRiver),
                None => (self.optimise_pools(), BeyondPools::HoldPoolLevel),
            };
        Ok(WaterIn {
            centreline,
            design_line,
            given: self.input_water_profile(),
            pools,
            beyond_pools,
            step_m,
        })
    }

    /// Where the stored water stands: not run, fresh, or stale. It is fresh while the hash of what
    /// it would read now equals the one it recorded.
    pub fn water_status(&self) -> StageStatus {
        let r = &self.doc["stages"][water_stage::STAGE];
        if !r["results"].is_object() {
            return StageStatus::NotRun;
        }
        match (r["depends_on"].as_str(), self.water_input()) {
            (Some(h), Some(i)) if h == hash_value(&json!(i)) => StageStatus::Fresh,
            _ => StageStatus::Stale,
        }
    }

    /// The stored water along the design line, fresh or not. `None` before it has run.
    pub fn design_water(&self) -> Option<DesignWater> {
        let r = &self.doc["stages"][water_stage::STAGE]["results"];
        r.is_object()
            .then(|| serde_json::from_value(r.clone()).ok())
            .flatten()
    }

    /// Run the water stage and store it as a stage record: the step as its settings, the answer as
    /// its results, the hash of its input as `depends_on`. Not a stage of the chain: the volumes
    /// stage runs it, and no other stage's hash reads it.
    pub fn run_water(&mut self) -> Result<(), DesignError> {
        let Some(input) = self.try_water_input()? else {
            return Err(DesignError::Input(if self.final_line().is_none() {
                "the water stage needs a design line: run the alignment stage first".into()
            } else {
                "the water stage needs the river centreline: import the river first".into()
            }));
        };
        let out = water_stage::water(&input).map_err(|e| DesignError::Input(e.message))?;
        if !self.doc["stages"].is_object() {
            self.doc["stages"] = json!({});
        }
        self.doc["stages"][water_stage::STAGE] = json!({
            "settings": {"step_m": input.step_m},
            "results": out,
            "depends_on": hash_value(&json!(input)),
            "engine_version": ENGINE_VERSION,
        });
        Ok(())
    }

    /// The profile set by hand or by import.
    pub fn input_water_profile(&self) -> Option<WaterProfile> {
        let p = profile_of(&self.doc["inputs"]["water_level"]["points"]);
        (!p.is_empty()).then_some(p)
    }

    /// The water level along a design line, as a function of the line's own chainage.
    pub(super) fn water_level_along<'a>(
        &self,
        profile: &'a WaterProfile,
        line: &'a [Pt],
        centre: &'a [Pt],
    ) -> impl Fn(f64) -> f64 + 'a {
        let mode = self.water_sampling;
        move |s| match mode {
            WaterSampling::DesignLineChainage => profile.level_at(Chainage(s)),
            WaterSampling::Projection => {
                let at = linear::interpolate(line, s).unwrap_or([f64::NAN; 2]);
                profile.level_at(Chainage(linear::project(centre, at)))
            }
        }
    }

    /// The water level at stations of a design line, given as chainages along that line. It is the
    /// level the stages read: the same place gives the same level whatever line asks.
    pub fn water_levels_along(
        &self,
        line: &[Pt],
        chainages_m: &[f64],
    ) -> Result<Vec<f64>, DesignError> {
        let (profile, _) = self
            .water_profile()
            .ok_or_else(|| self.needs_water_profile(Stage::Volumes))?;
        let centre = self.river_centreline()?;
        let level = self.water_level_along(&profile, line, &centre);
        Ok(chainages_m.iter().map(|&s| level(s)).collect())
    }

    /// The pool levels of the optimise stage as a profile on the river centreline: each pool holds its
    /// level between the river chainages of its two ends, with a step where one pool meets the next.
    pub(super) fn pool_profile(
        &self,
        baseline: &[Pt],
        pools: &[(f64, f64, f64)],
    ) -> Result<Vec<[f64; 2]>, DesignError> {
        let centre = self.base_line()?;
        let at = |c: f64| {
            let p = linear::interpolate(baseline, c).unwrap_or([f64::NAN; 2]);
            linear::project(&centre, p)
        };
        let mut out: Vec<[f64; 2]> = Vec::new();
        for &(a, b, l) in pools {
            let ca = at(a).max(out.last().map_or(f64::NEG_INFINITY, |p| p[0]));
            let cb = at(b).max(ca);
            out.push([ca, l]);
            out.push([cb, l]);
        }
        Ok(out)
    }

    /// Move the old per-stage water settings (a start level and a fall per metre of design-line
    /// chainage) into a two-point profile on the river centreline. The settings change, so every stage
    /// that held one shows as stale: its results came from the old model and are not kept as fresh.
    pub(super) fn migrate_water_settings(&mut self) {
        let mut start: Option<f64> = None;
        let mut slope = 0.0;
        let mut found = false;
        for (stage, holder, key) in [
            ("alignment", Some("reroute"), "water_level_start_m"),
            ("volumes", None, "water_level_start_m"),
        ] {
            for k in [key, "water_surface_slope"] {
                // `pointer_mut`, not `[]`: indexing a null for writing would turn it into an object.
                let path = match holder {
                    Some(h) => format!("/stages/{stage}/settings/{h}"),
                    None => format!("/stages/{stage}/settings"),
                };
                let obj = self.doc.pointer_mut(&path).and_then(Value::as_object_mut);
                let Some(o) = obj else { continue };
                if let Some(v) = o.remove(k) {
                    found = true;
                    if k == key && start.is_none() {
                        start = v.as_f64();
                    }
                    if k == "water_surface_slope" && slope == 0.0 {
                        slope = v.as_f64().unwrap_or(0.0);
                    }
                }
            }
        }
        if !found || self.input_water_profile().is_some() {
            return;
        }
        let Some(wl0) = start else { return };
        let len = self.base_line().map_or(0.0, |l| linear::length(&l));
        let pts = if len > 0.0 {
            vec![[0.0, wl0], [len, wl0 - slope * len]]
        } else {
            vec![[0.0, wl0]]
        };
        self.doc["inputs"]["water_level"] = json!({ "points": pts });
    }

    pub(super) fn river_centreline(&self) -> Result<Vec<Pt>, DesignError> {
        self.base_line()
    }

    /// The river centreline the water level profile is read on (its chainage is the profile's), and
    /// the study's UTM zone it is drawn in. For a generated water level (spec 0012).
    pub fn water_reference(&self) -> Result<(Vec<Pt>, super::crs::Utm), DesignError> {
        let utm = self.doc["project"]["crs"]["epsg"]
            .as_u64()
            .and_then(|c| super::crs::Crs::from_epsg(c as u32).ok())
            .and_then(|c| match c {
                super::crs::Crs::Utm(u) => Some(u),
                super::crs::Crs::Wgs84 => None,
            })
            .ok_or_else(|| {
                DesignError::Input("the document has no UTM zone: import the river first".into())
            })?;
        Ok((self.base_line()?, utm))
    }

    pub(super) fn needs_water_profile(&self, stage: Stage) -> DesignError {
        DesignError::Input(format!(
            "the {} stage needs a water level: set one with `owd import --water-level LEVEL` \
             (a constant), `--water-slope` (a fall per metre of river chainage) or `--water-table FILE`",
            stage.name()
        ))
    }
}
