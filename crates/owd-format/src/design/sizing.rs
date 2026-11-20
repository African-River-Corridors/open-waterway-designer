//! The sizing stage (spec 0017, ADR 0015): the vessel and the Rule Sets turned into the channel's
//! size, with the rule trace that says why.
//!
//! [`size`] is a pure function from a [`SizingIn`] and the Rule Set library to a [`ChannelSize`]. It
//! reads no document and no file. [`ChannelSize`] is what the stage stores (`stages.sizing.results`,
//! key for key as before) and what every later stage reads: they take the type, and none rebuilds
//! the shape from the settings.
//!
//! One part of the size is not data yet: the bend widening at an arbitrary radius is worked out by
//! the Rule engine from the selection and the vessel, so a [`ChannelSize`] made by [`size`] carries
//! the engine's shape beside its data (`ChannelSize::shape`). The stored copy has the data only.

use crate::types::StageError;
use arc_rules::load::Library;
use owd_core::rules::RuleError;
use owd_core::rules::custom::{CustomVessel, Formation};
use owd_core::rules::engine::{self, Selection};
use owd_core::rules::rulesets::{
    ChannelShape, channel_shape_for, channel_shape_for_custom, custom_selection, vessel_for,
};
use owd_core::rules::screening::{self, BasisRow as CoreBasisRow};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The stage's name, as the document stores it.
pub const STAGE: &str = "sizing";

/// The notice every sizing output carries while the Rules are unverified.
pub const RULES_NOTICE: &str = super::screening::UNVERIFIED_NOTICE;

/// A Rule Set the design selects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SelectedSet {
    /// The set's id, such as `pianc-wg141`.
    pub id: String,
    /// The set's version, such as `0.3.0`.
    pub version: String,
    /// The content hash the design was sized with. The loaded set must have it; an entry with none is
    /// refused (`SIZING_RULE_SET_NO_HASH`).
    #[serde(default)]
    pub hash: Option<String>,
}

/// A set input, such as `se_quality` `B`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SetInput {
    /// The input's name.
    pub name: String,
    /// Its value.
    pub value: String,
}

/// A custom vessel (spec 0015): the user's own dimensions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CustomVesselIn {
    /// The vessel's name.
    pub name: String,
    /// Length overall in metres (the whole convoy's, for a convoy).
    pub loa_m: f64,
    /// Beam in metres (the whole convoy's, for a convoy).
    pub beam_m: f64,
    /// Draught in metres.
    pub draught_m: f64,
    /// `motor` or `convoy`.
    pub formation: String,
    /// Vessels abreast, for a convoy.
    #[serde(default)]
    pub n_abreast: Option<u32>,
    /// Vessels in line, for a convoy.
    #[serde(default)]
    pub n_in_line: Option<u32>,
    /// The GB 50139 channel class, `I` to `VII`.
    #[serde(default)]
    pub gb_class: Option<String>,
    /// Stored only.
    #[serde(default)]
    pub air_draught_m: Option<f64>,
    /// Stored only.
    #[serde(default)]
    pub deadweight_t: Option<f64>,
}

impl CustomVesselIn {
    fn to_core(&self) -> Result<CustomVessel, RuleError> {
        Ok(CustomVessel {
            name: self.name.clone(),
            loa_m: self.loa_m,
            beam_m: self.beam_m,
            draught_m: self.draught_m,
            formation: Formation::parse(&self.formation)?,
            n_abreast: self.n_abreast,
            n_in_line: self.n_in_line,
            gb_class: self.gb_class.clone(),
            air_draught_m: self.air_draught_m,
            deadweight_t: self.deadweight_t,
        })
    }
}

