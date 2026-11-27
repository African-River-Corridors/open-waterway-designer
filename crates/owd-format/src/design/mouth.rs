//! The river mouth in the document (spec 0017, ticket 4 carried into ticket 6): where chainage 0 is.
//!
//! A design states its mouth with [`Session::set_mouth`]: a point (longitude and latitude), or the
//! word `downstream` with a water level profile or an elevation model to say which end is lower.
//! The river stage ([`super::river::river`]) decides which end of the centreline is the mouth. When
//! it is the far end, every river input is turned round once, here, so that the stored lines run from
//! the mouth and every stored river chainage (the water level, the bed depths, the waterway reaches,
//! the low-confidence stretches) counts from it. The mouth is stored as the point it resolved to
//! (`inputs.mouth`), so a later import keeps the same orientation without an elevation model.
//!
//! Left and right still name the banks looking downstream. A line stored from the mouth runs
//! upstream, so the river stage gives the bank checks the banks the other way round.
//!
//! A design with no mouth keeps its lines as they were given, and chainage runs from the first point
//! of the stored centreline, as before. A document from format 0.4 records no mouth: on open it is
//! worked out from the water level, else from the elevation model, and the design is turned with its
//! stored results so that the stages that were fresh stay fresh ([`Session::old_file_mouth`]). When
//! neither can tell, the design is not turned, and the notes say to state the mouth.

use super::river::{Mouth, RiverIn, river};
use super::session::{DesignError, OpenNote, Session};
use crate::types::TerrainSampler;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// How the user stated the mouth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MouthStatedAs {
    /// A point the user gave.
    Point,
    /// The lower end of the river, by the water level or the ground.
    Downstream,
}

/// The mouth as the document stores it (`inputs.mouth`): the first point of the stored centreline,
/// in the project CRS, and how the user stated it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StoredMouth {
    /// The mouth, in the project CRS.
    pub point: [f64; 2],
    /// How the user stated it.
    pub stated_as: MouthStatedAs,
}

/// The mouth as a user states it, before it is resolved.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MouthAt {
    /// Longitude and latitude (WGS84).
    LonLat([f64; 2]),
    /// The lower end of the river.
    Downstream,
}

/// Read a mouth as the command line and the app give it: `downstream`, or `LON,LAT`. The one
/// reader of the text for every surface.
pub fn parse_mouth(text: &str) -> Result<MouthAt, DesignError> {
    let t = text.trim();
    if t.eq_ignore_ascii_case("downstream") {
        return Ok(MouthAt::Downstream);
    }
    let bad = || {
        DesignError::Input(format!(
            "the mouth must be `downstream` or `LON,LAT` (for example -1.5,5.2), not `{t}`"
        ))
    };
    let nums: Vec<f64> = t
        .split(',')
        .map(|p| p.trim().parse::<f64>())
        .collect::<Result<_, _>>()
        .map_err(|_| bad())?;
    match nums[..] {
        [lon, lat] if lon.is_finite() && lat.is_finite() => {
            if lon.abs() <= 180.0 && lat.abs() <= 90.0 {
                Ok(MouthAt::LonLat([lon, lat]))
            } else {
                Err(DesignError::Input(format!(
                    "the mouth ({lon}, {lat}) is not a longitude and latitude"
                )))
            }
        }
        _ => Err(bad()),
    }
}

/// What stating the mouth changed.
#[derive(Debug, Clone, PartialEq)]
pub struct MouthReport {
    /// The mouth, in the project CRS.
    pub point: [f64; 2],
    /// True when the river inputs were turned round to run from the mouth.
    pub turned: bool,
    /// Plain sentences about anything else the turn changed.
    pub notes: Vec<String>,
}

/// A line stored as a GeoJSON `LineString`, turned round in place. Anything else (a null input)
/// is left as it is.
fn reverse_line(v: &mut Value) {
    if let Some(a) = v.get_mut("coordinates").and_then(Value::as_array_mut) {
        a.reverse();
    }
}

