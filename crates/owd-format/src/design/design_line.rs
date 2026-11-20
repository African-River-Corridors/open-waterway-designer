//! The alignment stage (spec 0017 ticket 6, ADR 0015): one design line per run.
//!
//! [`alignment`] is a pure function from an [`AlignmentIn`] to a [`DesignLine`]. It reads no
//! document and no file: the river comes in as the river stage's [`RiverGeometry`], the size as the
//! sizing stage's [`ChannelSize`], and the method is a setting. The document stores the one line
//! under `stages.alignment.results`; there is no list of options to choose from (ADR 0015, 8).
//!
//! A design line records its bends (chainage from and to, least radius, R_min, pass) and its
//! cut-offs (the stretches where it leaves the river's water, with the river chainage it leaves and
//! rejoins at). Chainage is along the line from its first point, which is the mouth end when the
//! design states a mouth.
//!
//! The pool lattice of the optimise stage also makes a [`DesignLine`] (method `lattice`). When it
//! has run, its line is the design line the later stages read ([`design_lines_of`]); the alignment's
//! line stays stored as what the lattice started from.

use super::alignment::{AlignMethod, AlignmentSettings, NO_FEASIBLE_LINE, WIDTH_ONLY};
use super::river::RiverGeometry;
use super::sizing::ChannelSize;
use crate::types::{Chainage, Polyline, StageError, TerrainSampler};
use owd_core::alignment::fairway_fit::{Fixed, Pose};
use owd_core::alignment::join::{JoinParams, join_legs};
use owd_core::alignment::limits::compute_chainages;
use owd_core::alignment::navigable::Stretch;
use owd_core::alignment::radius_check::{
    MAX_STRIDE_M, RADIUS_TOL, default_check_chord, min_radius_on_chord, radius_ok,
};
use owd_core::alignment::rolling_circle::{WaterInput, rolling_circle_with_water};
use owd_core::alignment::route::{self, GridParams, RouteParams};
use owd_core::geom::overlay::point_polygon_distance;
use owd_core::geom::{Geometry, Polygon, Pt, linear};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The stage's name, as the document stores it.
pub const STAGE: &str = "alignment";

/// A stretch of line whose radius is under this many times R_min is reported as a bend. It is OWD's
/// own reporting choice, not a figure from a standard: a gentler curve is read as straight.
pub const BEND_RADIUS_FACTOR: f64 = 5.0;

/// The step, in metres, at which a design line is tested for being in the water.
pub const CUT_OFF_STEP_M: f64 = 5.0;

/// A point of a design line closer than this to the river's water (metres) counts as in it, so a
/// line that runs along a bank is not read as leaving the river. OWD's own choice.
pub const CUT_OFF_TOLERANCE_M: f64 = 1.0;

/// A line the user already has (ticket C21), given as the design line: its name and its points in
/// the project CRS.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GivenLine {
    /// The name the user gave it.
    pub name: String,
    /// The line, in the project CRS: a GeoJSON `LineString`.
    #[serde(with = "line_string")]
    #[schemars(with = "line_string::LineString")]
    pub line: Polyline,
}

/// A pose: a point and a heading in degrees counter-clockwise from the project CRS's +x axis.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PoseIn {
    /// The point.
    pub point: [f64; 2],
    /// The heading in degrees.
    pub heading_deg: f64,
}

/// Seed legs (spec 0005): lines joined first, with pieces the joined line must hold.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct SeedLegs {
    /// The legs, in order.
    pub legs: Vec<Polyline>,
    /// The pose the joined line must start with.
    #[serde(default)]
    pub start_pose: Option<PoseIn>,
    /// The pose the joined line must end with.
    #[serde(default)]
    pub end_pose: Option<PoseIn>,
    /// Lines the joined line must follow exactly.
    #[serde(default)]
    pub must_follow: Vec<Polyline>,
}

/// What the alignment stage reads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AlignmentIn {
    /// The river stage's output: the centreline (from the mouth when one is stated), the river
    /// polygon with its islands, and the water level profile on river chainage.
    pub river: RiverGeometry,
    /// The sizing stage's output.
    pub size: ChannelSize,
    /// The method and the settings of each method.
    pub settings: AlignmentSettings,
    /// Seed legs to join first, for the rolling circle.
    #[serde(default)]
    pub seed: Option<SeedLegs>,
}