/// A marked reach of the river, as the rules read it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ReachIn {
    /// Chainage where the reach starts.
    pub start_m: f64,
    /// Chainage where it ends; absent runs to the end of the river.
    #[serde(default)]
    pub end_m: Option<f64>,
    /// `canal`, `impounded-river` or `free-flowing-river`.
    pub waterway: String,
    /// `yes` or `no`: the user claims the reduced radius on this reach.
    #[serde(default = "no")]
    pub reduced_radius_claim: String,
}

fn no() -> String {
    "no".into()
}

/// What the sizing stage reads.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct SizingIn {
    /// The selected Rule Sets, in selection order.
    pub rule_sets: Vec<SelectedSet>,
    /// The inputs the sets apply on.
    #[serde(default)]
    pub set_inputs: Vec<SetInput>,
    /// The vessel class, by catalogue id. Absent for a custom vessel.
    #[serde(default)]
    pub vessel_standard_id: Option<String>,
    /// The draught, when the class's own is not used.
    #[serde(default)]
    pub draught_m: Option<f64>,
    /// A custom vessel, which replaces the class.
    #[serde(default)]
    pub custom_vessel: Option<CustomVesselIn>,
    /// The marked reaches, in chainage order. None: the whole river is one free-flowing reach.
    #[serde(default)]
    pub waterway_reaches: Vec<ReachIn>,
}

impl SizingIn {
    /// The input from the document's sizing settings and its marked reaches (`inputs.waterway_reaches`).
    /// Anything the settings do not hold is left empty.
    pub fn from_document(settings: &Value, reaches: &[Value]) -> SizingIn {
        let text = |v: &Value| v.as_str().unwrap_or("").to_string();
        SizingIn {
            rule_sets: settings["rule_sets"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|s| SelectedSet {
                    id: text(&s["id"]),
                    version: text(&s["version"]),
                    hash: s["hash"].as_str().map(str::to_string),
                })
                .collect(),
            set_inputs: settings["set_inputs"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|i| SetInput {
                    name: text(&i["name"]),
                    value: text(&i["value"]),
                })
                .collect(),
            vessel_standard_id: settings["vessel_standard_id"].as_str().map(str::to_string),
            draught_m: settings["draught_m"].as_f64(),
            custom_vessel: settings["custom_vessel"]
                .is_object()
                .then(|| serde_json::from_value(settings["custom_vessel"].clone()).ok())
                .flatten(),
            waterway_reaches: reaches
                .iter()
                .map(|r| ReachIn {
                    start_m: r["start_m"].as_f64().unwrap_or(0.0),
                    end_m: r["end_m"].as_f64(),
                    waterway: r["waterway"]
                        .as_str()
                        .unwrap_or("free-flowing-river")
                        .to_string(),
                    reduced_radius_claim: r["reduced_radius_claim"]
                        .as_str()
                        .unwrap_or("no")
                        .to_string(),
                })
                .collect(),
        }
    }

    /// The label of the selection, such as `pianc-wg141@0.3.0 (se_quality B)`: the sets (without the
    /// always-selected `owd`), then the inputs.
    pub fn selection_label(&self) -> String {
        let sets: Vec<String> = self
            .rule_sets
            .iter()
            .filter(|s| s.id != engine::OWD_SET)
            .map(|s| format!("{}@{}", s.id, s.version))
            .collect();
        if self.set_inputs.is_empty() {
            sets.join(" + ")
        } else {
            let inputs: Vec<String> = self
                .set_inputs
                .iter()
                .map(|i| format!("{} {}", i.name, i.value))
                .collect();
            format!("{} ({})", sets.join(" + "), inputs.join(", "))
        }
    }

