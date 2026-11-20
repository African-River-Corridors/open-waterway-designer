//! The largest vessel the river allows (spec 0016 T2): `stages.envelope`. A sweep of single-hull
//! sizes on the reference line (see `owd_core::envelope`), kept in the study. Like `fits`, it is
//! never rerun by `run_stale`: a change to the banks, the river polygon, the reference line, the
//! custom vessel or the settings makes it stale, and only `owd envelope` replaces it.
//!
//! Spec 0017 ticket 7: its output is the typed [`EnvelopeResult`] (schema
//! `schema/stages/envelope.1.schema.json`), and its `depends_on` is the hash of the typed
//! [`EnvelopeIn`] it read: the plan line (the fresh reference line, else the centreline), the water,
//! the custom vessel, the settings and the bed's shallowest depth. It has no hash of its own.

use super::hash::hash_value;
use super::screening::SCREENING_NOTICE;
use super::session::{DesignError, ENGINE_VERSION, Session, StageStatus};
use crate::types::{Polyline, RiverPolygon};
use owd_core::envelope::{Envelope, Settings, Size, combine, longest_for_beam, size_result};
use owd_core::fits::FitStretch;
use owd_core::geom::Pt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Said on every output: the sweep reads width and radius on lines fitted in the banks as found.
pub const BANK_CAVEAT: &str = "the answer is only as good as the banks: a narrow or flagged \
     stretch of bank can set the limit for the whole river (see the low-confidence stretches)";

/// Said on every output: what the sweep reads.
pub const ENVELOPE_LABEL: &str = "single-hull motor vessels; width and radius only, no depth";

/// The stage's name, as the document stores it.
pub const STAGE: &str = "envelope";

/// A number the sweep reports; `null` where it is not a number (JSON has no NaN).
pub type Num = Option<f64>;

/// One stretch where a size does not fit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StretchOut {
    pub kind: String,
    pub start_chainage_m: Num,
    pub end_chainage_m: Num,
    pub start: [Num; 2],
    pub end: [Num; 2],
    pub required_m: Num,
    pub found_m: Num,
    pub deficit_m: Num,
}

/// A size of the series, by label.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SizeLabel {
    pub label: String,
    pub loa_m: Num,
    pub beam_m: Num,
}

/// One size's answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SizeRow {
    pub label: String,
    pub loa_m: Num,
    pub beam_m: Num,
    pub r_min_m: Num,
    pub bottom_width_straight_m: Num,
    pub passes: bool,
    pub stretch_count: usize,
    pub stretches: Vec<StretchOut>,
    pub notes: Vec<String>,
}

/// The largest size that fits in one kilometre of the plan line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct KmRow {
    pub from_m: Num,
    pub to_m: Num,
    pub from: [Num; 2],
    pub to: [Num; 2],
    pub largest: Option<SizeLabel>,
}

/// A vessel class (or the custom vessel) marked against the answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MarkRow {
    pub id: String,
    pub custom: bool,
    pub loa_m: Num,
    pub beam_m: Num,
    pub within: bool,
}

/// The longest length for one beam (ticket C5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BeamRow {
    pub beam_m: Num,
    pub longest_m: Num,
    pub at_max: bool,
    pub binding: Vec<StretchOut>,
}

/// The bed model's draught limit, beside the answer (the sweep is plan only).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BedLimit {
    pub shallowest_depth_m: Num,
    pub draught_m: Num,
    pub note: String,
}

/// What the envelope stage returns, as the study keeps it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EnvelopeResult {
    pub label: String,
    pub notice: String,
    pub bank_caveat: String,
    pub rule_set: String,
    pub draught_m: Num,
    pub clearance_m: Num,
    pub length_m: Num,
    pub largest: Option<SizeLabel>,
    pub sizes: Vec<SizeRow>,
    pub per_km: Vec<KmRow>,
    pub marks: Vec<MarkRow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by_beam: Option<Vec<BeamRow>>,
    /// `reference` or `centreline`: the line the sizes were read on.
    pub plan_line: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_line_note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bed_model: Option<BedLimit>,
}

/// What the envelope stage reads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EnvelopeIn {
    /// The line the sizes are read on: the fresh reference line, else the river centreline.
    pub plan_line: Polyline,
    /// `reference` or `centreline`.
    pub plan_line_kind: String,
    /// The river as water.
    pub river_polygon: RiverPolygon,
    /// The custom vessel, marked against the answer when there is one.
    #[serde(default)]
    pub custom_vessel: Value,
    /// The sweep's settings, as stored.
    pub settings: Value,
    /// The shallowest depth of the bed model, when there is one.
    #[serde(default)]
    pub bed_shallowest_depth_m: Option<f64>,
}