/// One bend of a design line.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Bend {
    /// Design-line chainage where the bend starts.
    pub from: Chainage,
    /// Design-line chainage where the bend ends.
    pub to: Chainage,
    /// The least radius in the bend, in metres, read the way the radius check reads it.
    pub radius_m: f64,
    /// The minimum radius the sizing allows, in metres.
    pub r_min_m: f64,
    /// True when the radius meets R_min (within the radius check's tolerance).
    pub pass: bool,
}

/// One stretch where the design line leaves the river's water: a cut-off of the river reach between.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CutOff {
    /// Design-line chainage where the line leaves the water.
    pub from: Chainage,
    /// Design-line chainage where it comes back.
    pub to: Chainage,
    /// River chainage where it leaves the river.
    pub river_from: Chainage,
    /// River chainage where it rejoins the river.
    pub river_to: Chainage,
}

/// What the alignment stage returns: the one design line, how it was made and what it does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DesignLine {
    /// A short id: `rolling-circle`, `reroute`, `centreline`, `lattice` or `given-<name>`.
    pub id: String,
    /// A name for people.
    pub name: String,
    /// The method: `rolling_circle`, `reroute`, `centreline`, `given` or `lattice`.
    pub method: String,
    /// The method's settings, as used.
    pub settings: Value,
    /// The line, from the mouth when the design states one: a GeoJSON `LineString` in the project
    /// CRS, as every line of the document is.
    #[serde(with = "line_string")]
    #[schemars(with = "line_string::LineString")]
    pub line: Polyline,
    /// The checks of the line: `r_min_m`, `radius_ok`, `measured_min_radius_m`,
    /// `radius_check_chord_m`, and `fits_in_water`, `feasible` and `join_guard` where they apply.
    pub checks: Value,
    /// What the method reports about its own work.
    #[serde(default)]
    pub details: Value,
    /// The design sanity check of the line: `{checked, warnings}`.
    #[serde(default)]
    pub sanity: Value,
    /// How a bend of this line widens, when not by the rules (`none` for the river centreline).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub widening_floor: Option<String>,
    /// The bends, in chainage order.
    pub bends: Vec<Bend>,
    /// Where the line leaves the river's water. `None` when the design has no river polygon to tell
    /// water from land.
    pub cut_offs: Option<Vec<CutOff>>,
    /// The names of the constraints the line touches. Empty until constraints exist (ticket 13).
    pub constraints_touched: Vec<String>,
}

/// A [`Polyline`] written as a GeoJSON `LineString`, as the document writes every line.
mod line_string {
    use crate::types::Polyline;
    use schemars::JsonSchema;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    /// A GeoJSON `LineString`.
    #[derive(Serialize, Deserialize, JsonSchema)]
    pub struct LineString {
        /// Always `LineString`.
        #[serde(rename = "type")]
        pub kind: String,
        /// The points.
        pub coordinates: Vec<[f64; 2]>,
    }

    pub fn serialize<S: Serializer>(p: &Polyline, s: S) -> Result<S::Ok, S::Error> {
        LineString {
            kind: "LineString".into(),
            coordinates: p.0.clone(),
        }
        .serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Polyline, D::Error> {
        let l = LineString::deserialize(d)?;
        if l.kind != "LineString" {
            return Err(serde::de::Error::custom(format!(
                "a design line is a LineString, not a {}",
                l.kind
            )));
        }
        Ok(Polyline(l.coordinates))
    }
}

fn refuse(code: &str, message: impl Into<String>) -> StageError {
    StageError::new(STAGE, code, &message.into())
}

impl DesignLine {
    /// The line as a JSON value, as the document stores it.
    pub fn to_view(&self) -> Value {
        serde_json::to_value(self).expect("a design line is JSON")
    }

    /// The radius check of the line, with the chord it was read on.
    pub fn r_min_m(&self) -> f64 {
        self.checks["r_min_m"].as_f64().unwrap_or(f64::NAN)
    }

