//! The reference line (spec 0016 T1; spec 0017 ticket 7: a typed stage). One smooth line in the
//! water that belongs to no vessel (see `owd_core::alignment::reference_line`), made from the two
//! banks and the river polygon.
//!
//! [`reference_line_stage`] is a pure function from a [`ReferenceLineIn`] to a [`ReferenceLine`]. The
//! document stores it as a stage record under `stages.reference_line`: the parameters as its
//! settings and the line as its results. Two hashes (QA on PR 560):
//!
//! - The record's `depends_on` hashes its whole input and the project CRS (ADR 0015). A new
//!   centreline or new bank notes change the line's low-confidence stretches, so the record goes
//!   stale and says so.
//! - The line's shape is keyed by `line_basis` in its results: the hash of the banks, the river
//!   polygon, the CRS and the parameters (`basis_of`), the formula OWD used before the line was a
//!   stage. The bed is centred on the line while that matches, so new notes do not make the
//!   volumes fall back to the centreline.
//!
//! The design stages do not read the line: storing it leaves them fresh. A line stored the old way,
//! as `inputs.reference_line`, moves to the stage record when the file opens: a fresh line stays
//! fresh (and its volumes keep their hash), a stale one stays stale.
//!
use super::alignment::pts_of;
use super::hash::hash_value;
use super::session::{DesignError, ENGINE_VERSION, OpenNote, Session, StageStatus};
use crate::types::{Polyline, RiverPolygon, StageError};
use owd_core::alignment::reference_line::{
    CHAIKIN_PASSES, CLEARANCE_MIN_M, CLEARANCE_SHARE, END_TRIM_M, REFIT_CHORD_M, REFIT_SCALE_M,
    SEED_MAX_STEPS_AHEAD, SEED_PULL_WEIGHT, SEED_STEP_M, TAUBIN_ITERATIONS, TAUBIN_LAMBDA,
    TAUBIN_MU, carry_by_chainage, reference_line,
};
use owd_core::geom::Pt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The stage's name, as the document stores it.
pub const STAGE: &str = "reference_line";

/// The settings the line is made with, as the study records them.
pub fn reference_line_parameters() -> Value {
    json!({
        "seed_step_m": SEED_STEP_M,
        "seed_max_steps_ahead": SEED_MAX_STEPS_AHEAD,
        "chaikin_passes": CHAIKIN_PASSES,
        "taubin": {"lambda": TAUBIN_LAMBDA, "mu": TAUBIN_MU, "iterations": TAUBIN_ITERATIONS},
        "clearance": {"min_m": CLEARANCE_MIN_M, "share_of_width": CLEARANCE_SHARE},
        "seed_pull_weight": SEED_PULL_WEIGHT,
        "refit_scale_m": REFIT_SCALE_M,
        "refit_chord_m": REFIT_CHORD_M,
        "end_trim_m": END_TRIM_M,
    })
}

/// A stretch of a line by chainage, in metres, with its kind when it has one.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct LineStretch {
    /// Where it starts.
    pub from_m: f64,
    /// Where it ends.
    pub to_m: f64,
    /// What it is (`narrow`, `no-water`, ...), when it says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// What the reference line stage reads.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct ReferenceLineIn {
    /// The left bank.
    pub left_bank: Polyline,
    /// The right bank.
    pub right_bank: Polyline,
    /// The river as water, with its islands.
    pub river_polygon: RiverPolygon,
    /// The river centreline the low-confidence stretches are counted on, when there is one.
    #[serde(default)]
    pub centreline: Option<Polyline>,
    /// The banks' low-confidence stretches (`low_confidence_width`), on the centreline.
    #[serde(default)]
    pub low_confidence: Vec<LineStretch>,
    /// The parameters the line is made with (the engine's own; recorded so a change shows).
    #[serde(default)]
    pub parameters: Value,
}

/// What the reference line stage returns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ReferenceLine {
    /// The line: a GeoJSON `LineString` in the project CRS. Chainage is along it from its start.
    #[serde(with = "super::design_line::line_string")]
    #[schemars(with = "super::design_line::line_string::LineString")]
    pub line: Polyline,
    /// The banks' low-confidence stretches, carried onto the line by position.
    pub low_confidence: Vec<LineStretch>,
    /// Stretches where the water is too narrow to hold the line.
    pub outside_water: Vec<LineStretch>,
    /// What the engine says about the line.
    pub notes: Vec<String>,
    /// The hash of what shaped the line: the banks, the river polygon, the project CRS and the
    /// parameters (the record's `depends_on` hashes its whole input). The bed is centred on the
    /// line while this matches, so new bank notes leave the volumes fresh (QA on PR 560).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_basis: Option<String>,
}