fn stretch_json(st: &FitStretch) -> Value {
    json!({
        "kind": st.kind.as_str(),
        "start_chainage_m": st.start_chainage_m,
        "end_chainage_m": st.end_chainage_m,
        "start": st.start,
        "end": st.end,
        "required_m": st.required_m,
        "found_m": st.found_m,
        "deficit_m": st.deficit_m,
    })
}

fn size_json(s: &Size) -> Value {
    json!({"label": s.label(), "loa_m": s.loa_m, "beam_m": s.beam_m})
}

/// The answer as the study keeps it. `positions` gives the point at a chainage of the plan line.
fn envelope_json(e: &Envelope, at: &dyn Fn(f64) -> Pt) -> Value {
    let label = |i: Option<usize>| i.map_or(Value::Null, |i| size_json(&e.sizes[i].size));
    json!({
        "label": ENVELOPE_LABEL,
        "notice": SCREENING_NOTICE,
        "bank_caveat": BANK_CAVEAT,
        "rule_set": e.settings.rule_set,
        "draught_m": e.settings.draught_m,
        "clearance_m": e.settings.clearance_m,
        "length_m": e.length_m,
        "largest": label(e.largest),
        "sizes": e.sizes.iter().map(|s| json!({
            "label": s.size.label(),
            "loa_m": s.size.loa_m,
            "beam_m": s.size.beam_m,
            "r_min_m": s.r_min_m,
            "bottom_width_straight_m": s.bottom_width_straight_m,
            "passes": s.passes,
            "stretch_count": s.stretches.len(),
            "stretches": s.stretches.iter().map(stretch_json).collect::<Vec<_>>(),
            "notes": s.notes,
        })).collect::<Vec<_>>(),
        "per_km": e.per_km.iter().map(|k| json!({
            "from_m": k.from_m,
            "to_m": k.to_m,
            "from": at(k.from_m),
            "to": at(k.to_m),
            "largest": label(k.largest),
        })).collect::<Vec<_>>(),
        "marks": e.marks.iter().map(|m| json!({
            "id": m.id,
            "custom": m.custom,
            "loa_m": m.loa_m,
            "beam_m": m.beam_m,
            "within": m.within,
        })).collect::<Vec<_>>(),
    })
}

impl Session {
    /// The settings as the record stores them.
    fn envelope_settings_value(settings: &Settings) -> Value {
        let mut v = json!({
            "sizes": settings.sizes.iter().map(size_json).collect::<Vec<_>>(),
            "draught_m": settings.draught_m,
            "rule_set": settings.rule_set,
            "clearance_m": settings.clearance_m,
        });
        // The length search's settings, only when it ran: a sweep with no beam is as before.
        if !settings.beams_m.is_empty() {
            v["beams_m"] = json!(settings.beams_m);
            let ls = settings.length_search;
            v["length_search"] = json!({"min_m": ls.min_m, "max_m": ls.max_m, "step_m": ls.step_m});
        }
        v
    }

    /// The bed model's shallowest depth, when there is a bed model.
    fn bed_shallowest(&self) -> Option<f64> {
        let b = self.bed_model_value()?;
        let d = b["points"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| p[1].as_f64())
            .fold(f64::INFINITY, f64::min);
        d.is_finite().then_some(d)
    }

    /// What the envelope stage reads, from the document, with `settings` as the record stores them:
    /// the fresh reference line, else the centreline. `None` without a river polygon or a line.
    fn envelope_input_with(&self, settings: Value) -> Option<EnvelopeIn> {
        let river_polygon = RiverPolygon::from_geometry(&self.doc["inputs"]["river_polygon"])?;
        let (line, kind) = match self.bed_axis_line() {
            Some(l) => (l, "reference"),
            None => (self.river_centreline().ok()?, "centreline"),
        };
        Some(EnvelopeIn {
            plan_line: Polyline(line),
            plan_line_kind: kind.into(),
            river_polygon,
            custom_vessel: self.doc["stages"]["sizing"]["settings"]["custom_vessel"].clone(),
            settings,
            bed_shallowest_depth_m: self.bed_shallowest(),
        })
    }

    /// What the stored sweep would read now, with its stored settings.
    pub fn envelope_input(&self) -> Option<EnvelopeIn> {
        let settings = self.doc["stages"][STAGE]["settings"].clone();
        (!settings.is_null())
            .then(|| self.envelope_input_with(settings))
            .flatten()
    }