/// `[chainage, value]` points measured from the other end of a river `len` metres long.
fn turn_points(v: &mut Value, len: f64) {
    if let Some(a) = v.as_array_mut() {
        a.reverse();
        for p in a.iter_mut() {
            if let Some(c) = p[0].as_f64() {
                p[0] = json!(len - c);
            }
        }
    }
}

/// Stretches `{<from>, <to>}` measured from the other end of a river `len` metres long. A missing
/// or null end (`end_m` of a waterway reach) runs to the end of the river.
fn turn_stretches(v: &mut Value, from: &str, to: &str, len: f64) {
    let Some(a) = v.as_array_mut() else { return };
    a.reverse();
    for s in a.iter_mut() {
        let f = s[from].as_f64().unwrap_or(0.0);
        let t = s[to].as_f64().unwrap_or(len);
        s[from] = json!((len - t).max(0.0));
        // A waterway reach that started at the old start now runs to the end: null, as the input
        // says it.
        s[to] = if from == "start_m" && f <= 0.0 {
            Value::Null
        } else {
            json!(len - f)
        };
    }
}

impl Session {
    /// The mouth the document states, if any.
    pub fn stored_mouth(&self) -> Option<StoredMouth> {
        serde_json::from_value(self.doc["inputs"]["mouth"].clone()).ok()
    }

    /// Turn the three river lines round, and nothing else. For an import, which names the banks
    /// looking along the centreline, to see the lines run towards the mouth again.
    pub(super) fn turn_lines(&mut self) {
        for k in ["centreline", "left_bank", "right_bank"] {
            if let Some(v) = self.doc.pointer_mut(&format!("/inputs/{k}")) {
                reverse_line(v);
            }
        }
    }

    /// Turn every river input round: the lines, and every river chainage measured along them (the
    /// river is `len` metres long). Returns what it could not turn, in plain words.
    fn turn_river(&mut self, len: f64) -> Vec<String> {
        let mut notes = Vec::new();
        // The optimise stage's candidate weir sites are metres along its baseline (the line it ran
        // on, as it stored it; else the alignment's line, else the river centreline): counted from
        // the other end of that line.
        let baseline =
            super::geojson::all_positions(&self.doc["stages"]["optimise"]["results"]["baseline"]);
        let line_len = if baseline.len() >= 2 {
            owd_core::geom::linear::length(&baseline)
        } else {
            super::design_line::alignment_line_of(&self.doc).map_or(len, |l| l.line.length_m())
        };
        if let Some(sites) = self
            .doc
            .pointer_mut("/stages/optimise/settings/candidate_sites_m")
            .and_then(Value::as_array_mut)
        {
            for c in sites.iter_mut() {
                if let Some(x) = c.as_f64() {
                    *c = json!(line_len - x);
                }
            }
            sites.reverse();
        }
        self.turn_lines();
        // `pointer_mut`, not indexing: indexing would add every missing key as null.
        let doc = &mut self.doc;
        if let Some(v) = doc.pointer_mut("/inputs/water_level/points") {
            turn_points(v, len);
        }
        if let Some(v) = doc.pointer_mut("/inputs/bed_model/points") {
            turn_points(v, len);
        }
        if let Some(v) = doc.pointer_mut("/inputs/waterway_reaches") {
            turn_stretches(v, "start_m", "end_m", len);
        }
        if let Some(v) = doc.pointer_mut("/inputs/banks/low_confidence") {
            turn_stretches(v, "from_m", "to_m", len);
        }
        if let Some(v) = doc.pointer_mut("/low_confidence_width/stretches") {
            turn_stretches(v, "from_m", "to_m", len);
        }
        // The seed legs run the other way, in the other order; the poses change ends and face back.
        if let Some(legs) = doc
            .pointer_mut("/inputs/seed_legs")
            .and_then(Value::as_array_mut)
        {
            legs.reverse();
            legs.iter_mut().for_each(reverse_line);
        }
        if let Some(f) = doc.pointer_mut("/inputs/fixed").filter(|f| f.is_object()) {
            let o = f.as_object_mut().expect("checked above");
            let (a, b) = (o.remove("start_pose"), o.remove("end_pose"));
            for (k, p) in [("start_pose", b), ("end_pose", a)] {
                if let Some(mut p) = p {
                    if let Some(h) = p["heading_deg"].as_f64() {
                        p["heading_deg"] = json!((h + 180.0).rem_euclid(360.0));
                    }
                    o.insert(k.to_string(), p);
                }
            }
            if let Some(m) = o.get_mut("must_follow").and_then(Value::as_array_mut) {
                m.iter_mut().for_each(reverse_line);
            }
        }
        // The reference line is made from the banks with its own chainage: it is made again.
        if let Some(o) = doc["stages"].as_object_mut()
            && o.remove("reference_line").is_some()
        {
            notes.push(
                "the reference line ran from the other end, so it was removed: make it again with \
                 `owd reference-line`"
                    .to_string(),
            );
        }
        notes
    }