    /// A design line from an option of a 0.4 document. Its bends and cut-offs were never worked out:
    /// they are left empty and unknown, and the stage that holds it is marked stale on open.
    pub fn from_old_option(o: &Value) -> Option<DesignLine> {
        let line: Vec<Pt> = super::geojson::all_positions(&o["line"]);
        if line.len() < 2 {
            return None;
        }
        let text = |k: &str| o[k].as_str().unwrap_or_default().to_string();
        Some(DesignLine {
            id: text("id"),
            name: text("name"),
            method: text("method"),
            settings: o["settings"].clone(),
            line: Polyline(line),
            checks: o["checks"].clone(),
            details: o.get("details").cloned().unwrap_or(json!({})),
            sanity: o.get("sanity").cloned().unwrap_or(Value::Null),
            widening_floor: o["widening_floor"].as_str().map(str::to_string),
            bends: Vec::new(),
            cut_offs: None,
            constraints_touched: Vec::new(),
        })
    }
}

/// The design line a document holds, read from the document alone (see [`DesignLine::to_view`]):
/// the pool lattice's line when the optimise stage has made one, else the alignment's. It cannot
/// tell whether the optimise stage is stale: the stages, the outputs and the surfaces read
/// [`Session::design_lines`](super::session::Session::design_lines), which can.
pub fn design_lines_of(doc: &Value) -> Vec<Value> {
    final_line_of(doc)
        .map(|l| l.to_view())
        .into_iter()
        .collect()
}

/// The design line the later stages read, typed. See [`design_lines_of`].
pub fn final_line_of(doc: &Value) -> Option<DesignLine> {
    let refined = &doc["stages"]["optimise"]["results"]["design_line"];
    let line = if refined.is_object() {
        refined
    } else {
        &doc["stages"]["alignment"]["results"]
    };
    line.is_object()
        .then(|| serde_json::from_value(line.clone()).ok())
        .flatten()
}

/// The alignment stage's own line (not the lattice's). `None` before it has run.
pub fn alignment_line_of(doc: &Value) -> Option<DesignLine> {
    let line = &doc["stages"]["alignment"]["results"];
    line.is_object()
        .then(|| serde_json::from_value(line.clone()).ok())
        .flatten()
}

impl super::session::Session {
    /// The design line the later stages read: the pool lattice's refined line while the optimise
    /// stage is fresh, else the alignment's own (QA on PR 558: a stale lattice line is never read).
    /// `None` before the alignment has run.
    pub fn final_line(&self) -> Option<DesignLine> {
        use super::session::{Stage, StageStatus};
        let refined = &self.doc["stages"]["optimise"]["results"]["design_line"];
        if refined.is_object() && self.stage_status(Stage::Optimise) == StageStatus::Fresh {
            return serde_json::from_value(refined.clone()).ok();
        }
        alignment_line_of(&self.doc)
    }

    /// [`Session::final_line`] in the shape the readers of an option use: at most one.
    pub fn design_lines(&self) -> Vec<Value> {
        self.final_line().map(|l| l.to_view()).into_iter().collect()
    }

