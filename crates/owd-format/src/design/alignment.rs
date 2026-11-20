//! The alignment stage on the document (spec 0002, spec 0017 ticket 6): one design line per run.
//!
//! The method is a setting (`stages.alignment.settings.method`); the stage itself is the pure
//! function [`super::design_line::alignment`], and the session only reads its inputs from the
//! document and stores the [`DesignLine`] it returns under `stages.alignment.results`.
//!
//! Spec 0005 (#100): the rolling circle is water-aware when a river polygon is present (setting
//! `use_water`, default on then, whether the user gave the polygon or the import built it from the
//! two banks; a document from before spec 0005 records it off on open). Its line records the value
//! used and then carries the `fits_in_water` check. Optional `seed_legs` in the inputs are joined
//! first (`join_legs`, with the `fixed` pieces held), and the line carries the `join_guard` check.

use super::design_line::{AlignmentIn, DesignLine, GivenLine, PoseIn, SeedLegs};
use super::geojson::all_positions;
use super::rule_sets::stage_error;
use super::session::{DesignError, ENGINE_VERSION, Session, Stage, StageStatus};
use crate::types::{Polyline, RiverPolygon, TerrainSampler};
use owd_core::alignment::centreline::build_centreline;
use owd_core::geom::{Polygon, Pt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// A way of finding a design line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AlignMethod {
    /// The rolling circle (water-aware when there is a river polygon).
    #[default]
    RollingCircle,
    /// The least-cost re-route over the terrain.
    Reroute,
    /// The river centreline as it is, with no repair: a line to compare with. It may be tighter
    /// than the minimum radius; its checks say so.
    Centreline,
    /// A line the user gives (ticket C21).
    Given,
}

impl AlignMethod {
    pub fn name(self) -> &'static str {
        match self {
            AlignMethod::RollingCircle => "rolling_circle",
            AlignMethod::Reroute => "reroute",
            AlignMethod::Centreline => "centreline",
            AlignMethod::Given => "given",
        }
    }

    pub fn from_name(s: &str) -> Option<AlignMethod> {
        match s {
            "rolling_circle" | "rolling-circle" => Some(AlignMethod::RollingCircle),
            "reroute" => Some(AlignMethod::Reroute),
            "centreline" => Some(AlignMethod::Centreline),
            "given" => Some(AlignMethod::Given),
            _ => None,
        }
    }

    pub(crate) fn option_id(self) -> &'static str {
        match self {
            AlignMethod::RollingCircle => "rolling-circle",
            AlignMethod::Reroute => "reroute",
            AlignMethod::Centreline => "centreline",
            AlignMethod::Given => "given",
        }
    }

    pub(crate) fn option_name(self) -> &'static str {
        match self {
            AlignMethod::RollingCircle => "Rolling circle",
            AlignMethod::Reroute => "Least-cost re-route",
            AlignMethod::Centreline => "River centreline",
            AlignMethod::Given => "Given line",
        }
    }
}

/// The settings of the rolling circle, typed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct RollingCircleSettings {
    /// The sampling step in metres.
    pub step_m: f64,
    /// The number of passes.
    pub passes: i64,
    /// The chord the minimum radius is read on; `None`: a third of R_min.
    pub check_chord_m: Option<f64>,
    /// Keep the line in the water; `None`: when the document has a river polygon.
    pub use_water: Option<bool>,
    pub clearance_m: f64,
    pub r_pref_m: Option<f64>,
}

/// The settings of the least-cost re-route, typed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct RerouteSettings {
    /// The half-widths of the corridors to try, in metres.
    pub corridor_sweep_m: Vec<f64>,
    pub station_step_m: f64,
    pub offset_step_m: f64,
    pub side_slope: f64,
    pub wet_weight: f64,
    pub dry_weight: f64,
    pub check_chord_m: Option<f64>,
    /// The fairness weight; `None` (a document from before spec 0005) reads as 0.
    pub fairness_weight: Option<f64>,
}