    /// The hash of what the stored sweep would read now.
    fn envelope_dependency_hash(&self) -> String {
        hash_value(&json!(self.envelope_input()))
    }

    /// Where the sweep stands: not run, fresh, or stale.
    pub fn envelope_status(&self) -> StageStatus {
        let e = &self.doc["stages"]["envelope"];
        if e["results"].is_null() {
            return StageStatus::NotRun;
        }
        if e["depends_on"].as_str() == Some(self.envelope_dependency_hash().as_str()) {
            StageStatus::Fresh
        } else {
            StageStatus::Stale
        }
    }

    /// The sweep the study holds, if it has run.
    pub fn envelope_result(&self) -> Option<&Value> {
        let r = &self.doc["stages"]["envelope"]["results"];
        (!r.is_null()).then_some(r)
    }

    /// Run the sweep on the reference line (made again first when missing or stale) and keep it.
    /// The sizes run at once when `parallel`. Without both banks, the centreline is read, and the
    /// answer says so.
    pub fn run_envelope(&mut self, settings: Settings, parallel: bool) -> Result<(), DesignError> {
        if settings.sizes.is_empty() {
            return Err(DesignError::Input(
                "the series needs at least one size".into(),
            ));
        }
        if !(settings.draught_m.is_finite() && settings.draught_m > 0.0) {
            return Err(DesignError::Input(
                "the draught must be a positive number".into(),
            ));
        }
        let water = self.river_polygons().ok_or_else(|| {
            DesignError::Input(
                "the sweep needs the river polygon: import it first (`owd import --polygon FILE`)"
                    .into(),
            )
        })?;
        let (line, plan_line, note) = match self.fresh_reference_line() {
            Ok(l) => (l, "reference", None),
            Err(e) => (
                self.river_centreline()?,
                "centreline",
                Some(format!(
                    "the reference line could not be made ({e}): the sizes are seeded from the centreline"
                )),
            ),
        };
        let answers = super::fits::run_all(
            &settings.sizes,
            |s| size_result(&line, &water, &settings, *s),
            parallel,
        );
        let sizes = answers
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| DesignError::Input(format!("the sweep: {}", e.message)))?;
        // The longest length for each beam (ticket C5), on the same plan line and water.
        let by_beam = settings
            .beams_m
            .iter()
            .map(|&b| longest_for_beam(&line, &water, &settings, b))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| DesignError::Input(format!("the length search: {}", e.message)))?;
        let custom = self.custom_vessel();
        let mut e = combine(&line, settings.clone(), sizes, custom.as_ref());
        e.by_beam = by_beam;
        let chained = owd_core::geom::linear::length(&line);
        let at = |s: f64| {
            owd_core::geom::linear::interpolate(&line, s.min(chained)).unwrap_or([f64::NAN; 2])
        };
        let mut results = envelope_json(&e, &at);
        if !e.settings.beams_m.is_empty() {
            results["by_beam"] = json!(
                e.by_beam
                    .iter()
                    .map(|b| json!({
                        "beam_m": b.beam_m,
                        "longest_m": b.longest_m,
                        "at_max": b.at_max,
                        "binding": b.binding.iter().map(stretch_json).collect::<Vec<_>>(),
                    }))
                    .collect::<Vec<_>>()
            );
        }
        results["plan_line"] = json!(plan_line);
        if let Some(n) = note {
            results["plan_line_note"] = json!(n);
        }
        // The draught limit from the bed model, beside the answer: the sweep is plan only.
        if let Some(b) = self.bed_model_value() {
            let depths: Vec<f64> = b["points"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|p| p[1].as_f64())
                .collect();
            let shallowest = depths.iter().copied().fold(f64::INFINITY, f64::min);
            if shallowest.is_finite() {
                results["bed_model"] = json!({
                    "shallowest_depth_m": shallowest,
                    "draught_m": settings.draught_m,
                    "note": "the depth is assumed, not measured, and not read by the sweep: \
                             compare the draught with it",
                });
            }
        }
        // The typed output: what the study keeps must read as the contract says.
        let typed: EnvelopeResult = serde_json::from_value(results).map_err(|e| {
            DesignError::Input(format!(
                "the sweep's answer does not read as its contract: {e}"
            ))
        })?;
        let set = Self::envelope_settings_value(&settings);
        let input = self.envelope_input_with(set.clone());
        if !self.doc["stages"].is_object() {
            self.doc["stages"] = json!({});
        }
        self.doc["stages"][STAGE] = json!({
            "settings": set,
            "results": typed,
            "depends_on": hash_value(&json!(input)),
            "engine_version": ENGINE_VERSION,
        });
        Ok(())
    }
}