    /// State the river mouth: chainage 0 from now on. The river stage decides which end of the
    /// centreline it is; when it is the far end, every river input is turned round to run from it,
    /// and the stages that read them go stale. `terrain` is the elevation model, for `downstream`
    /// with no water level; the caller reads the file. Nothing changes when it is refused.
    pub fn set_mouth(
        &mut self,
        at: MouthAt,
        terrain: Option<TerrainSampler>,
    ) -> Result<MouthReport, DesignError> {
        let utm = match self.doc["project"]["crs"]["epsg"]
            .as_u64()
            .and_then(|c| super::crs::Crs::from_epsg(c as u32).ok())
        {
            Some(super::crs::Crs::Utm(u)) => u,
            _ => {
                return Err(DesignError::Input(
                    "a mouth needs the river: import the centreline or the banks first".into(),
                ));
            }
        };
        let mut input: RiverIn = self.river_input();
        // The bank checks are not under test here.
        input.river_polygon = None;
        let (mouth, stated_as) = match at {
            MouthAt::LonLat([lon, lat]) => (
                Mouth::Point(
                    utm.forward(lon, lat)
                        .map_err(|e| DesignError::Input(format!("the mouth: {e}")))?,
                ),
                MouthStatedAs::Point,
            ),
            MouthAt::Downstream => (Mouth::Downstream, MouthStatedAs::Downstream),
        };
        input.mouth = Some(mouth);
        if let (Some(t), Ok(line)) = (terrain, self.base_line())
            && let (Some(a), Some(b)) = (line.first(), line.last())
        {
            let z = t(&[a[0], b[0]], &[a[1], b[1]]);
            if z.len() == 2 {
                input.end_levels_m = Some([z[0], z[1]]);
            }
        }
        let g =
            river(&input).map_err(|e| DesignError::Input(format!("{}: {}", e.code, e.message)))?;
        let mut notes = Vec::new();
        if g.reversed {
            notes = self.turn_river(g.length_m);
        }
        let point = g.mouth.unwrap_or(g.centreline.0[0]);
        self.doc["inputs"]["mouth"] = json!(StoredMouth { point, stated_as });
        Ok(MouthReport {
            point,
            turned: g.reversed,
            notes,
        })
    }

    /// After an import into a design with a mouth: turn the three lines so that they run from the
    /// mouth again (an import leaves them running towards it). Chainage data is already from the
    /// mouth and is not touched.
    pub(super) fn lines_from_mouth(&mut self, mouth: StoredMouth) -> Result<(), DesignError> {
        let input = RiverIn {
            river_polygon: None,
            mouth: Some(Mouth::Point(mouth.point)),
            water_profile: None,
            ..self.river_input()
        };
        let g = river(&input).map_err(|e| DesignError::Input(e.message))?;
        if g.reversed {
            self.turn_lines();
        }
        Ok(())
    }
}