fn refuse(code: &str, message: &str) -> StageError {
    StageError::new(STAGE, code, message)
}

/// The reference line stage: the line from the two banks and the water.
pub fn reference_line_stage(input: &ReferenceLineIn) -> Result<ReferenceLine, StageError> {
    if input.left_bank.len() < 2 || input.right_bank.len() < 2 {
        return Err(refuse(
            "REFERENCE_LINE_NO_BANKS",
            "the reference line needs both bank lines: import them first",
        ));
    }
    if input.river_polygon.shell.len() < 3 {
        return Err(refuse(
            "REFERENCE_LINE_NO_WATER",
            "the reference line needs the river polygon: import it first (`owd import --polygon FILE`)",
        ));
    }
    let water = vec![input.river_polygon.to_polygon()];
    let rl = reference_line(&input.left_bank.0, &input.right_bank.0, &water).map_err(|e| {
        refuse(
            "REFERENCE_LINE_FAILED",
            &format!("the reference line: {}", e.message),
        )
    })?;
    // The low-confidence stretches are by chainage on the study's centreline: carried one by one,
    // so each keeps its kind.
    let mut low: Vec<LineStretch> = match &input.centreline {
        Some(centre) if centre.len() >= 2 => input
            .low_confidence
            .iter()
            .flat_map(|s| {
                carry_by_chainage(&centre.0, &rl.line, &[(s.from_m, s.to_m)])
                    .into_iter()
                    .map(|(a, b)| LineStretch {
                        from_m: a,
                        to_m: b,
                        kind: s.kind.clone(),
                    })
            })
            .collect(),
        _ => vec![],
    };
    low.sort_by(|a, b| a.from_m.total_cmp(&b.from_m));
    Ok(ReferenceLine {
        line: Polyline(rl.line.clone()),
        low_confidence: low,
        outside_water: rl
            .outside_water
            .iter()
            .map(|s| LineStretch {
                from_m: s.start_chainage_m,
                to_m: s.end_chainage_m,
                kind: None,
            })
            .collect(),
        notes: rl.notes.clone(),
        line_basis: None,
    })
}

/// What makes the line, hashed: the two banks, the river polygon, the project CRS and the
/// parameters, as the document holds them. The stored line is fresh while its `depends_on` equals
/// this. The same formula as before the line became a stage, so an older file's hash still holds.
pub(super) fn basis_of(doc: &Value) -> String {
    let inp = &doc["inputs"];
    hash_value(&json!({
        "left_bank": inp["left_bank"],
        "right_bank": inp["right_bank"],
        "river_polygon": inp["river_polygon"],
        "crs": doc["project"]["crs"],
        "parameters": reference_line_parameters(),
    }))
}

/// Move a reference line stored the old way (`inputs.reference_line`) to its stage record, keeping
/// its hash, and say what happened in plain words. For any format: on the raw JSON, before the
/// schema check. A file that also holds the stage record keeps the record; the old copy goes.
pub(super) fn move_from_inputs(doc: &mut Value) -> (Vec<OpenNote>, bool) {
    let Some(old) = doc
        .get_mut("inputs")
        .and_then(Value::as_object_mut)
        .and_then(|o| o.remove("reference_line"))
    else {
        return (vec![], false);
    };
    if !old.is_object() {
        return (vec![], false);
    }
    if doc["stages"][STAGE].is_object() {
        return (
            vec![OpenNote::action(
                "The file held the reference line in two places: the stage record \
             `stages.reference_line` is kept and the old copy under `inputs.reference_line` is \
             dropped.",
            )],
            false,
        );
    }
    if !doc["stages"].is_object() {
        doc["stages"] = json!({});
    }
    let fresh = old["depends_on"].as_str() == Some(basis_of(doc).as_str());
    // A hash that is not one (an edited file) is replaced by one that matches nothing: stale.
    let depends_on = match old["depends_on"].as_str() {
        Some(h) if h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()) => json!(h),
        _ => json!(hash_value(&json!({"moved_from_inputs": old["depends_on"]}))),
    };
    doc["stages"][STAGE] = json!({
        "settings": old["parameters"],
        "results": {
            "line": old["line"],
            "low_confidence": old.get("low_confidence").cloned().unwrap_or(json!([])),
            "outside_water": old.get("outside_water").cloned().unwrap_or(json!([])),
            "notes": old.get("notes").cloned().unwrap_or(json!([])),
            "line_basis": depends_on,
        },
        "depends_on": depends_on,
        "engine_version": doc["engine_version"],
    });
    (
        vec![if fresh {
            OpenNote::info(
                "The reference line moved from `inputs.reference_line` to its stage record \
         `stages.reference_line`. It is still fresh: nothing it is made from changed.",
            )
        } else {
            OpenNote::info(
                "The reference line moved from `inputs.reference_line` to its stage record \
         `stages.reference_line`. It was stale and stays stale: the next `owd fits` or \
         `owd envelope` makes it again.",
            )
        }],
        fresh,
    )
}