    /// The reaches as the rules read them, in chainage order: the marked reaches, with the chainage
    /// they leave uncovered as free-flowing reaches with no claim.
    pub fn reach_ranges(&self) -> Vec<(f64, f64, engine::WaterwayReach)> {
        if self.waterway_reaches.is_empty() {
            return vec![(0.0, f64::INFINITY, engine::WaterwayReach::whole_river())];
        }
        let free = |label: String| engine::WaterwayReach {
            label,
            ..engine::WaterwayReach::whole_river()
        };
        let mut out = Vec::new();
        let mut at = 0.0_f64;
        for r in &self.waterway_reaches {
            let (start, end) = (r.start_m, r.end_m);
            if start > at {
                out.push((at, start, free(format!("{at} to {start} m"))));
            }
            out.push((
                start,
                end.unwrap_or(f64::INFINITY),
                engine::WaterwayReach {
                    label: match end {
                        Some(e) => format!("{start} to {e} m"),
                        None => format!("{start} m to the end"),
                    },
                    waterway: r.waterway.clone(),
                    claim_typed: r.reduced_radius_claim == "yes",
                    claim: r.reduced_radius_claim.clone(),
                },
            ));
            at = end.unwrap_or(f64::INFINITY);
        }
        if at.is_finite() {
            out.push((at, f64::INFINITY, free(format!("{at} m to the end"))));
        }
        out
    }
}

/// One row of the design basis: a quantity with the Rule that governs it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BasisRow {
    /// `bottom-width`, `min-bend-radius`, `design-depth` or `widening-at-rmin`.
    pub quantity: String,
    /// The waterway kind of the reach, for the minimum radius.
    pub waterway: Option<String>,
    /// The value.
    pub value: f64,
    /// Its unit.
    pub unit: String,
    /// The governing Rule.
    pub rule: String,
    /// The sets that hold the Rule, as `id@version`.
    pub rule_sets: Vec<String>,
    /// The Rule's citation.
    pub citation: String,
    /// Where the Rule comes from.
    pub origin: String,
    /// `verified` or `unverified`.
    pub status: String,
    /// The other Rules of the Requirement that have a limit.
    pub other_rules: Vec<OtherRule>,
}

/// Another Rule of a Requirement, with its limit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OtherRule {
    /// The Rule.
    pub rule: String,
    /// Its limit.
    pub limit: f64,
}

impl From<&CoreBasisRow> for BasisRow {
    fn from(b: &CoreBasisRow) -> Self {
        BasisRow {
            quantity: b.quantity.to_string(),
            waterway: b.waterway.clone(),
            value: b.value,
            unit: b.unit.clone(),
            rule: b.rule.clone(),
            rule_sets: b.rule_sets.clone(),
            citation: b.citation.clone(),
            origin: b.origin.clone(),
            status: b.status.to_string(),
            other_rules: b
                .others
                .iter()
                .map(|(r, l)| OtherRule {
                    rule: r.clone(),
                    limit: *l,
                })
                .collect(),
        }
    }
}

/// What the sizing stage returns: the channel's size and why. The keys are the ones the document has
/// always stored under `stages.sizing.results`.
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChannelSize {
    /// The selection that sized it, such as `pianc-wg141@0.3.0 (se_quality C)`.
    pub rule_sets: String,
    /// The class's catalogue id; null for a custom vessel.
    pub vessel_standard_id: Option<String>,
    /// The vessel's length overall in metres.
    pub loa_m: f64,
    /// The vessel's beam in metres.
    pub beam_m: f64,
    /// The draught in metres.
    pub draught_m: f64,
    /// `motor_vessel` or `pushed_convoy`.
    pub formation: String,
    /// The bottom width of the channel on a straight, in metres.
    pub bottom_width_straight_m: f64,
    /// The minimum bend radius in metres.
    pub min_bend_radius_m: f64,
    /// The design depth in metres.
    pub design_depth_m: f64,
    /// The bend widening at the minimum radius, in metres.
    pub bend_widening_at_rmin_m: f64,
    /// The bottom width at the minimum radius, in metres.
    pub bottom_width_at_rmin_m: f64,
    /// `table`, `appendix-a` or `rule`.
    pub width_basis: String,
    /// Where the vessel's figures come from.
    pub source: String,
    /// Warnings of the sizing.
    pub warnings: Vec<String>,
    /// A custom vessel's label (it has no class id); absent for a class.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vessel_label: Option<String>,
    /// B, the design basis: each quantity with its governing Rule.
    pub design_basis: Vec<BasisRow>,
    /// The notice every output carries while the Rules are unverified.
    pub rules_notice: String,
    /// The engine's shape, for the bend widening at an arbitrary radius. Not stored.
    #[serde(skip)]
    #[schemars(skip)]
    shape: Option<ChannelShape>,
}

