//! Run inputs (spec 0017, ticket 11): what a run aims at and must stay within, and the output level
//! it aims to reach. The document stores them as `inputs.run`.
//!
//! - The **objective** is one design measure and a sense: minimise or maximise.
//! - A **limit** is one design measure and a bound: at most or at least a value. A bound on cost is
//!   money, so it carries its currency; a bound on any other measure is a number in the measure's
//!   unit ([`DesignMeasure::unit`]) and carries none.
//! - The **target level** is concept, pre-feasibility or feasibility.
//!
//! Nothing optimises on them yet: the alignment and the structures stages read them in later tickets,
//! and the design measures stage (ticket 17) reports each limit's pass or fail and the level reached.
//! A stage that reads them gets a typed [`RunInputs`].
//!
//! A document with no `inputs.run` opens and behaves as before: it states no objective and no
//! limits, and its target reads as concept, the lowest level, so no run is ever held to a level the
//! user did not ask for. Nothing is written into the document until the user sets a run input, and
//! clearing every part removes the block again.
//!
//! The command line (`owd run-inputs`) and the app's fields both call
//! [`Session::edit_run_inputs`], with the text forms read by [`parse_objective`], [`parse_limit`]
//! and [`parse_level`].

use super::session::{DesignError, Session};
use crate::types::{Money, StageError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The name refusals carry in their `stage` field: the input's key in `inputs`.
pub const INPUT: &str = "run";

/// The name of the run inputs' schema contract: `schema/stages/inputs.run.<version>.schema.json`.
pub const CONTRACT: &str = "inputs.run";

/// One quantity of the whole design (spec 0017). Not the rules engine's Measure, which is per object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DesignMeasure {
    /// The cost: rates times volumes, plus structures (ticket 12). Money, in the rates' currency.
    Cost,
    /// Dredge plus dry cut, m³.
    TotalCut,
    /// The cut in the river, m³.
    Dredge,
    /// The cut on dry land, m³.
    DryCut,
    /// The area of the channel's top width outside the river polygon, m².
    LandTake,
    /// The design line's length, m.
    LineLength,
    /// The number of weirs and locks.
    StructureCount,
    /// The governing vessel's size. Its unit is open (spec 0017, open assumption 1: deadweight, or
    /// length times beam), so a limit on it is refused until the maintainer settles it.
    Capacity,
}

impl DesignMeasure {
    /// Every design measure, in the order the spec lists them.
    pub const ALL: [DesignMeasure; 8] = [
        DesignMeasure::Cost,
        DesignMeasure::TotalCut,
        DesignMeasure::Dredge,
        DesignMeasure::DryCut,
        DesignMeasure::LandTake,
        DesignMeasure::LineLength,
        DesignMeasure::StructureCount,
        DesignMeasure::Capacity,
    ];

    /// The name the document and the text forms use.
    pub fn name(self) -> &'static str {
        match self {
            DesignMeasure::Cost => "cost",
            DesignMeasure::TotalCut => "total_cut",
            DesignMeasure::Dredge => "dredge",
            DesignMeasure::DryCut => "dry_cut",
            DesignMeasure::LandTake => "land_take",
            DesignMeasure::LineLength => "line_length",
            DesignMeasure::StructureCount => "structure_count",
            DesignMeasure::Capacity => "capacity",
        }
    }

    /// The unit a bound on the measure is in.
    pub fn unit(self) -> &'static str {
        match self {
            DesignMeasure::Cost => "money, in the currency the limit names",
            DesignMeasure::TotalCut | DesignMeasure::Dredge | DesignMeasure::DryCut => "m³",
            DesignMeasure::LandTake => "m²",
            DesignMeasure::LineLength => "m",
            DesignMeasure::StructureCount => "structures",
            DesignMeasure::Capacity => "not settled (spec 0017, open assumption 1)",
        }
    }

    /// The measure of a name, if OWD knows it.
    pub fn from_name(name: &str) -> Option<DesignMeasure> {
        DesignMeasure::ALL.into_iter().find(|m| m.name() == name)
    }
}

/// Minimise or maximise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Sense {
    Minimise,
    Maximise,
}

/// What the run aims at: one design measure and a sense.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Objective {
    pub measure: DesignMeasure,
    pub sense: Sense,
}

/// At most or at least.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Bound {
    AtMost,
    AtLeast,
}