    /// Why the pool lattice's line is not the design line now, when the optimise stage holds one
    /// but is stale. `owd status` prints it.
    pub fn stale_refined_line_note(&self) -> Option<String> {
        use super::session::{Stage, StageStatus};
        let refined = &self.doc["stages"]["optimise"]["results"]["design_line"];
        (refined.is_object() && self.stage_status(Stage::Optimise) == StageStatus::Stale).then(|| {
            "the pool lattice's line is stale (the alignment, the inputs or the settings changed \
             since `owd optimise` ran): the later stages read the alignment's line until \
             `owd optimise` (or `owd run`) runs again"
                .to_string()
        })
    }
}

/// The volumes of the design line, as rows the readers of the volumes results use: at most one.
pub fn volume_rows_of(doc: &Value) -> Vec<Value> {
    let r = &doc["stages"]["volumes"]["results"];
    (r.is_object() && r.get("id").is_some())
        .then(|| r.clone())
        .into_iter()
        .collect()
}

/// A run of stations whose radius is under a limit: `(from, to, chainage of the least radius,
/// least radius, its point)`, in chainage order. The radius is read the way the radius check reads
/// it: three points a half chord apart, each step at most [`MAX_STRIDE_M`].
pub(crate) fn radius_runs(pts: &[Pt], chord: f64, limit: f64) -> Vec<(f64, f64, f64, f64, Pt)> {
    if pts.len() < 2 || !(chord.is_finite() && chord > 0.0 && limit.is_finite()) {
        return Vec::new();
    }
    let chain = compute_chainages(pts);
    let half = chord / 2.0;
    let k = ((half / MAX_STRIDE_M).ceil() as usize).max(1);
    let h = half / k as f64;
    let total = *chain.last().unwrap_or(&0.0);
    let n = (total / h).floor() as usize;
    let s: Vec<(f64, Pt)> = (0..=n)
        .map(|i| {
            let c = i as f64 * h;
            (c, linear::interpolate(pts, c).unwrap_or([f64::NAN; 2]))
        })
        .collect();
    let mut out = Vec::new();
    let mut run: Option<(f64, f64, f64, f64, Pt)> = None;
    for i in k..s.len().saturating_sub(k) {
        let r = super::map::circumradius(s[i - k].1, s[i].1, s[i + k].1);
        if r < limit {
            run = Some(match run {
                None => (s[i].0, s[i].0, s[i].0, r, s[i].1),
                Some((a, _, at, rr, p)) if rr <= r => (a, s[i].0, at, rr, p),
                Some((a, _, _, _, _)) => (a, s[i].0, s[i].0, r, s[i].1),
            });
        } else if let Some(x) = run.take() {
            out.push(x);
        }
    }
    out.extend(run);
    out
}

/// The bends of a line against `r_min`: each stretch whose radius, read on `chord`, is under
/// [`BEND_RADIUS_FACTOR`] times R_min, with its least radius and whether that meets R_min.
pub fn bends(line: &[Pt], r_min: f64, chord: f64) -> Vec<Bend> {
    if !(r_min.is_finite() && r_min > 0.0) {
        return Vec::new();
    }
    let tight = (1.0 - RADIUS_TOL) * r_min;
    radius_runs(line, chord, BEND_RADIUS_FACTOR * r_min)
        .into_iter()
        .map(|(a, b, _, r, _)| Bend {
            from: Chainage(a),
            to: Chainage(b),
            radius_m: r,
            r_min_m: r_min,
            pass: r >= tight,
        })
        .collect()
}

/// The stretches of `line` outside the river's water (islands count as land), each with the river
/// chainage, on `centre`, where it leaves and rejoins. `None` with no water to test against.
pub fn cut_offs(line: &[Pt], water: &[Polygon], centre: &[Pt]) -> Option<Vec<CutOff>> {
    if water.iter().all(|p| p.exterior.len() < 3) || line.len() < 2 {
        return None;
    }
    let total = linear::length(line);
    let n = (total / CUT_OFF_STEP_M).ceil().max(1.0) as usize;
    let at = |c: f64| linear::interpolate(line, c).unwrap_or([f64::NAN; 2]);
    let dry = |c: f64| point_polygon_distance(at(c), water) > CUT_OFF_TOLERANCE_M;
    let river = |c: f64| {
        if centre.len() >= 2 {
            linear::project(centre, at(c))
        } else {
            f64::NAN
        }
    };
    let mut out = Vec::new();
    let mut start: Option<f64> = None;
    let mut last = 0.0;
    for i in 0..=n {
        let c = (i as f64 * total / n as f64).min(total);
        match (dry(c), start) {
            (true, None) => start = Some(c),
            (false, Some(a)) => {
                out.push((a, last));
                start = None;
            }
            _ => {}
        }
        last = c;
    }
    if let Some(a) = start {
        out.push((a, total));
    }
    Some(
        out.into_iter()
            .map(|(a, b)| {
                let (ra, rb) = (river(a), river(b));
                CutOff {
                    from: Chainage(a),
                    to: Chainage(b),
                    river_from: Chainage(ra.min(rb)),
                    river_to: Chainage(ra.max(rb)),
                }
            })
            .collect(),
    )
}

/// The design sanity check of a line (`owd_core::sanity`): `{"checked": true, "warnings": [...]}`.
pub fn sanity(line: &[Pt], chord: f64, r_min: f64) -> Value {
    let warnings: Vec<String> = owd_core::sanity::line_findings(line, chord, r_min)
        .iter()
        .map(owd_core::sanity::Finding::message)
        .collect();
    json!({"checked": true, "warnings": warnings})
}

/// The radius check of a line, as every design line has it.
pub fn radius_checks(line: &[Pt], r_min: f64, chord: Option<f64>) -> Result<Value, StageError> {
    let chord = chord.unwrap_or_else(|| default_check_chord(r_min));
    let measured =
        min_radius_on_chord(line, chord).map_err(|e| refuse("ALIGN_RADIUS_CHECK", e.message))?;
    Ok(json!({
        "r_min_m": r_min,
        "radius_ok": radius_ok(measured, r_min),
        "measured_min_radius_m": measured,
        "radius_check_chord_m": chord,
    }))
}

/// A design line from its parts: the bends, cut-offs and sanity check are worked out here, the same
/// way for every method.
#[allow(clippy::too_many_arguments)]
pub fn finish(
    id: &str,
    name: &str,
    method: &str,
    settings: Value,
    line: Vec<Pt>,
    checks: Value,
    details: Value,
    river: &RiverGeometry,
) -> DesignLine {
    let r_min = checks["r_min_m"].as_f64().unwrap_or(f64::NAN);
    let chord = checks["radius_check_chord_m"]
        .as_f64()
        .unwrap_or_else(|| default_check_chord(r_min));
    let water: Vec<Polygon> = river
        .polygon
        .as_ref()
        .map(|p| vec![p.to_polygon()])
        .unwrap_or_default();
    DesignLine {
        id: id.to_string(),
        name: name.to_string(),
        method: method.to_string(),
        settings,
        sanity: sanity(&line, chord, r_min),
        bends: bends(&line, r_min, chord),
        cut_offs: if water.is_empty() {
            None
        } else {
            cut_offs(&line, &water, &river.centreline.0)
        },
        constraints_touched: Vec::new(),
        widening_floor: None,
        details,
        checks,
        line: Polyline(line),
    }
}

/// A stretch as the document records it.
fn stretch_json(line: &[Pt], s: &Stretch) -> Value {
    let (a, b) = (linear::project(line, s.start), linear::project(line, s.end));
    json!({
        "start_chainage_m": a.min(b), "end_chainage_m": a.max(b),
        "start": s.start, "end": s.end, "reason": s.reason.as_str(),
    })
}

fn pose(p: &Option<PoseIn>) -> Option<Pose> {
    p.map(|p| Pose {
        at: p.point,
        heading_rad: p.heading_deg.to_radians(),
    })
}

/// The alignment stage: the one design line of the method the settings name.
///
/// The re-route needs `terrain` and a water level profile in the river geometry. A line under R_min
/// is not refused: its checks and its bends say so (issue #367).
pub fn alignment(
    input: &AlignmentIn,
    terrain: Option<TerrainSampler>,
) -> Result<DesignLine, StageError> {
    let r_min = input.size.min_bend_radius_m;
    let method = input.settings.method;
    match method {
        AlignMethod::Given => given(input, r_min),
        AlignMethod::Centreline => {
            let s = &input.settings.centreline;
            let line = input.river.centreline.0.clone();
            let checks = radius_checks(&line, r_min, s.check_chord_m)?;
            let mut d = finish(
                method.option_id(),
                method.option_name(),
                method.name(),
                serde_json::to_value(s).expect("settings are JSON"),
                line,
                checks,
                json!({}),
                &input.river,
            );
            // Its bends widen by their own radius.
            d.widening_floor = Some(owd_core::cut_model::WideningFloor::None.name().to_string());
            Ok(d)
        }
        AlignMethod::RollingCircle => rolling_circle(input, r_min),
        AlignMethod::Reroute => reroute(input, r_min, terrain),
    }
}

fn given(input: &AlignmentIn, r_min: f64) -> Result<DesignLine, StageError> {
    let g = input.settings.given.as_ref().ok_or_else(|| {
        refuse(
            "ALIGN_NO_LINE",
            "the method is `given` but there is no given line: give one with `owd design-line`",
        )
    })?;
    if g.line.len() < 2 {
        return Err(refuse(
            "ALIGN_NO_LINE",
            "the design line needs at least two points",
        ));
    }
    let id = given_id(&g.name).ok_or_else(|| {
        refuse(
            "ALIGN_NO_LINE",
            "a given design line needs a name with a letter or a digit",
        )
    })?;
    let line = g.line.0.clone();
    let checks = radius_checks(&line, r_min, None)?;
    Ok(finish(
        &id,
        &g.name,
        AlignMethod::Given.name(),
        json!({}),
        line,
        checks,
        json!({}),
        &input.river,
    ))
}

/// `given-` and the name in lower case, each run of characters that are not ASCII letters or
/// digits one `-`, and no `-` at either end. `None` when the name has no letter or digit.
pub fn given_id(name: &str) -> Option<String> {
    let mut slug = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-');
    (!slug.is_empty()).then(|| format!("given-{slug}"))
}

fn rolling_circle(input: &AlignmentIn, r_min: f64) -> Result<DesignLine, StageError> {
    let core = |e: owd_core::alignment::PyError| refuse("ALIGN_NO_ROUTE", e.message);
    let s = &input.settings.rolling_circle;
    let water: Option<Vec<Polygon>> = input.river.polygon.as_ref().map(|p| vec![p.to_polygon()]);
    // Unset: keep to the water whenever there is a river polygon, given or built.
    let use_water = s.use_water.unwrap_or(water.is_some());
    let water = if use_water {
        Some(water.ok_or_else(|| {
            refuse(
                "ALIGN_NO_WATER",
                "`use_water` is on but there is no river polygon: import one, or set it off",
            )
        })?)
    } else {
        None
    };
    let shape = input.size.shape()?;
    let mut checks = serde_json::Map::new();
    let mut join_stretches: Vec<Stretch> = Vec::new();
    let base = match input.seed.as_ref().filter(|s| !s.legs.is_empty()) {
        None => input.river.centreline.0.clone(),
        Some(seed) => {
            let legs: Vec<Vec<Pt>> = seed.legs.iter().map(|l| l.0.clone()).collect();
            let fixed = Fixed {
                start_pose: pose(&seed.start_pose),
                end_pose: pose(&seed.end_pose),
                must_follow: seed.must_follow.iter().map(|l| l.0.clone()).collect(),
            };
            // Without water the legs are joined in open space: a box well round them.
            let open = || {
                let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
                for p in legs.iter().flatten() {
                    for k in 0..2 {
                        lo[k] = lo[k].min(p[k]);
                        hi[k] = hi[k].max(p[k]);
                    }
                }
                let m = 10.0 * r_min;
                vec![Polygon {
                    exterior: vec![
                        [lo[0] - m, lo[1] - m],
                        [hi[0] + m, lo[1] - m],
                        [hi[0] + m, hi[1] + m],
                        [lo[0] - m, hi[1] + m],
                        [lo[0] - m, lo[1] - m],
                    ],
                    holes: vec![],
                }]
            };
            let w = water.clone().unwrap_or_else(open);
            let mut jp = JoinParams::new(r_min);
            jp.clearance_m = s.clearance_m;
            let j = join_legs(&legs, &w, shape, &fixed, &jp).map_err(core)?;
            checks.insert(
                "join_guard".into(),
                json!({"ok": j.guard_failed.is_empty(), "joins": j.guard_failed}),
            );
            join_stretches = j.infeasible.clone();
            j.line
        }
    };
    let wi = water.as_ref().map(|w| WaterInput {
        water: w,
        shape,
        clearance_m: s.clearance_m,
        smooth_m: 0.0,
        r_pref_m: s.r_pref_m,
    });
    let r = rolling_circle_with_water(
        &Geometry::LineString(base),
        r_min,
        s.step_m,
        s.passes,
        &["left".to_string(), "right".to_string()],
        None,
        s.check_chord_m,
        wi.as_ref(),
    )
    .map_err(core)?;
    let line = r.result.centreline.clone();
    let mut all = json!({
        "r_min_m": r_min,
        "radius_ok": r.result.radius_ok,
        "measured_min_radius_m": r.result.measured_min_radius_m,
        "radius_check_chord_m": r.result.radius_check_chord_m,
    });
    if wi.is_some() {
        let stretches: Vec<Value> = r
            .infeasible
            .iter()
            .chain(&join_stretches)
            .map(|st| stretch_json(&line, st))
            .collect();
        checks.insert(
            "fits_in_water".into(),
            json!({"ok": stretches.is_empty(), "stretches": stretches, "note": WIDTH_ONLY}),
        );
        // Issue #366: every bend infeasible is no feasible line, not a design.
        let mut feasible = json!({"ok": r.feasible});
        if !r.feasible {
            feasible["note"] = json!(NO_FEASIBLE_LINE);
        }
        checks.insert("feasible".into(), feasible);
    }
    for (k, v) in checks {
        all[k] = v;
    }
    // The line records the value used, also when it came from the default.
    let mut used = serde_json::to_value(s).expect("settings are JSON");
    used["use_water"] = json!(use_water);
    let mut details = json!({"arcs": r.result.circles.len()});
    if wi.is_some() {
        details["rungs"] = json!(r.rungs.iter().map(|b| b.rung.as_str()).collect::<Vec<_>>());
    }
    let m = AlignMethod::RollingCircle;
    Ok(finish(
        m.option_id(),
        m.option_name(),
        m.name(),
        used,
        line,
        all,
        details,
        &input.river,
    ))
}

fn reroute(
    input: &AlignmentIn,
    r_min: f64,
    terrain: Option<TerrainSampler>,
) -> Result<DesignLine, StageError> {
    let s = &input.settings.reroute;
    let terrain = terrain.ok_or_else(|| {
        refuse(
            "ALIGN_NO_TERRAIN",
            "the re-route needs terrain: give it a DEM",
        )
    })?;
    let river = input
        .river
        .polygon
        .as_ref()
        .map(|p| vec![p.to_polygon()])
        .ok_or_else(|| {
            refuse(
                "ALIGN_NO_WATER",
                "the re-route needs the river polygon: import it first",
            )
        })?;
    let profile = input
        .river
        .water_profile
        .as_ref()
        .filter(|p| !p.is_empty())
        .ok_or_else(|| {
            refuse(
                "ALIGN_NO_WATER",
                "the re-route needs a water level: set one with `owd import --water-level LEVEL` (a constant), `--water-slope` or `--water-table FILE`",
            )
        })?;
    let fairness = match s.fairness_weight {
        None => 0.0,
        Some(w) if w.is_finite() && w >= 0.0 => w,
        Some(_) => {
            return Err(refuse(
                "ALIGN_SETTING",
                "the setting `fairness_weight` must be a number at or above 0",
            ));
        }
    };
    let depth = input.size.design_depth_m;
    // The re-route starts from the river centreline: one design line per run, so there is no
    // earlier line of another method to start from.
    let centre = &input.river.centreline.0;
    let start = centre.clone();
    let widest = s
        .corridor_sweep_m
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    let params = GridParams {
        bottom_width: input.size.bottom_width_straight_m,
        depth,
        side_slope: s.side_slope,
        corridor_half: widest,
        station_step: s.station_step_m,
        offset_step: s.offset_step_m,
        transect_step: 2.0,
        daylight_m: None,
    };
    let level = |c: f64| {
        let at = linear::interpolate(&start, c).unwrap_or([f64::NAN; 2]);
        profile.level_at(Chainage(linear::project(centre, at)))
    };
    let grade = move |c: f64| level(c) - depth;
    let grid = route::build_cost_grid(&start, terrain, &river, &grade, &params)
        .map_err(|e| refuse("ALIGN_NO_ROUTE", e.message))?;
    let r = route::optimise_route(
        &grid,
        &RouteParams {
            r_min,
            wet_weight: s.wet_weight,
            dry_weight: s.dry_weight,
            corridor_sweep: s.corridor_sweep_m.clone(),
            check_chord_m: s.check_chord_m,
            fairness_weight: fairness,
        },
    )
    .map_err(|e| refuse("ALIGN_NO_ROUTE", e.message))?;
    let m = AlignMethod::Reroute;
    Ok(finish(
        m.option_id(),
        m.option_name(),
        m.name(),
        serde_json::to_value(s).expect("settings are JSON"),
        r.line.clone(),
        json!({
            "r_min_m": r_min,
            "radius_ok": r.radius_ok,
            "measured_min_radius_m": r.measured_min_radius_m,
            "radius_check_chord_m": r.radius_check_chord_m,
        }),
        json!({
            "corridor_half_m": r.corridor_half_m,
            "approx_saving_pct": r.approx_saving_pct,
        }),
        &input.river,
    ))
}