/// The settings of the alignment stage: the method that makes the design line, one typed part per
/// method under the method's name, and the given line, as the document holds them. Each method
/// knows its own keys only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct AlignmentSettings {
    /// The method that makes the one design line.
    pub method: AlignMethod,
    pub rolling_circle: RollingCircleSettings,
    pub reroute: RerouteSettings,
    pub centreline: CentrelineSettings,
    /// The line the user gave, for the method `given`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub given: Option<GivenLine>,
}

/// The settings of the river centreline as a design line: the chord its radius is read on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct CentrelineSettings {
    /// The chord the minimum radius is read on; `None`: a third of R_min.
    pub check_chord_m: Option<f64>,
}

impl CentrelineSettings {
    fn apply(&self, source: &Value) -> Result<CentrelineSettings, DesignError> {
        let mut out = self.clone();
        let Some(o) = source.as_object() else {
            return Ok(out);
        };
        for k in o.keys() {
            match k.as_str() {
                "check_chord_m" => out.check_chord_m = opt_num(source, k)?,
                other => {
                    return Err(DesignError::Input(format!(
                        "the river centreline has no setting `{other}`"
                    )));
                }
            }
        }
        Ok(out)
    }
}

impl Default for AlignmentSettings {
    fn default() -> Self {
        AlignmentSettings {
            method: AlignMethod::RollingCircle,
            rolling_circle: RollingCircleSettings {
                step_m: 10.0,
                passes: 2,
                check_chord_m: None,
                use_water: None,
                clearance_m: 0.0,
                r_pref_m: None,
            },
            reroute: RerouteSettings {
                corridor_sweep_m: vec![40.0, 80.0, 120.0],
                station_step_m: 100.0,
                offset_step_m: 10.0,
                side_slope: 3.0,
                wet_weight: 1.0,
                dry_weight: 1.0,
                check_chord_m: None,
                fairness_weight: Some(0.0),
            },
            centreline: CentrelineSettings {
                check_chord_m: None,
            },
            given: None,
        }
    }
}

impl Default for RollingCircleSettings {
    fn default() -> Self {
        AlignmentSettings::default().rolling_circle
    }
}

impl Default for RerouteSettings {
    fn default() -> Self {
        AlignmentSettings::default().reroute
    }
}

impl Default for CentrelineSettings {
    fn default() -> Self {
        AlignmentSettings::default().centreline
    }
}

impl RollingCircleSettings {
    fn apply(&self, source: &Value) -> Result<RollingCircleSettings, DesignError> {
        let mut out = self.clone();
        let Some(o) = source.as_object() else {
            return Ok(out);
        };
        for (k, v) in o {
            match k.as_str() {
                "step_m" => out.step_m = num(source, k)?,
                "passes" => {
                    out.passes = v.as_i64().ok_or_else(|| {
                        DesignError::Input("the setting `passes` must be a whole number".into())
                    })?
                }
                "check_chord_m" => out.check_chord_m = opt_num(source, k)?,
                "use_water" => {
                    out.use_water = match v {
                        Value::Bool(b) => Some(*b),
                        Value::Null => None,
                        _ => {
                            return Err(DesignError::Input(
                                "the setting `use_water` must be true, false or null".into(),
                            ));
                        }
                    }
                }
                "clearance_m" => out.clearance_m = num(source, k)?,
                "r_pref_m" => out.r_pref_m = opt_num(source, k)?,
                other => {
                    return Err(DesignError::Input(format!(
                        "the rolling circle has no setting `{other}`"
                    )));
                }
            }
        }
        Ok(out)
    }
}