/// What the run must stay within: one design measure and a bound.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Limit {
    pub measure: DesignMeasure,
    pub bound: Bound,
    /// The bound, in the measure's unit. Zero or more.
    pub value: f64,
    /// ISO 4217 code of a bound on cost (three capital letters). Required for cost, absent for
    /// every other measure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
}

impl Limit {
    /// The limit as the text forms write it: `total_cut<=500000`, `cost<=2000000 USD`.
    pub fn to_text(&self) -> String {
        let op = match self.bound {
            Bound::AtMost => "<=",
            Bound::AtLeast => ">=",
        };
        let mut t = format!("{}{op}{}", self.measure.name(), self.value);
        if let Some(c) = &self.currency {
            t.push(' ');
            t.push_str(c);
        }
        t
    }
}

/// How far a run's figures can be trusted (spec 0017): the inputs each reach has decide the level it
/// reaches. Ordered from the lowest.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum OutputLevel {
    /// Banks (or a river polygon) and an open elevation model.
    Concept,
    /// Plus spot levels or a partial survey, and a rates table.
    PreFeasibility,
    /// Plus bathymetry and a ground survey covering the reach.
    Feasibility,
}

impl OutputLevel {
    /// The name as the text forms write it: `concept`, `pre-feasibility`, `feasibility`.
    pub fn label(self) -> &'static str {
        match self {
            OutputLevel::Concept => "concept",
            OutputLevel::PreFeasibility => "pre-feasibility",
            OutputLevel::Feasibility => "feasibility",
        }
    }
}

/// The run inputs, as `inputs.run` stores them. Every part is optional; a part not stated is absent
/// from the JSON, so what was read writes back the same.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunInputs {
    /// What the run aims at. Absent: no objective stated; each method keeps its own rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective: Option<Objective>,
    /// What the run must stay within. At most one bound of each kind on a measure.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limits: Vec<Limit>,
    /// The output level the run aims at. Absent: concept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_level: Option<OutputLevel>,
}

fn refuse(code: &str, message: impl AsRef<str>) -> StageError {
    StageError::new(INPUT, code, message.as_ref())
}

fn known_measures() -> String {
    DesignMeasure::ALL
        .iter()
        .map(|m| m.name())
        .collect::<Vec<_>>()
        .join(", ")
}

fn measure_of(name: &str) -> Result<DesignMeasure, StageError> {
    DesignMeasure::from_name(name).ok_or_else(|| {
        refuse(
            "RUN_MEASURE_UNKNOWN",
            format!(
                "`{name}` is not a design measure OWD knows; use one of {}",
                known_measures()
            ),
        )
    })
}

impl RunInputs {
    /// The level the run aims at: the one stated, or concept.
    pub fn target(&self) -> OutputLevel {
        self.target_level.unwrap_or(OutputLevel::Concept)
    }

    /// True when nothing is stated.
    pub fn is_empty(&self) -> bool {
        self.objective.is_none() && self.limits.is_empty() && self.target_level.is_none()
    }