impl Session {
    /// What the reference line stage reads, from the document: `None` without both banks and the
    /// river polygon.
    pub fn reference_line_input(&self) -> Option<ReferenceLineIn> {
        let inp = &self.doc["inputs"];
        let (left, right) = (pts_of(&inp["left_bank"])?, pts_of(&inp["right_bank"])?);
        let river_polygon = RiverPolygon::from_geometry(&inp["river_polygon"])?;
        let low = self
            .low_confidence_width()
            .and_then(|lc| lc["stretches"].as_array().cloned())
            .unwrap_or_default()
            .iter()
            .map(|s| LineStretch {
                from_m: s["from_m"].as_f64().unwrap_or(0.0),
                to_m: s["to_m"].as_f64().unwrap_or(0.0),
                kind: s["kind"].as_str().map(str::to_string),
            })
            .collect();
        Some(ReferenceLineIn {
            left_bank: Polyline(left),
            right_bank: Polyline(right),
            river_polygon,
            centreline: pts_of(&inp["centreline"]).map(Polyline),
            low_confidence: low,
            parameters: reference_line_parameters(),
        })
    }

    /// Where the stored reference line stands: not made, fresh, or stale.
    pub fn reference_line_status(&self) -> StageStatus {
        let r = &self.doc["stages"][STAGE];
        if !r["results"].is_object() {
            return StageStatus::NotRun;
        }
        match (r["depends_on"].as_str(), self.reference_line_record_hash()) {
            (Some(h), Some(now)) if h == now => StageStatus::Fresh,
            _ => StageStatus::Stale,
        }
    }

    /// The hash the stage record keeps: its whole typed input and the project CRS (ADR 0015).
    pub(super) fn reference_line_record_hash(&self) -> Option<String> {
        self.reference_line_input()
            .map(|i| hash_value(&json!({"input": i, "crs": self.doc["project"]["crs"]})))
    }

    /// True while the stored line's shape is what its banks, polygon, CRS and parameters make now,
    /// whether or not its notes are up to date.
    fn reference_line_shape_fresh(&self) -> bool {
        let r = &self.doc["stages"][STAGE];
        let basis = r["results"]["line_basis"]
            .as_str()
            .or(r["depends_on"].as_str());
        r["results"].is_object() && basis == Some(basis_of(&self.doc).as_str())
    }

    /// The stored reference line (the stage's results), fresh or not.
    pub fn reference_line_value(&self) -> Option<&Value> {
        let r = &self.doc["stages"][STAGE]["results"];
        r.is_object().then_some(r)
    }

    /// The stored reference line's points, fresh or not.
    pub fn reference_line(&self) -> Option<Vec<Pt>> {
        self.reference_line_value().and_then(|r| pts_of(&r["line"]))
    }

    /// Make the reference line from the banks and the river polygon and store it as a stage record,
    /// with the banks' low-confidence stretches carried onto it by position.
    pub fn run_reference_line(&mut self) -> Result<(), DesignError> {
        let input = self.reference_line_input();
        let input = input.unwrap_or_else(|| {
            // The stage names what is missing.
            let inp = &self.doc["inputs"];
            ReferenceLineIn {
                left_bank: Polyline(pts_of(&inp["left_bank"]).unwrap_or_default()),
                right_bank: Polyline(pts_of(&inp["right_bank"]).unwrap_or_default()),
                ..ReferenceLineIn::default()
            }
        });
        let mut out = reference_line_stage(&input).map_err(|e| DesignError::Input(e.message))?;
        out.line_basis = Some(basis_of(&self.doc));
        if !self.doc["stages"].is_object() {
            self.doc["stages"] = json!({});
        }
        self.doc["stages"][STAGE] = json!({
            "settings": input.parameters,
            "results": out,
            "depends_on": hash_value(&json!({"input": input, "crs": self.doc["project"]["crs"]})),
            "engine_version": ENGINE_VERSION,
        });
        Ok(())
    }

    /// The line the assumed bed is centred on, when it is not the centreline: a fresh reference
    /// line (#398).
    pub(super) fn bed_axis_line(&self) -> Option<Vec<Pt>> {
        self.reference_line_shape_fresh()
            .then(|| self.reference_line())
            .flatten()
    }

    /// What [`Session::bed_axis_line`] was made from, for the hashes of what reads the bed: the
    /// hash of what shaped the line (the formula before the line was a stage, so an older file's
    /// volumes keep their hash).
    pub(super) fn bed_axis_hash(&self) -> Option<Value> {
        self.bed_axis_line().map(|_| json!(basis_of(&self.doc)))
    }