impl RerouteSettings {
    fn apply(&self, source: &Value) -> Result<RerouteSettings, DesignError> {
        let mut out = self.clone();
        let Some(o) = source.as_object() else {
            return Ok(out);
        };
        for (k, v) in o {
            match k.as_str() {
                "corridor_sweep_m" => {
                    out.corridor_sweep_m = v
                        .as_array()
                        .and_then(|a| a.iter().map(Value::as_f64).collect::<Option<Vec<f64>>>())
                        .ok_or_else(|| {
                            DesignError::Input(
                                "the setting `corridor_sweep_m` must be a list of numbers".into(),
                            )
                        })?
                }
                "station_step_m" => out.station_step_m = num(source, k)?,
                "offset_step_m" => out.offset_step_m = num(source, k)?,
                "side_slope" => out.side_slope = num(source, k)?,
                "wet_weight" => out.wet_weight = num(source, k)?,
                "dry_weight" => out.dry_weight = num(source, k)?,
                "check_chord_m" => out.check_chord_m = opt_num(source, k)?,
                "fairness_weight" => {
                    out.fairness_weight = match v {
                        Value::Null => None,
                        v => Some(v.as_f64().ok_or_else(|| {
                            DesignError::Input(
                                "the setting `fairness_weight` must be a number at or above 0"
                                    .into(),
                            )
                        })?),
                    }
                }
                other => {
                    return Err(DesignError::Input(format!(
                        "the re-route has no setting `{other}`"
                    )));
                }
            }
        }
        Ok(out)
    }
}

impl AlignmentSettings {
    /// The settings as the stage writes them into the document.
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("the settings are JSON")
    }

    /// The settings of one method as a value.
    fn part(&self, method: AlignMethod) -> Value {
        match method {
            AlignMethod::RollingCircle => serde_json::to_value(&self.rolling_circle),
            AlignMethod::Reroute => serde_json::to_value(&self.reroute),
            AlignMethod::Centreline => serde_json::to_value(&self.centreline),
            AlignMethod::Given => Ok(json!({})),
        }
        .expect("the settings are JSON")
    }

    /// Lay `source` (the settings of `method`, an object or null) over this method's part. A key the
    /// method does not know, or a bad value, is refused by name.
    fn apply_part(&mut self, method: AlignMethod, source: &Value) -> Result<(), DesignError> {
        match method {
            AlignMethod::RollingCircle => {
                self.rolling_circle = self.rolling_circle.apply(source)?
            }
            AlignMethod::Reroute => self.reroute = self.reroute.apply(source)?,
            AlignMethod::Centreline => self.centreline = self.centreline.apply(source)?,
            AlignMethod::Given => {
                if source.as_object().is_some_and(|o| !o.is_empty()) {
                    return Err(DesignError::Input(
                        "a given design line has no settings: give the line with `owd design-line`"
                            .into(),
                    ));
                }
            }
        }
        Ok(())
    }

    /// The defaults with the document's stored settings laid over them, each key checked.
    fn from_document(stored: &Value) -> Result<AlignmentSettings, DesignError> {
        let mut out = AlignmentSettings::default();
        for m in [
            AlignMethod::RollingCircle,
            AlignMethod::Reroute,
            AlignMethod::Centreline,
        ] {
            out.apply_part(m, &stored[m.name()])?;
        }
        if let Some(name) = stored.get("method").filter(|v| !v.is_null()) {
            out.method = name
                .as_str()
                .and_then(AlignMethod::from_name)
                .ok_or_else(|| {
                    DesignError::Input(format!(
                        "the alignment method `{name}` is not one OWD knows: use rolling_circle, \
                         reroute, centreline or given"
                    ))
                })?;
        }
        if let Some(g) = stored.get("given").filter(|v| !v.is_null()) {
            out.given = Some(serde_json::from_value(g.clone()).map_err(|e| {
                DesignError::Input(format!("the given design line cannot be read: {e}"))
            })?);
        }
        Ok(out)
    }
}

pub(super) fn pts_of(line: &Value) -> Option<Vec<Pt>> {
    (line["type"] == "LineString").then(|| all_positions(line))
}

fn num(v: &Value, key: &str) -> Result<f64, DesignError> {
    v[key]
        .as_f64()
        .ok_or_else(|| DesignError::Input(format!("the setting `{key}` must be a number")))
}