    /// Check the rules JSON Schema cannot state.
    ///
    /// Refusals: `RUN_LIMIT_UNIT_OPEN` (a bound on capacity, whose unit is not settled),
    /// `RUN_LIMIT_VALUE` (a bound that is not a finite number of zero or more),
    /// `RUN_LIMIT_CURRENCY` (a cost bound with no valid currency, another measure's bound with
    /// one, or cost bounds in two currencies), `RUN_LIMIT_DUPLICATE` (two bounds of one kind on one
    /// measure) and `RUN_LIMITS_CONTRADICT` (an at-least bound above the at-most bound).
    pub fn validate(&self) -> Result<(), StageError> {
        let mut cost_currency: Option<&str> = None;
        for l in &self.limits {
            let name = l.measure.name();
            if l.measure == DesignMeasure::Capacity {
                return Err(refuse(
                    "RUN_LIMIT_UNIT_OPEN",
                    "a limit on capacity needs its unit, which is not settled yet (spec 0017, \
                     open assumption 1); capacity can be the objective",
                ));
            }
            if !(l.value.is_finite() && l.value >= 0.0) {
                return Err(refuse(
                    "RUN_LIMIT_VALUE",
                    format!(
                        "the bound on {name} is {}; a design measure is zero or more",
                        l.value
                    ),
                ));
            }
            match (l.measure, l.currency.as_deref()) {
                (DesignMeasure::Cost, None) => {
                    return Err(refuse(
                        "RUN_LIMIT_CURRENCY",
                        "a bound on cost needs its currency, for example `cost<=2000000 USD`",
                    ));
                }
                (DesignMeasure::Cost, Some(c)) => {
                    Money::new(l.value, c).map_err(|e| refuse("RUN_LIMIT_CURRENCY", e))?;
                    match cost_currency {
                        Some(first) if first != c => {
                            return Err(refuse(
                                "RUN_LIMIT_CURRENCY",
                                format!(
                                    "the bounds on cost are in {first} and {c}; give them in one \
                                     currency"
                                ),
                            ));
                        }
                        _ => cost_currency = Some(c),
                    }
                }
                (_, Some(c)) => {
                    return Err(refuse(
                        "RUN_LIMIT_CURRENCY",
                        format!(
                            "only cost is money: the bound on {name} is in {}, not {c}",
                            l.measure.unit()
                        ),
                    ));
                }
                (_, None) => {}
            }
        }
        for (i, a) in self.limits.iter().enumerate() {
            for b in &self.limits[i + 1..] {
                if a.measure != b.measure {
                    continue;
                }
                let name = a.measure.name();
                if a.bound == b.bound {
                    return Err(refuse(
                        "RUN_LIMIT_DUPLICATE",
                        format!(
                            "two bounds of one kind on {name} ({} and {}); give one",
                            a.to_text(),
                            b.to_text()
                        ),
                    ));
                }
                let (most, least) = if a.bound == Bound::AtMost {
                    (a, b)
                } else {
                    (b, a)
                };
                if least.value > most.value {
                    return Err(refuse(
                        "RUN_LIMITS_CONTRADICT",
                        format!(
                            "no design meets both {} and {}",
                            least.to_text(),
                            most.to_text()
                        ),
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Read and check a run-inputs block from JSON. A measure, a sense or a level OWD does not know is
/// refused by name (`RUN_MEASURE_UNKNOWN`, `RUN_SENSE_UNKNOWN`, `RUN_LEVEL_UNKNOWN`); any other
/// shape that is not a run-inputs block, a misspelt key included, is `RUN_INPUT_INVALID`; then the
/// rules of [`RunInputs::validate`].
pub fn read_run_inputs(v: &Value) -> Result<RunInputs, StageError> {
    let mut measures: Vec<&Value> = vec![&v["objective"]["measure"]];
    if let Some(ls) = v["limits"].as_array() {
        measures.extend(ls.iter().map(|l| &l["measure"]));
    }
    for m in measures {
        if let Some(name) = m.as_str() {
            measure_of(name)?;
        }
    }
    if let Some(s) = v["objective"]["sense"].as_str()
        && serde_json::from_value::<Sense>(Value::String(s.into())).is_err()
    {
        return Err(refuse(
            "RUN_SENSE_UNKNOWN",
            format!("an objective is `minimise` or `maximise`, not `{s}`"),
        ));
    }
    if let Some(l) = v["target_level"].as_str()
        && serde_json::from_value::<OutputLevel>(Value::String(l.into())).is_err()
    {
        return Err(level_unknown(l));
    }
    let run: RunInputs = serde_json::from_value(v.clone())
        .map_err(|e| refuse("RUN_INPUT_INVALID", format!("not run inputs: {e}")))?;
    run.validate()?;
    Ok(run)
}

fn level_unknown(l: &str) -> StageError {
    refuse(
        "RUN_LEVEL_UNKNOWN",
        format!("an output level is concept, pre-feasibility or feasibility, not `{l}`"),
    )
}

/// Read an objective as the command line and the app give it: `SENSE:MEASURE` or `SENSE MEASURE`,
/// for example `minimise:cost`. An empty text or `none` states no objective.
pub fn parse_objective(text: &str) -> Result<Option<Objective>, StageError> {
    let t = text.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    let parts: Vec<&str> = if t.contains(':') {
        t.split(':').map(str::trim).collect()
    } else {
        t.split_whitespace().collect()
    };
    let [sense, measure] = parts[..] else {
        return Err(refuse(
            "RUN_OBJECTIVE_SYNTAX",
            format!("an objective is `SENSE:MEASURE`, for example `minimise:cost`, not `{t}`"),
        ));
    };
    let sense = match sense.to_ascii_lowercase().as_str() {
        "minimise" | "minimize" => Sense::Minimise,
        "maximise" | "maximize" => Sense::Maximise,
        _ => {
            return Err(refuse(
                "RUN_SENSE_UNKNOWN",
                format!("an objective is `minimise` or `maximise`, not `{sense}`"),
            ));
        }
    };
    Ok(Some(Objective {
        measure: measure_of(measure)?,
        sense,
    }))
}

/// Read a limit as the command line and the app give it: `MEASURE<=VALUE` or `MEASURE>=VALUE`, and
/// for cost the currency after a space: `cost<=2000000 USD`. The rules are checked when the limits
/// are set ([`RunInputs::validate`]).
pub fn parse_limit(text: &str) -> Result<Limit, StageError> {
    let t = text.trim();
    let syntax = || {
        refuse(
            "RUN_LIMIT_SYNTAX",
            format!(
                "a limit is `MEASURE<=VALUE` or `MEASURE>=VALUE`, with the currency after a cost \
                 (`cost<=2000000 USD`), not `{t}`"
            ),
        )
    };
    let (at, bound) = match (t.find("<="), t.find(">=")) {
        (Some(i), None) => (i, Bound::AtMost),
        (None, Some(i)) => (i, Bound::AtLeast),
        _ => return Err(syntax()),
    };
    let measure = measure_of(t[..at].trim())?;
    let rest: Vec<&str> = t[at + 2..].split_whitespace().collect();
    let (value, currency) = match rest[..] {
        [v] => (v, None),
        [v, c] => (v, Some(c.to_string())),
        _ => return Err(syntax()),
    };
    let value: f64 = value.parse().map_err(|_| syntax())?;
    Ok(Limit {
        measure,
        bound,
        value,
        currency,
    })
}

/// Read a target level as the command line and the app give it: `concept`, `pre-feasibility` (or
/// `pre_feasibility`) or `feasibility`, in any case. An empty text states none.
pub fn parse_level(text: &str) -> Result<Option<OutputLevel>, StageError> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(None);
    }
    match t.to_ascii_lowercase().replace('-', "_").as_str() {
        "concept" => Ok(Some(OutputLevel::Concept)),
        "pre_feasibility" => Ok(Some(OutputLevel::PreFeasibility)),
        "feasibility" => Ok(Some(OutputLevel::Feasibility)),
        _ => Err(level_unknown(t)),
    }
}

/// A change to the run inputs, as the text forms give it. `None` keeps that part as it is; an empty
/// text (or an empty list of limits) clears it.
#[derive(Debug, Clone, Copy, Default)]
pub struct RunInputsEdit<'a> {
    pub objective: Option<&'a str>,
    pub limits: Option<&'a [String]>,
    pub target_level: Option<&'a str>,
}

fn input_error(e: StageError) -> DesignError {
    DesignError::Input(format!("{}: {}", e.code, e.message))
}

impl Session {
    /// The run inputs the document states, or `None` when it states none.
    pub fn run_inputs(&self) -> Result<Option<RunInputs>, DesignError> {
        match self.doc["inputs"].get(INPUT) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => read_run_inputs(v).map(Some).map_err(input_error),
        }
    }

    /// Store the run inputs, checked. `None`, or inputs that state nothing, remove the block. No
    /// stage reads them yet, so no stage goes stale. Nothing changes when they are refused.
    pub fn set_run_inputs(&mut self, run: Option<RunInputs>) -> Result<(), DesignError> {
        let inputs = self.doc["inputs"]
            .as_object_mut()
            .ok_or_else(|| DesignError::Input("the document has no inputs".into()))?;
        match run.filter(|r| !r.is_empty()) {
            None => {
                inputs.shift_remove(INPUT);
            }
            Some(r) => {
                r.validate().map_err(input_error)?;
                inputs.insert(
                    INPUT.into(),
                    serde_json::to_value(&r).expect("run inputs serialise"),
                );
            }
        }
        Ok(())
    }

    /// Change the run inputs from the text forms: the one function the command line and the app
    /// call. Each part named replaces that part; a part not named is kept. Returns what is stored
    /// now. Nothing changes when any part is refused.
    pub fn edit_run_inputs(&mut self, edit: &RunInputsEdit) -> Result<RunInputs, DesignError> {
        let mut run = self.run_inputs()?.unwrap_or_default();
        if let Some(o) = edit.objective {
            run.objective = parse_objective(o).map_err(input_error)?;
        }
        if let Some(ls) = edit.limits {
            run.limits = ls
                .iter()
                .map(|l| parse_limit(l))
                .collect::<Result<_, _>>()
                .map_err(input_error)?;
        }
        if let Some(l) = edit.target_level {
            run.target_level = parse_level(l).map_err(input_error)?;
        }
        self.set_run_inputs(Some(run.clone()))?;
        Ok(run)
    }
}