/// How the note begins that says a moved file waits for its elevation model to find its mouth.
const WAITS_FOR_DEM: &str =
    "no mouth is recorded and the water level does not say which end is lower";

/// The length of a stored GeoJSON line, in metres; 0 for anything else.
fn line_len(v: &Value) -> f64 {
    owd_core::geom::linear::length(&super::geojson::all_positions(v))
}

/// Stretches `{<from>, <to>}` measured from the other end of a line `len` metres long, in the other
/// order. The points named `start` and `end`, where a stretch has them, change places too. Unlike
/// [`turn_stretches`], an end is never read as open.
fn turn_spans(v: &mut Value, from: &str, to: &str, len: f64) {
    let Some(a) = v.as_array_mut() else { return };
    a.reverse();
    for s in a.iter_mut() {
        let (f, t) = (s[from].as_f64(), s[to].as_f64());
        if let (Some(f), Some(t)) = (f, t) {
            // Never below 0 by a rounding of the length.
            s[from] = json!((len - t).max(0.0));
            s[to] = json!((len - f).max(0.0));
        }
        if let Some(o) = s.as_object_mut()
            && o.contains_key("start")
            && o.contains_key("end")
        {
            let (a, b) = (o.remove("start"), o.remove("end"));
            o.insert("start".into(), b.unwrap_or(Value::Null));
            o.insert("end".into(), a.unwrap_or(Value::Null));
        }
    }
}

/// A chainage written in a sentence (`between 120 m and 480 m`, `near 300 m along the line`),
/// counted from the other end of a line `len` metres long. The text gives whole metres, so the
/// turned number is within a metre of the exact one.
fn turn_text(text: &str, len: f64) -> String {
    let num = |s: &str| s.trim().parse::<f64>().ok();
    let m = |x: f64| format!("{:.0}", (len - x).max(0.0));
    if let Some(i) = text.find("between ")
        && let Some(j) = text[i..].find(" m and ")
        && let Some(k) = text[i + j + 7..].find(" m")
    {
        let a = &text[i + 8..i + j];
        let b = &text[i + j + 7..i + j + 7 + k];
        if let (Some(a), Some(b)) = (num(a), num(b)) {
            return format!(
                "{}between {} m and {} m{}",
                &text[..i],
                m(b),
                m(a),
                &text[i + j + 7 + k + 2..]
            );
        }
    }
    if let Some(i) = text.find("near ")
        && let Some(j) = text[i..].find(" m along the line")
        && let Some(x) = num(&text[i + 5..i + j])
    {
        return format!("{}near {} m{}", &text[..i], m(x), &text[i + j + 2..]);
    }
    text.to_string()
}

/// A design line (as stored) turned to run from the other end: the line, and every chainage along
/// it that is not worked out again ([`Session::finish_moved_lines`] works out its bends, cut-offs
/// and sanity check). `baseline_len` is the length of the line a lattice's pools are counted on.
fn turn_design_line(v: &mut Value, baseline_len: f64) {
    if !v.is_object() {
        return;
    }
    let len = line_len(&v["line"]);
    reverse_line(&mut v["line"]);
    if let Some(s) = v.pointer_mut("/checks/fits_in_water/stretches") {
        turn_spans(s, "start_chainage_m", "end_chainage_m", len);
    }
    if let Some(s) = v.get_mut("constraints_touched") {
        turn_spans(s, "from", "to", len);
    }
    if let Some(p) = v.pointer_mut("/details/pools") {
        turn_spans(p, "start_m", "end_m", baseline_len);
    }
}