fn opt_num(v: &Value, key: &str) -> Result<Option<f64>, DesignError> {
    match &v[key] {
        Value::Null => Ok(None),
        x => x.as_f64().map(Some).ok_or_else(|| {
            DesignError::Input(format!("the setting `{key}` must be a number or null"))
        }),
    }
}

impl Session {
    fn upstream_ready(&self) -> Result<(), DesignError> {
        match self.stage_status(Stage::Sizing) {
            StageStatus::Stale => Err(DesignError::StaleUpstream {
                stage: Stage::Alignment,
                run_first: Stage::Sizing,
            }),
            StageStatus::NotRun => Err(DesignError::Input(
                "the sizing stage has not been run: choose a ruleset and a vessel class first"
                    .into(),
            )),
            StageStatus::Fresh => Ok(()),
        }
    }

    /// The river centreline the methods start from: the imported one, or the smoothed line midway
    /// between the two banks. It is the river stage's centreline: the inputs are stored from the
    /// mouth when the design states one.
    pub(super) fn base_line(&self) -> Result<Vec<Pt>, DesignError> {
        let inp = &self.doc["inputs"];
        if let Some(c) = pts_of(&inp["centreline"]) {
            return Ok(c);
        }
        match (pts_of(&inp["left_bank"]), pts_of(&inp["right_bank"])) {
            (Some(l), Some(r)) => build_centreline(&l, &r, 20.0, 5, 2).map_err(DesignError::Input),
            _ => Err(DesignError::Input(
                "the alignment needs a river centreline or both bank lines: import them first"
                    .into(),
            )),
        }
    }

    /// The alignment settings the document holds, over the defaults. A key a method does not know
    /// is refused by name.
    pub fn alignment_settings(&self) -> Result<AlignmentSettings, DesignError> {
        AlignmentSettings::from_document(&self.doc["stages"]["alignment"]["settings"])
    }

    /// The settings of one method: the defaults, then the document's, then `overrides`. A key the
    /// method does not know, in the document or from the caller, is refused by name.
    fn method_settings(
        &self,
        method: AlignMethod,
        overrides: &Value,
    ) -> Result<AlignmentSettings, DesignError> {
        let mut s = self.alignment_settings()?;
        s.apply_part(method, overrides)?;
        Ok(s)
    }

    /// Set settings of one method in the document's alignment settings: the defaults, then what
    /// the document holds, then `settings`. Nothing runs; a fresh alignment shows as stale. The
    /// command line (`owd align --use-water`) and the app shell both call this. A key the method
    /// does not know, or a bad value, is refused by name and changes nothing.
    pub fn set_alignment_settings(
        &mut self,
        method: AlignMethod,
        settings: &Value,
    ) -> Result<(), DesignError> {
        let merged = self.method_settings(method, settings)?;
        let st = &mut self.doc["stages"]["alignment"]["settings"];
        if !st.is_object() {
            *st = json!({});
        }
        st[method.name()] = merged.part(method);
        Ok(())
    }

    /// The seed legs and fixed pieces of the inputs (spec 0005), when there are legs.
    fn seed_legs(&self) -> Option<SeedLegs> {
        let legs: Vec<Polyline> = self.doc["inputs"]["seed_legs"]
            .as_array()?
            .iter()
            .filter_map(pts_of)
            .map(Polyline)
            .collect();
        if legs.is_empty() {
            return None;
        }
        let f = &self.doc["inputs"]["fixed"];
        let pose = |v: &Value| -> Option<PoseIn> {
            let p = v["point"].as_array()?;
            Some(PoseIn {
                point: [p.first()?.as_f64()?, p.get(1)?.as_f64()?],
                heading_deg: v["heading_deg"].as_f64()?,
            })
        };
        Some(SeedLegs {
            legs,
            start_pose: pose(&f["start_pose"]),
            end_pose: pose(&f["end_pose"]),
            must_follow: f["must_follow"]
                .as_array()
                .map(|a| a.iter().filter_map(pts_of).map(Polyline).collect())
                .unwrap_or_default(),
        })
    }

