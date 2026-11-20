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
//! of the stored centreline, as before (a document from format 0.4 opens this way).

use super::river::{Mouth, RiverIn, river};
use super::session::{DesignError, Session};
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
        // The optimise stage's candidate weir sites are metres along the design line (the
        // alignment's line, else the river centreline): counted from the other end of that line.
        let line_len =
            super::design_line::alignment_line_of(&self.doc).map_or(len, |l| l.line.length_m());
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