impl Session {
    /// True when this design was moved from format 0.4 and only its elevation model can say which
    /// end is the mouth, and the host has not given it yet: call [`Session::settle_old_mouth`] with
    /// the DEM once it is loaded.
    pub fn old_mouth_pending(&self) -> bool {
        self.old_mouth_pending
    }

    /// Work out the mouth of a design moved from format 0.4 from its elevation model (`terrain`, the
    /// DEM the document names, in the project CRS), and turn the design to run from it when it is
    /// the far end. Does nothing unless [`Session::old_mouth_pending`]. With no terrain, or one that
    /// cannot tell, nothing is turned and the note says to state the mouth. Returns the notes, which
    /// are added to [`Session::open_notes`] too.
    pub fn settle_old_mouth(&mut self, terrain: Option<TerrainSampler>) -> Vec<String> {
        if !self.old_mouth_pending {
            return Vec::new();
        }
        self.old_mouth_pending = false;
        // The note that said it waits for the elevation model gives way to what it found.
        self.notes.retain(|n| !n.text.starts_with(WAITS_FOR_DEM));
        let notes = self.old_file_mouth(terrain, false);
        self.notes.extend(notes.iter().cloned());
        notes.into_iter().map(|n| n.text).collect()
    }

    /// The mouth of a file moved from format 0.4, which records none (spec 0017): the end with the
    /// lower water level, else (with `terrain`) the lower end of the ground. When it is the far end,
    /// the river inputs are turned as [`Session::set_mouth`] turns them, and the stored results of
    /// every stage that was fresh are turned with them, so they stay fresh: the design lines, the
    /// optimise stage's baseline, pools, water profile and weirs, and the volumes' constraint
    /// stretches and warnings. When it is the first end, the mouth is recorded and nothing else
    /// changes. With `may_wait`, no terrain and a DEM named, it waits for the host
    /// ([`Session::old_mouth_pending`]).
    pub(super) fn old_file_mouth(
        &mut self,
        terrain: Option<TerrainSampler>,
        may_wait: bool,
    ) -> Vec<OpenNote> {
        // A design with no river has no end to orient.
        if self.stored_mouth().is_some() || self.base_line().is_err() {
            return Vec::new();
        }
        let state = "state the mouth with `owd import --mouth downstream` (or `--mouth LON,LAT`); \
                     chainage still runs from the first point of the stored centreline";
        let mut fresh: Vec<super::session::Stage> = super::session::Stage::ALL
            .into_iter()
            .filter(|s| self.fresh_as_sized(*s))
            .collect();
        let river_len = self.river_geometry().map(|g| g.length_m).unwrap_or(0.0);
        let bed_on_line = self.bed_model_value().is_some() && self.bed_axis_hash().is_some();
        let report = match self.set_mouth(MouthAt::Downstream, terrain) {
            Ok(r) => r,
            Err(_) if may_wait && terrain.is_none() && self.dem_path().is_some() => {
                self.old_mouth_pending = true;
                return vec![OpenNote::action(format!(
                    "{WAITS_FOR_DEM}: the mouth is worked out from the elevation model once it is \
                     loaded (the command line and the app load it beside the file); without it, \
                     {state}"
                ))];
            }
            Err(_) => {
                return vec![OpenNote::action(format!(
                    "no mouth is recorded, and neither the water level nor the elevation model \
                     says which end is lower, so the design is not turned: {state}"
                ))];
            }
        };
        let by = if self.input_water_profile().is_some_and(|p| {
            let (a, b) = (
                p.level_at(crate::types::Chainage(0.0)),
                p.level_at(crate::types::Chainage(river_len)),
            );
            (a - b).abs() > 1e-9
        }) {
            "the water level"
        } else {
            "the elevation model"
        };
        let mut notes = Vec::new();
        if !report.turned {
            notes.push(OpenNote::info(format!(
                "the mouth is the first end of the stored centreline (the lower end, by {by}); it \
                 is recorded, and nothing else changes"
            )));
        } else {
            notes.push(OpenNote::info(format!(
                "the mouth is the far end of the stored centreline (the lower end, by {by}), so the \
                 design is turned to run from it: the river, its inputs and the stored results \
                 now count chainage from the mouth"
            )));
            notes.extend(report.notes.into_iter().map(OpenNote::action));
            if bed_on_line && fresh.contains(&super::session::Stage::Volumes) {
                fresh.retain(|s| *s != super::session::Stage::Volumes);
                notes.push(OpenNote::action(
                    "the volumes read a bed centred on the reference line, which is made again \
                     from the mouth, so they are marked stale: run `owd volumes`"
                        .to_string(),
                ));
            }
            notes.extend(self.turn_results(river_len, &fresh));
        }
        self.finish_moved_lines(&fresh, report.turned);
        self.stamp_fresh(&fresh);
        notes
    }