/// Two sizes are equal when their data is: the engine's shape is not compared.
impl PartialEq for ChannelSize {
    fn eq(&self, other: &Self) -> bool {
        serde_json::to_value(self).ok() == serde_json::to_value(other).ok()
    }
}

impl std::fmt::Debug for ChannelSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match serde_json::to_string(self) {
            Ok(t) => write!(f, "ChannelSize {t}"),
            Err(_) => f.write_str("ChannelSize"),
        }
    }
}

impl ChannelSize {
    /// The engine's shape of this size. Present on a size made by [`size`] or read through
    /// `Session::channel_size`; it works out the bend widening at any radius. A size read from JSON
    /// holds the data only, so it has no shape and this says so (`SIZING_NO_SHAPE`).
    pub fn shape(&self) -> Result<&ChannelShape, StageError> {
        self.shape.as_ref().ok_or_else(|| {
            refuse(
                "SIZING_NO_SHAPE",
                "this channel size was read from stored data and has no engine shape: read it \
                 through the design session, or make it with `size`",
            )
        })
    }

    /// This size with the engine's shape attached.
    pub fn with_shape(mut self, shape: ChannelShape) -> ChannelSize {
        self.shape = Some(shape);
        self
    }

    /// The bend widening at `radius_m`, in metres; refused when the size has no shape.
    pub fn bend_widening_m(&self, radius_m: f64) -> Result<f64, StageError> {
        Ok(self.shape()?.bend_widening_m(radius_m))
    }

    /// The bottom width at `radius_m`, in metres; refused when the size has no shape.
    pub fn bottom_width_at_m(&self, radius_m: f64) -> Result<f64, StageError> {
        Ok(self.shape()?.bottom_width_at_m(radius_m))
    }
}

fn refuse(code: &str, message: &str) -> StageError {
    StageError::new(STAGE, code, message)
}

fn rule_refusal(e: RuleError) -> StageError {
    let code = match e {
        RuleError::Key(_) => "SIZING_UNKNOWN_VESSEL",
        RuleError::Value(_) => "SIZING_RULES_REFUSED",
    };
    refuse(code, e.raw_message())
}

/// The engine selection of the input, with its reaches. A set the library does not hold, or holds
/// with another content hash than the input records, is refused.
pub fn selection(library: &Library, input: &SizingIn) -> Result<Selection, StageError> {
    selection_without_reaches(library, input).map(|s| {
        let reaches = input
            .reach_ranges()
            .into_iter()
            .map(|(_, _, r)| r)
            .collect();
        s.with_reaches(reaches)
    })
}