    /// What the alignment stage reads, from the document, with `settings` for the settings.
    pub fn alignment_input(&self, settings: AlignmentSettings) -> Result<AlignmentIn, DesignError> {
        let river = self.river_geometry().map_err(stage_error)?;
        Ok(AlignmentIn {
            river,
            size: self.channel_size()?,
            settings,
            seed: self.seed_legs(),
            constraints: self.constraints()?,
        })
    }

    /// Run the alignment stage with `method` and store its one design line, replacing the line any
    /// earlier run made. The least-cost re-route needs a terrain sampler (a DEM) and a water level.
    /// The method and its settings are stored with the line; nothing changes when the stage refuses.
    pub fn align(
        &mut self,
        method: AlignMethod,
        overrides: &Value,
        terrain: Option<TerrainSampler>,
    ) -> Result<(), DesignError> {
        self.upstream_ready()?;
        let mut settings = self.method_settings(method, overrides)?;
        settings.method = method;
        let input = self.alignment_input(settings.clone())?;
        // The multiplier raster, when the design names one and it is attached; a named raster that
        // is not attached reaches the stage as none, and the re-route refuses by name.
        let raster = self.stage_multiplier().ok().flatten();
        let weight = match &raster {
            Some(r) => Some(
                super::constraints::multiplier_sampler(r, self.multiplier_epsg())
                    .map_err(stage_error)?,
            ),
            None => None,
        };
        let line = super::design_line::alignment_with(
            &input,
            terrain,
            weight.as_ref().map(|w| w as TerrainSampler),
        )
        .map_err(stage_error)?;
        self.store_design_line(&settings, method, &line);
        Ok(())
    }

    /// Store the alignment's settings and its design line, with the hash of what it read.
    pub(super) fn store_design_line(
        &mut self,
        settings: &AlignmentSettings,
        method: AlignMethod,
        line: &DesignLine,
    ) {
        let st = &mut self.doc["stages"]["alignment"];
        if !st["settings"].is_object() {
            st["settings"] = json!({});
        }
        st["settings"]["method"] = json!(method.name());
        if method != AlignMethod::Given {
            // The method's settings as set: the defaults filled in, a setting left unset still null.
            st["settings"][method.name()] = settings.part(method);
        }
        match &settings.given {
            Some(g) => st["settings"]["given"] = json!(g),
            None => {
                if let Some(o) = st["settings"].as_object_mut() {
                    o.remove("given");
                }
            }
        }
        st["results"] = serde_json::to_value(line).expect("a design line is JSON");
        st["engine_version"] = json!(ENGINE_VERSION);
        let h = self.dependency_hash(Stage::Alignment);
        self.doc["stages"]["alignment"]["depends_on"] = json!(h);
    }

    /// `run_stage(Alignment)`: run the method the settings name (the rolling circle by default). A
    /// re-route needs a DEM: without terrain it is refused.
    pub(super) fn rerun_alignment(
        &mut self,
        terrain: Option<TerrainSampler>,
    ) -> Result<(), DesignError> {
        let method = self.alignment_settings()?.method;
        if method == AlignMethod::Reroute && terrain.is_none() {
            return Err(DesignError::NotAvailable(
                "the re-route needs a DEM: rerun it with `owd align --method reroute --dem FILE`"
                    .into(),
            ));
        }
        self.align(method, &Value::Null, terrain)
    }

    /// The river polygon(s), with holes, when the inputs have one. The one place the document's
    /// polygon is read ([`RiverPolygon::from_geometry`]); every stage that needs it asks here.
    pub(super) fn river_polygons(&self) -> Option<Vec<Polygon>> {
        RiverPolygon::from_geometry(&self.doc["inputs"]["river_polygon"])
            .map(|p| vec![p.to_polygon()])
    }
}

/// The text every export of a water-aware line carries (spec 0005).
pub const WIDTH_ONLY: &str = "width only, no depth data";

/// What a line whose every bend is infeasible says (issue #366). Its line is the fit that breaks
/// the rules least, not a design: the volumes stage does not cost it.
pub const NO_FEASIBLE_LINE: &str = "no feasible line";