    /// Turn the stored results of a design whose river was just turned (it is `river_len` metres
    /// long): every stage's, fresh or not, so no chainage is left counting from the old start.
    /// Returns what could not be turned exactly, in plain words.
    fn turn_results(&mut self, river_len: f64, fresh: &[super::session::Stage]) -> Vec<OpenNote> {
        use super::session::Stage;
        let mut notes = Vec::new();
        let doc = &mut self.doc;
        let baseline_len = line_len(&doc["stages"]["optimise"]["results"]["baseline"]);
        turn_design_line(&mut doc["stages"]["alignment"]["results"], baseline_len);
        let op = &mut doc["stages"]["optimise"];
        if op["results"].is_object() {
            let r = &mut op["results"];
            reverse_line(&mut r["baseline"]);
            if let Some(p) = r["pools"].as_array_mut() {
                p.reverse();
                for q in p.iter_mut() {
                    if let (Some(a), Some(b)) = (q[0].as_f64(), q[1].as_f64()) {
                        q[0] = json!((baseline_len - b).max(0.0));
                        q[1] = json!((baseline_len - a).max(0.0));
                    }
                }
            }
            if let Some(l) = r["pools"][0][2].as_f64() {
                r["base_level_m"] = json!(l);
            }
            turn_points(&mut r["profile"], river_len);
            // A weir now sits between the same two pools the other way round: the pool after it
            // (from the mouth) is the one that was before it, and its lift changes sign.
            if let Some(w) = r["cascade"]["weirs"].as_array_mut() {
                w.reverse();
                for x in w.iter_mut() {
                    if let Some(c) = x["chainage_m"].as_f64() {
                        x["chainage_m"] = json!(baseline_len - c);
                    }
                    if let (Some(level), Some(lift)) =
                        (x["pool_level_m"].as_f64(), x["lift_m"].as_f64())
                    {
                        x["pool_level_m"] = json!(level - lift);
                        x["lift_m"] = json!(-lift);
                    }
                }
            }
            // The flooded land of each pool (written with the flood terms on): the same pools,
            // listed from the other end. A pool's lower and upper ends are places on the ground,
            // so they keep their values; the lift changes sign as the weir's does.
            if let Some(p) = r["cascade"]["flood"]["pools"].as_array_mut() {
                p.reverse();
                for q in p.iter_mut() {
                    if let (Some(a), Some(b)) = (q["start_m"].as_f64(), q["end_m"].as_f64()) {
                        q["start_m"] = json!((baseline_len - b).max(0.0));
                        q["end_m"] = json!((baseline_len - a).max(0.0));
                    }
                    if let Some(l) = q["lift_m"].as_f64() {
                        q["lift_m"] = json!(-l);
                    }
                }
            }
            turn_design_line(&mut r["design_line"], baseline_len);
            if fresh.contains(&Stage::Optimise) {
                notes.push(OpenNote::action(
                    "the pools and the weirs are the same, counted from the mouth: a weir's pool \
                     level is now the level on its upstream side looking from the mouth, and its \
                     lift is negative where the water steps down going upstream. The optimise \
                     settings (the base level and the terminal level) are kept as they were, but \
                     they now read from the mouth: check them before you run `owd optimise` again"
                        .to_string(),
                ));
            }
        }
        // The volumes: the row of the design line the later stages read.
        let id = doc["stages"]["volumes"]["results"]["id"].clone();
        let row_len = [
            &doc["stages"]["optimise"]["results"]["design_line"],
            &doc["stages"]["alignment"]["results"],
        ]
        .iter()
        .find(|l| l["id"] == id)
        .map(|l| line_len(&l["line"]));
        let vo = &mut doc["stages"]["volumes"]["results"];
        if vo.is_object()
            && let Some(len) = row_len
        {
            if let Some(cs) = vo.pointer_mut("/screening/constraint_stretches") {
                turn_spans(cs, "start_chainage_m", "end_chainage_m", len);
                if let Some(a) = cs.as_array_mut() {
                    for s in a.iter_mut() {
                        if let Some(w) = s["worst_chainage_m"].as_f64() {
                            s["worst_chainage_m"] = json!((len - w).max(0.0));
                        }
                    }
                    // In start order, as the volumes stage writes them (they may overlap).
                    a.sort_by(|x, y| {
                        let at = |v: &Value| v["start_chainage_m"].as_f64().unwrap_or(0.0);
                        at(x).total_cmp(&at(y))
                    });
                }
            }
            let mut texts = false;
            if let Some(w) = vo
                .pointer_mut("/sanity/warnings")
                .and_then(Value::as_array_mut)
            {
                for t in w.iter_mut() {
                    if let Some(s) = t.as_str() {
                        let turned = turn_text(s, len);
                        texts |= turned != s;
                        *t = json!(turned);
                    }
                }
            }
            if fresh.contains(&Stage::Volumes) {
                notes.push(OpenNote::info(
                    "the volumes keep their totals (the same line, the same cut); their stations \
                     were walked from the old start, so a new `owd volumes` run, which walks from \
                     the mouth, may move the totals a little"
                        .to_string(),
                ));
                if texts {
                    notes.push(OpenNote::info(
                        "the chainages in the volumes' warnings are turned from whole metres, so \
                         each is within a metre"
                            .to_string(),
                    ));
                }
            }
        }
        for (k, what) in [("fits", "fits"), ("envelope", "envelope")] {
            let stored = |v: Option<&Value>| v.is_some_and(|v| !v.is_null());
            if stored(doc.get(k)) || stored(doc["stages"].get(k)) {
                notes.push(OpenNote::action(format!(
                    "the {what} answer stored with the file ran from the other end: it shows as \
                     stale; run `owd {what}` again"
                )));
            }
        }
        notes
    }
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;

    #[test]
    fn a_chainage_in_a_sentence_counts_from_the_other_end() {
        assert_eq!(
            turn_text(
                "the cut between 120 m and 480 m reaches 90 m2 a station",
                1000.0
            ),
            "the cut between 520 m and 880 m reaches 90 m2 a station"
        );
        assert_eq!(
            turn_text(
                "the design line crosses itself near 300 m along the line, at (1.0, 2.0)",
                1000.0
            ),
            "the design line crosses itself near 700 m along the line, at (1.0, 2.0)"
        );
        assert_eq!(turn_text("no chainage here", 1000.0), "no chainage here");
    }

    #[test]
    fn a_stretch_turns_and_its_ends_change_places() {
        let mut v = json!([
            {"from": 0.0, "to": 100.0, "start": [0, 0], "end": [1, 1]},
            {"from": 600.0, "to": 1000.0}
        ]);
        turn_spans(&mut v, "from", "to", 1000.0);
        assert_eq!(
            v,
            json!([
                {"from": 0.0, "to": 400.0},
                {"from": 900.0, "to": 1000.0, "start": [1, 1], "end": [0, 0]}
            ])
        );
    }
}