    /// The reference line, made again first when it is missing or stale.
    pub fn fresh_reference_line(&mut self) -> Result<Vec<Pt>, DesignError> {
        if self.reference_line_status() != StageStatus::Fresh {
            self.run_reference_line()?;
        }
        self.reference_line()
            .ok_or_else(|| DesignError::Input("the reference line was not stored".into()))
    }
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;
    use crate::design::{Role, Stage};

    const UTM30: &str =
        r#""crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

    fn at(x: f64, y: f64) -> [f64; 2] {
        [x + 500_000.0, y + 600_000.0]
    }

    fn line(pts: &[[f64; 2]]) -> String {
        format!(
            r#"{{"type":"LineString",{UTM30},"coordinates":{}}}"#,
            json!(pts)
        )
    }

    /// A straight river 100 m wide and 2 km long, with its banks, polygon and centreline.
    fn study(shift: f64) -> Session {
        let mut s = Session::new("t", "2026-10-22");
        let left: Vec<[f64; 2]> = (0..=100)
            .map(|i| at(20.0 * i as f64, 50.0 + shift))
            .collect();
        let right: Vec<[f64; 2]> = (0..=100).map(|i| at(20.0 * i as f64, -50.0)).collect();
        let centre: Vec<[f64; 2]> = (0..=100)
            .map(|i| at(20.0 * i as f64, shift / 2.0))
            .collect();
        let mut ring = right.clone();
        ring.extend(left.iter().rev());
        ring.push(ring[0]);
        let polygon = format!(
            r#"{{"type":"Polygon",{UTM30},"coordinates":[{}]}}"#,
            json!(ring)
        );
        s.import(&[
            (Role::RiverPolygon, polygon.as_str()),
            (Role::LeftBank, line(&left).as_str()),
            (Role::RightBank, line(&right).as_str()),
            (Role::Centreline, line(&centre).as_str()),
        ])
        .unwrap();
        s
    }

    #[test]
    fn it_is_stored_and_reopens_fresh() {
        let mut s = study(0.0);
        assert_eq!(s.reference_line_status(), StageStatus::NotRun);
        s.run_reference_line().unwrap();
        let back = Session::open(&s.to_text()).unwrap();
        assert_eq!(back.reference_line_status(), StageStatus::Fresh);
        let l = back.reference_line().unwrap();
        assert!(l.iter().all(|p| (p[1] - 600_000.0).abs() < 0.5));
        assert_eq!(
            back.document()["stages"][STAGE]["settings"]["seed_pull_weight"],
            json!(SEED_PULL_WEIGHT)
        );
    }

    #[test]
    fn a_change_to_the_banks_makes_it_stale() {
        let mut s = study(0.0);
        s.run_reference_line().unwrap();
        let moved: Vec<[f64; 2]> = (0..=100).map(|i| at(20.0 * i as f64, 60.0)).collect();
        s.import(&[(Role::LeftBank, line(&moved).as_str())])
            .unwrap();
        assert_eq!(s.reference_line_status(), StageStatus::Stale);
        let l = s.fresh_reference_line().unwrap();
        assert_eq!(s.reference_line_status(), StageStatus::Fresh);
        // Now midway between y = -50 and y = 60 (the polygon still ends at 50: the clearance holds
        // the line inside it).
        assert!(l.iter().all(|p| p[1] - 600_000.0 > 0.0));
    }

    #[test]
    fn storing_it_leaves_the_stages_as_they_were() {
        let mut s = study(0.0);
        let before: Vec<String> = Stage::ALL.iter().map(|st| s.dependency_hash(*st)).collect();
        s.run_reference_line().unwrap();
        let after: Vec<String> = Stage::ALL.iter().map(|st| s.dependency_hash(*st)).collect();
        assert_eq!(before, after);
    }

    #[test]
    fn low_confidence_stretches_are_carried_onto_it() {
        let mut s = study(0.0);
        s.set_low_confidence_width(json!({"min_width_m": 90.0, "stretches": [
            {"from_m": 500.0, "to_m": 700.0, "kind": "narrow"}]}))
            .unwrap();
        s.run_reference_line().unwrap();
        let low = &s.reference_line_value().unwrap()["low_confidence"];
        assert_eq!(low[0]["kind"], "narrow");
        // The line starts `END_TRIM_M` along the centreline: the stretch keeps its place.
        assert!((low[0]["from_m"].as_f64().unwrap() - (500.0 - END_TRIM_M)).abs() < 1.0);
        assert!((low[0]["to_m"].as_f64().unwrap() - (700.0 - END_TRIM_M)).abs() < 1.0);
    }
}