/// [`selection`] before the reaches are added.
pub fn selection_without_reaches(
    library: &Library,
    input: &SizingIn,
) -> Result<Selection, StageError> {
    for s in &input.rule_sets {
        let set = library.find(&s.id, &s.version).ok_or_else(|| {
            refuse(
                "SIZING_RULE_SET_NOT_LOADED",
                &format!(
                    "the design selects Rule Set {}@{}, which is not loaded: load its file \
                     with --rule-set-file",
                    s.id, s.version
                ),
            )
        })?;
        // A design that records no content hash cannot say which rules it was sized with: refused,
        // as the session's own path always did. (`SizingIn::from_document` leaves the hash empty
        // only when the document holds none.)
        if s.hash.is_none() {
            return Err(refuse(
                "SIZING_RULE_SET_NO_HASH",
                &format!(
                    "the design selects Rule Set {}@{} but records no content hash for it, so it \
                     cannot say which rules it was sized with: choose the set again (owd size \
                     --rule-set {}) and rerun",
                    s.id, s.version, s.id
                ),
            ));
        }
        if let Some(stored) = &s.hash
            && stored != &set.hash
        {
            return Err(refuse(
                "SIZING_RULE_SET_CHANGED",
                &format!(
                    "the design selects Rule Set {}@{} with content hash {stored}, but the loaded \
                     set has {}: the rules changed. Choose the set again (owd size --rule-set {}) \
                     and rerun",
                    s.id, s.version, set.hash, s.id
                ),
            ));
        }
    }
    let refs: Vec<(&str, &str)> = input
        .rule_sets
        .iter()
        .map(|s| (s.id.as_str(), s.version.as_str()))
        .collect();
    let inputs = input
        .set_inputs
        .iter()
        .map(|i| (i.name.clone(), i.value.clone()))
        .collect();
    Selection::from_library(library, &refs, inputs)
        .map_err(|e| refuse("SIZING_SELECTION_REFUSED", e.raw_message()))
}

/// The engine's channel shape of the input.
pub fn shape(library: &Library, input: &SizingIn) -> Result<ChannelShape, StageError> {
    let selection = selection(library, input)?;
    if let Some(custom) = &input.custom_vessel {
        let custom = custom.to_core().map_err(rule_refusal)?;
        return channel_shape_for_custom(&selection, &custom).map_err(rule_refusal);
    }
    let Some(vessel) = input.vessel_standard_id.as_deref() else {
        return Err(refuse(
            "SIZING_NO_VESSEL",
            "the sizing stage has no vessel class: choose one first",
        ));
    };
    channel_shape_for(&selection, vessel, input.draught_m).map_err(rule_refusal)
}

/// The sizing stage: the channel's size, with the design basis, for the input.
pub fn size(input: &SizingIn, library: &Library) -> Result<ChannelSize, StageError> {
    let shape = shape(library, input)?;
    // The design basis reads the same selection and vessel the shape was sized with.
    let mut sel = selection(library, input)?;
    let vessel = match &input.custom_vessel {
        Some(c) => {
            let c = c.to_core().map_err(rule_refusal)?;
            sel = custom_selection(&sel, &c);
            c.vessel()
        }
        None => {
            let id = input.vessel_standard_id.as_deref().ok_or_else(|| {
                refuse(
                    "SIZING_NO_VESSEL",
                    "the sizing stage has no vessel class: choose one first",
                )
            })?;
            vessel_for(id, input.draught_m).map_err(rule_refusal)?
        }
    };
    let sized = engine::size(&sel, &vessel).map_err(rule_refusal)?;
    let rows = screening::design_basis(&sel, &vessel, &sized).map_err(rule_refusal)?;
    let custom = shape.custom;
    Ok(ChannelSize {
        rule_sets: input.selection_label(),
        vessel_standard_id: (!custom).then(|| shape.vessel_standard_id.clone()),
        loa_m: shape.loa_m,
        beam_m: shape.beam_m,
        draught_m: shape.draught_m,
        formation: shape.formation.clone(),
        bottom_width_straight_m: shape.bottom_width_straight_m,
        min_bend_radius_m: shape.min_bend_radius_m,
        design_depth_m: shape.design_depth_m,
        bend_widening_at_rmin_m: shape.bend_widening_at_rmin_m,
        bottom_width_at_rmin_m: shape.bottom_width_at_rmin_m,
        width_basis: shape.width_basis.clone(),
        source: shape.source.clone(),
        warnings: shape.warnings.clone(),
        vessel_label: custom.then(|| shape.vessel_standard_id.clone()),
        design_basis: rows.iter().map(BasisRow::from).collect(),
        rules_notice: RULES_NOTICE.to_string(),
        shape: Some(shape),
    })
}
