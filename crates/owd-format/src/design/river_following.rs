//! The river-following line as an alignment method (spec OPT-4): its settings, the record it
//! writes into the design line, and the method itself over `owd_core::alignment::river_following`.
//!
//! Every length the method uses comes from the sizing stage (R_min, the vessel's length, the
//! bottom width): the settings are ratios, or lengths that default to a multiple of those. So a
//! change of vessel, rule set or rule quality changes the size, which makes the alignment stale,
//! and the next run builds the line again from the river.

use super::design_line::{AlignmentIn, DesignLine, finish, radius_checks};
use super::session::DesignError;
use crate::types::{Chainage, StageError, TerrainSampler};
use owd_core::alignment::radius_check::default_check_chord;
use owd_core::alignment::river_following::elements::Chain;
use owd_core::alignment::river_following::mask::RiverMask;
use owd_core::alignment::river_following::seed::{Seed, SeedAim, SeedParams, seed};
use owd_core::geom::Pt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

/// The name of the method in the settings and the line.
pub const METHOD: &str = "river_following";

/// What the seed aims at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Aim {
    /// The river centreline (the default).
    #[default]
    Centreline,
    /// The river's water: at a bend tighter than R_min, the arc that keeps most of the line in it.
    InWater,
}

/// The settings of the river-following line, typed. Each length left unset is worked out from the
/// sizing stage when the line is built, and the line records the value it used.
#[derive(Debug, Clone, PartialEq, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct RiverFollowingSettings {
    /// The seed's least radius as a multiple of R_min.
    pub seed_clip_ratio: f64,
    /// The radius of a cut-through's arcs as a multiple of R_min.
    pub cut_radius_ratio: f64,
    /// The sideways shift's radius limit as a multiple of R_min.
    pub shift_limit_ratio: f64,
    /// The straight between two opposite arcs when either is tighter than 2 R_min, in metres;
    /// unset: the vessel's length.
    pub min_straight_m: Option<f64>,
    /// The largest gap of the seed from the river centreline outside a bend tighter than R_min, in
    /// metres.
    pub seed_tolerance_m: f64,
    /// The moving average of the centreline before the seed is fitted, in metres; unset: R_min / 6.
    pub seed_presmooth_m: Option<f64>,
    /// What the seed aims at.
    pub seed_aim: Aim,
    /// A cut-through saves at least this length, in metres; unset: twice the vessel's length.
    pub cut_min_save_m: Option<f64>,
    /// The river stretch a cut-through replaces is at least this many times the cut's length.
    pub cut_min_ratio: f64,
    /// Each arc of a cut-through turns at most this, in degrees.
    pub cut_max_arc_turn_deg: f64,
    /// A cut-through is taken only when it is at least this share cheaper than staying in the
    /// river (a tie goes to the river).
    pub cut_min_gain: f64,
    /// A cut-through whose tallest face is over this (and over staying's) is refused, in metres.
    pub face_max_m: f64,
    /// Cut-throughs forced on (`true`) or off (`false`) by id; this wins over the rule.
    pub cut_force: BTreeMap<String, bool>,
    /// The river polygon grown (+) or shrunk (−) for the in-river tests, in metres.
    pub in_river_margin_m: f64,
    /// The room either side of an accepted cut-through the shift may use, in metres; unset: twice
    /// the bottom width.
    pub cut_through_room_m: Option<f64>,
    /// Run the sideways shift.
    pub shift: bool,
    /// The largest sideways offset, in metres; unset: the larger of 150 m and R_min / 2.
    pub shift_reach_m: Option<f64>,
    /// The shift's station spacing, in metres; unset: R_min / 6, from 20 to 50 m.
    pub shift_station_m: Option<f64>,
    /// The shift's offset step, in metres; unset: from the station spacing and R_min (spec 4.8).
    pub shift_offset_step_m: Option<f64>,
    /// The steepest sideways slope of the shift.
    pub shift_max_slope: f64,
    /// Refine the shift after the dynamic programme.
    pub shift_refine: bool,
    /// Keep the seed's curves and straights in the shift (spec 4.5).
    pub same_curves: bool,
    /// The bottom widths the shift's cost tables are built at, to price bend widening.
    pub widening_levels: u32,
    /// A price on turning in the shift, per metre of line per unit of R_min × curvature (0: off).
    pub turn_weight: f64,
    /// The chord the radius check reads on; unset: R_min / 3.
    pub check_chord_m: Option<f64>,
}

impl Default for RiverFollowingSettings {
    fn default() -> Self {
        RiverFollowingSettings {
            seed_clip_ratio: 1.0333,
            cut_radius_ratio: 1.0556,
            shift_limit_ratio: 1.0694,
            min_straight_m: None,
            seed_tolerance_m: 2.0,
            seed_presmooth_m: None,
            seed_aim: Aim::Centreline,
            cut_min_save_m: None,
            cut_min_ratio: 1.3,
            cut_max_arc_turn_deg: 100.0,
            cut_min_gain: 0.05,
            face_max_m: 30.0,
            cut_force: BTreeMap::new(),
            in_river_margin_m: -3.0,
            cut_through_room_m: None,
            shift: true,
            shift_reach_m: None,
            shift_station_m: None,
            shift_offset_step_m: None,
            shift_max_slope: 0.30,
            shift_refine: true,
            same_curves: true,
            widening_levels: 4,
            turn_weight: 0.0,
            check_chord_m: None,
        }
    }
}

/// Written only where a setting is not its default, so the document says what the user chose.
impl Serialize for RiverFollowingSettings {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.to_value().serialize(s)
    }
}

impl RiverFollowingSettings {
    /// The settings that are not their default, as the document stores them.
    pub fn to_value(&self) -> Value {
        let d = RiverFollowingSettings::default();
        let full = self.full();
        let def = d.full();
        let mut out = Map::new();
        if let (Value::Object(f), Value::Object(df)) = (full, def) {
            for (k, v) in f {
                if df.get(&k) != Some(&v) {
                    out.insert(k, v);
                }
            }
        }
        Value::Object(out)
    }

    /// Every setting, as used.
    pub fn full(&self) -> Value {
        json!({
            "seed_clip_ratio": self.seed_clip_ratio,
            "cut_radius_ratio": self.cut_radius_ratio,
            "shift_limit_ratio": self.shift_limit_ratio,
            "min_straight_m": self.min_straight_m,
            "seed_tolerance_m": self.seed_tolerance_m,
            "seed_presmooth_m": self.seed_presmooth_m,
            "seed_aim": self.seed_aim,
            "cut_min_save_m": self.cut_min_save_m,
            "cut_min_ratio": self.cut_min_ratio,
            "cut_max_arc_turn_deg": self.cut_max_arc_turn_deg,
            "cut_min_gain": self.cut_min_gain,
            "face_max_m": self.face_max_m,
            "cut_force": self.cut_force,
            "in_river_margin_m": self.in_river_margin_m,
            "cut_through_room_m": self.cut_through_room_m,
            "shift": self.shift,
            "shift_reach_m": self.shift_reach_m,
            "shift_station_m": self.shift_station_m,
            "shift_offset_step_m": self.shift_offset_step_m,
            "shift_max_slope": self.shift_max_slope,
            "shift_refine": self.shift_refine,
            "same_curves": self.same_curves,
            "widening_levels": self.widening_levels,
            "turn_weight": self.turn_weight,
            "check_chord_m": self.check_chord_m,
        })
    }

    /// True when every setting is its default.
    pub fn is_default(&self) -> bool {
        *self == RiverFollowingSettings::default()
    }

    /// Lay `source` (an object or null) over these settings. A key the method does not know, or a
    /// bad value, is refused by name.
    pub fn apply(&self, source: &Value) -> Result<RiverFollowingSettings, DesignError> {
        let mut out = self.clone();
        let Some(o) = source.as_object() else {
            return Ok(out);
        };
        let bad = |k: &str, what: &str| {
            DesignError::Input(format!("the river-following setting `{k}` must be {what}"))
        };
        let pos = |k: &str, v: &Value| -> Result<f64, DesignError> {
            v.as_f64()
                .filter(|x| x.is_finite() && *x > 0.0)
                .ok_or_else(|| bad(k, "a number above 0"))
        };
        let opt_pos = |k: &str, v: &Value| -> Result<Option<f64>, DesignError> {
            match v {
                Value::Null => Ok(None),
                v => pos(k, v).map(Some),
            }
        };
        let flag = |k: &str, v: &Value| v.as_bool().ok_or_else(|| bad(k, "true or false"));
        for (k, v) in o {
            match k.as_str() {
                "seed_clip_ratio" | "cut_radius_ratio" | "shift_limit_ratio" => {
                    let x = v
                        .as_f64()
                        .filter(|x| x.is_finite() && *x >= 1.0)
                        .ok_or_else(|| bad(k, "a number at or above 1 (a multiple of R_min)"))?;
                    match k.as_str() {
                        "seed_clip_ratio" => out.seed_clip_ratio = x,
                        "cut_radius_ratio" => out.cut_radius_ratio = x,
                        _ => out.shift_limit_ratio = x,
                    }
                }
                "min_straight_m" => {
                    out.min_straight_m = match v {
                        Value::Null => None,
                        v => Some(
                            v.as_f64()
                                .filter(|x| x.is_finite() && *x >= 0.0)
                                .ok_or_else(|| bad(k, "a number at or above 0, or null"))?,
                        ),
                    }
                }
                "seed_tolerance_m" => out.seed_tolerance_m = pos(k, v)?,
                "seed_presmooth_m" => out.seed_presmooth_m = opt_pos(k, v)?,
                "seed_aim" => {
                    out.seed_aim = serde_json::from_value(v.clone())
                        .map_err(|_| bad(k, "`centreline` or `in_water`"))?
                }
                "cut_min_save_m" => out.cut_min_save_m = opt_pos(k, v)?,
                "cut_min_ratio" => {
                    out.cut_min_ratio = v
                        .as_f64()
                        .filter(|x| x.is_finite() && *x >= 1.0)
                        .ok_or_else(|| bad(k, "a number at or above 1"))?
                }
                "cut_max_arc_turn_deg" => {
                    out.cut_max_arc_turn_deg = v
                        .as_f64()
                        .filter(|x| *x > 0.0 && *x <= 180.0)
                        .ok_or_else(|| bad(k, "a number of degrees above 0 and at most 180"))?
                }
                "cut_min_gain" => {
                    out.cut_min_gain = v
                        .as_f64()
                        .filter(|x| (0.0..1.0).contains(x))
                        .ok_or_else(|| bad(k, "a share from 0 to under 1 (0.05 is 5 %)"))?
                }
                "face_max_m" => out.face_max_m = pos(k, v)?,
                "cut_force" => {
                    let m = v.as_object().ok_or_else(|| {
                        bad(
                            k,
                            "an object of cut-through ids, each true (on) or false (off)",
                        )
                    })?;
                    let mut f = BTreeMap::new();
                    for (id, on) in m {
                        let on = match on {
                            Value::Bool(b) => *b,
                            Value::String(s) if s == "on" => true,
                            Value::String(s) if s == "off" => false,
                            _ => {
                                return Err(bad(
                                    k,
                                    "an object of ids, each on (true) or off (false)",
                                ));
                            }
                        };
                        f.insert(id.clone(), on);
                    }
                    out.cut_force = f;
                }
                "in_river_margin_m" => {
                    out.in_river_margin_m = v
                        .as_f64()
                        .filter(|x| x.is_finite())
                        .ok_or_else(|| bad(k, "a number (negative: inside the bank)"))?
                }
                "cut_through_room_m" => out.cut_through_room_m = opt_pos(k, v)?,
                "shift" => out.shift = flag(k, v)?,
                "shift_reach_m" => out.shift_reach_m = opt_pos(k, v)?,
                "shift_station_m" => out.shift_station_m = opt_pos(k, v)?,
                "shift_offset_step_m" => out.shift_offset_step_m = opt_pos(k, v)?,
                "shift_max_slope" => {
                    out.shift_max_slope = v
                        .as_f64()
                        .filter(|x| *x > 0.0 && *x <= 1.0)
                        .ok_or_else(|| bad(k, "a number above 0 and at most 1"))?
                }
                "shift_refine" => out.shift_refine = flag(k, v)?,
                "same_curves" => out.same_curves = flag(k, v)?,
                "widening_levels" => {
                    out.widening_levels = v
                        .as_u64()
                        .filter(|x| (1..=8).contains(x))
                        .ok_or_else(|| bad(k, "a whole number from 1 to 8"))?
                        as u32
                }
                "turn_weight" => {
                    out.turn_weight = v
                        .as_f64()
                        .filter(|x| x.is_finite() && *x >= 0.0)
                        .ok_or_else(|| bad(k, "a number at or above 0"))?
                }
                "check_chord_m" => out.check_chord_m = opt_pos(k, v)?,
                other => {
                    return Err(DesignError::Input(format!(
                        "the river-following line has no setting `{other}`"
                    )));
                }
            }
        }
        Ok(out)
    }
}

/// The lengths the method used, worked out from the sizing stage and the settings. The line
/// records them, so a reader sees that each came from R_min, the vessel or the bottom width.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Derived {
    pub r_min_m: f64,
    pub loa_m: f64,
    pub bottom_width_m: f64,
    /// The seed's least radius.
    pub r_clip_m: f64,
    /// The radius of a cut-through's arcs.
    pub r_cut_m: f64,
    /// The shift's radius limit.
    pub r_shift_m: f64,
    pub min_straight_m: f64,
    /// Opposite arcs tighter than this need the minimum straight (2 R_min).
    pub tight_reverse_r_m: f64,
    pub seed_presmooth_m: f64,
    pub seed_step_m: f64,
    /// A bend of the river curves at least this much, 1/m (1 / 8 R_min).
    pub curve_threshold_per_m: f64,
    pub cut_min_save_m: f64,
    pub cut_through_room_m: f64,
    /// A stretch of the line out of the river is recorded when at least this long (2 × LOA).
    pub off_river_min_m: f64,
    pub check_chord_m: f64,
}

impl Derived {
    fn of(s: &RiverFollowingSettings, input: &AlignmentIn) -> Derived {
        let r_min = input.size.min_bend_radius_m;
        let loa = input.size.loa_m;
        let bw = input.size.bottom_width_straight_m;
        let p = SeedParams::scaled(r_min, s.seed_clip_ratio, 0.0, s.seed_tolerance_m);
        Derived {
            r_min_m: r_min,
            loa_m: loa,
            bottom_width_m: bw,
            r_clip_m: s.seed_clip_ratio * r_min,
            r_cut_m: s.cut_radius_ratio * r_min,
            r_shift_m: s.shift_limit_ratio * r_min,
            min_straight_m: s.min_straight_m.unwrap_or(loa),
            tight_reverse_r_m: 2.0 * r_min,
            seed_presmooth_m: s.seed_presmooth_m.unwrap_or(p.presmooth_m),
            seed_step_m: p.step_m,
            curve_threshold_per_m: p.curve_threshold,
            cut_min_save_m: s.cut_min_save_m.unwrap_or(2.0 * loa),
            cut_through_room_m: s.cut_through_room_m.unwrap_or(2.0 * bw),
            off_river_min_m: 2.0 * loa,
            check_chord_m: s
                .check_chord_m
                .unwrap_or_else(|| default_check_chord(r_min)),
        }
    }

    fn seed_params(&self, s: &RiverFollowingSettings) -> SeedParams {
        let mut p = SeedParams::scaled(
            self.r_min_m,
            s.seed_clip_ratio,
            self.min_straight_m,
            s.seed_tolerance_m,
        );
        p.presmooth_m = self.seed_presmooth_m;
        p.aim = match s.seed_aim {
            Aim::Centreline => SeedAim::Centreline,
            Aim::InWater => SeedAim::InWater,
        };
        p
    }
}

/// One element of a line: a straight or an arc.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ElementRecord {
    /// `straight` or `arc`.
    pub kind: String,
    /// Design-line chainage of its start, m.
    pub start: Chainage,
    /// Its length, m.
    pub length_m: f64,
    /// Its radius, m; null for a straight.
    pub radius_m: Option<f64>,
    /// `left`, `right`, or null for a straight.
    pub hand: Option<String>,
    /// Its start point and heading (degrees counter-clockwise from +x).
    pub start_point: [f64; 2],
    pub start_heading_deg: f64,
}

/// The elements of a chain, as the record writes them.
pub fn element_records(chain: &Chain) -> Vec<ElementRecord> {
    chain
        .elements
        .iter()
        .map(|e| ElementRecord {
            kind: if e.is_arc() { "arc" } else { "straight" }.into(),
            start: Chainage(e.s0),
            length_m: e.length,
            radius_m: e.is_arc().then(|| e.radius()),
            hand: match e.hand() {
                1 => Some("left".into()),
                -1 => Some("right".into()),
                _ => None,
            },
            start_point: [e.start.0, e.start.1],
            start_heading_deg: e.start.2.to_degrees(),
        })
        .collect()
}

/// The seed, as the line records it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SeedRecord {
    pub elements: Vec<ElementRecord>,
    pub element_count: usize,
    pub arc_count: usize,
    pub least_radius_m: Option<f64>,
    /// The largest gap from the river centreline (smoothed) outside the tight bends, m.
    pub gap_m: f64,
    /// The largest gap anywhere, the tight bends too, m.
    pub gap_all_m: f64,
    /// Refinement passes, and whether the gap outside the tight bends is within the tolerance.
    pub passes: usize,
    pub settled: bool,
}

/// A stretch of the line the river forces: a bend tighter than the seed may follow, or a
/// stretch out of the river not chosen as a cut-through.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ForcedStretch {
    /// A stable name: `forced-` and the river chainage of its middle, to 100 m.
    pub id: String,
    /// `tight_bend`: the river is tighter than R_min allows; `straight`: the seed left the river
    /// to hold the minimum straight between opposite arcs; `off_river`: the line's centre is out
    /// of the river (with the margin) for at least `off_river_min_m`.
    pub kind: String,
    /// Design-line chainage, from and to.
    pub from: Chainage,
    pub to: Chainage,
    /// River chainage, from and to.
    pub river_from: Chainage,
    pub river_to: Chainage,
    /// The river's tightest radius there (a tight bend), m.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub river_radius_m: Option<f64>,
    /// The farthest the line is from the river centreline in the stretch, m.
    pub farthest_from_centreline_m: f64,
}

/// What the river-following line records about itself (spec OPT-4, section 6). The window optimiser
/// (OPT-5) reads [`RiverFollowingRecord::permitted_stretches`] from it for its permitted ground.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RiverFollowingRecord {
    /// The settings, every one as used.
    pub settings: Value,
    /// The lengths worked out from the sizing.
    pub derived: Derived,
    pub seed: SeedRecord,
    /// The forced stretches, in chainage order.
    pub forced: Vec<ForcedStretch>,
    /// The delivered line's elements.
    pub elements: Vec<ElementRecord>,
    /// Warnings in words.
    pub warnings: Vec<String>,
}

/// A stretch of the line, for the window optimiser's permitted ground (OPT-5 section 3.16).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PermittedStretch {
    /// The cut-through's or the forced stretch's id.
    pub id: String,
    /// `cut_through` (room either side: `cut_through_room_m`) or `forced` (room: half the bottom
    /// width).
    pub kind: String,
    pub from: Chainage,
    pub to: Chainage,
    /// The room either side of the line the optimiser may use, m.
    pub room_m: f64,
}

impl RiverFollowingRecord {
    /// The stretches of the line out of the river the method chose or was forced into, each with
    /// the room the window optimiser may use about it.
    pub fn permitted_stretches(&self) -> Vec<PermittedStretch> {
        self.forced
            .iter()
            .map(|f| PermittedStretch {
                id: f.id.clone(),
                kind: "forced".into(),
                from: f.from,
                to: f.to,
                room_m: self.derived.bottom_width_m / 2.0,
            })
            .collect()
    }
}

/// The record a design line holds, when it is a river-following line.
pub fn record_of(line: &DesignLine) -> Option<RiverFollowingRecord> {
    line.river_following.clone()
}

fn refuse(code: &str, message: impl Into<String>) -> StageError {
    StageError::new(super::design_line::STAGE, code, &message.into())
}

/// A name for a river chainage, to 100 m.
fn hm(c: f64) -> String {
    format!("{:05}", (c / 100.0).round().max(0.0) as i64)
}

/// The river-following line of `input`.
pub fn river_following(
    input: &AlignmentIn,
    _terrain: Option<TerrainSampler>,
) -> Result<DesignLine, StageError> {
    let s = &input.settings.river_following;
    let centre = &input.river.centreline.0;
    if centre.len() < 2 {
        return Err(refuse(
            "ALIGN_NO_LINE",
            "the river-following line needs the river centreline",
        ));
    }
    let r_min = input.size.min_bend_radius_m;
    if !(r_min.is_finite() && r_min > 0.0) {
        return Err(refuse(
            "ALIGN_SETTING",
            "the river-following line needs R_min from the sizing stage",
        ));
    }
    let d = Derived::of(s, input);
    let p = d.seed_params(s);
    let mut warnings = Vec::new();
    let mask = input
        .river
        .polygon
        .as_ref()
        .map(|poly| RiverMask::new(&[poly.to_polygon()], s.in_river_margin_m));
    if mask.is_none() {
        warnings.push(
            "the design has no river polygon: the in-river tests are off, so no stretch out of \
             the river is found or recorded"
                .to_string(),
        );
    }
    let in_water = mask.as_ref().map(|m| move |q: Pt| m.contains(q));
    let in_water_dyn: Option<&dyn Fn(Pt) -> bool> =
        in_water.as_ref().map(|f| f as &dyn Fn(Pt) -> bool);
    let sd = seed(centre, &p, in_water_dyn);
    if !sd.settled {
        warnings.push(format!(
            "the seed is up to {:.1} m from the river centreline outside the bends tighter than \
             R_min (the tolerance is {:.1} m)",
            sd.gap_m, s.seed_tolerance_m
        ));
    }
    let chain = sd.chain.clone();
    let line = chain.points();
    let forced = forced_stretches(&sd, &chain, centre, mask.as_ref(), &d);
    let record = RiverFollowingRecord {
        settings: s.full(),
        derived: d.clone(),
        seed: seed_record(&sd),
        forced,
        elements: element_records(&chain),
        warnings: warnings.clone(),
    };
    let checks = radius_checks(&line, r_min, Some(d.check_chord_m))?;
    let m = super::alignment::AlignMethod::RiverFollowing;
    let mut out = finish(
        m.option_id(),
        m.option_name(),
        m.name(),
        s.full(),
        line,
        checks,
        json!({
            "element_count": chain.elements.len(),
            "arc_count": chain.elements.iter().filter(|e| e.is_arc()).count(),
            "seed_gap_m": sd.gap_m,
            "forced_stretches": record.forced.len(),
            "warnings": warnings,
        }),
        &input.river,
    );
    out.river_following = Some(record);
    Ok(out)
}

fn seed_record(sd: &Seed) -> SeedRecord {
    let lr = sd.chain.least_radius();
    SeedRecord {
        elements: element_records(&sd.chain),
        element_count: sd.chain.elements.len(),
        arc_count: sd.chain.elements.iter().filter(|e| e.is_arc()).count(),
        least_radius_m: lr.is_finite().then_some(lr),
        gap_m: sd.gap_m,
        gap_all_m: sd.gap_all_m,
        passes: sd.passes,
        settled: sd.settled,
    }
}

/// The farthest `line`'s points from `from` to `to` (chainage) are from the centreline.
fn farthest(chain: &Chain, centre: &[Pt], from: f64, to: f64) -> f64 {
    let n = (((to - from) / 5.0).ceil() as usize).max(1);
    (0..=n)
        .map(|i| {
            let q = chain.pose_at(from + (to - from) * i as f64 / n as f64);
            owd_core::geom::linear::point_line_distance([q.0, q.1], centre)
        })
        .fold(0.0, f64::max)
}

/// The forced stretches of the line: the seed's tight bends, then the stretches out of the river
/// (with the margin) at least `off_river_min_m` long that no tight bend explains.
fn forced_stretches(
    sd: &Seed,
    chain: &Chain,
    centre: &[Pt],
    mask: Option<&RiverMask>,
    d: &Derived,
) -> Vec<ForcedStretch> {
    let river_at = |s: f64| {
        let q = chain.pose_at(s);
        owd_core::geom::linear::project(centre, [q.0, q.1])
    };
    let mut out: Vec<ForcedStretch> = sd
        .tight
        .iter()
        .map(|t| {
            let (ra, rb) = (river_at(t.from), river_at(t.to));
            ForcedStretch {
                id: format!("forced-{}", hm(0.5 * (ra + rb))),
                kind: if t.reason == "tight" {
                    "tight_bend"
                } else {
                    "straight"
                }
                .into(),
                from: Chainage(t.from),
                to: Chainage(t.to),
                river_from: Chainage(ra.min(rb)),
                river_to: Chainage(ra.max(rb)),
                river_radius_m: Some(t.river_radius_m),
                farthest_from_centreline_m: farthest(chain, centre, t.from, t.to),
            }
        })
        .collect();
    if let Some(m) = mask {
        for (a, b) in off_river_runs(chain, m, d.off_river_min_m) {
            // Out of the river round a tight bend: the bend's stretch grows to hold it.
            if let Some(t) = out
                .iter_mut()
                .find(|t| t.kind != "off_river" && a <= t.to.0 && b >= t.from.0)
            {
                t.from = Chainage(t.from.0.min(a));
                t.to = Chainage(t.to.0.max(b));
                let (ra, rb) = (river_at(t.from.0), river_at(t.to.0));
                t.river_from = Chainage(ra.min(rb));
                t.river_to = Chainage(ra.max(rb));
                t.farthest_from_centreline_m = farthest(chain, centre, t.from.0, t.to.0);
                continue;
            }
            let (ra, rb) = (river_at(a), river_at(b));
            out.push(ForcedStretch {
                id: format!("forced-{}", hm(0.5 * (ra + rb))),
                kind: "off_river".into(),
                from: Chainage(a),
                to: Chainage(b),
                river_from: Chainage(ra.min(rb)),
                river_to: Chainage(ra.max(rb)),
                river_radius_m: None,
                farthest_from_centreline_m: farthest(chain, centre, a, b),
            });
        }
    }
    out.sort_by(|x, y| x.from.0.total_cmp(&y.from.0));
    out
}

/// The runs of `chain` whose centre is out of the river (with the margin) for at least `min_m`,
/// read every 5 m: `(from, to)` chainage.
pub fn off_river_runs(chain: &Chain, mask: &RiverMask, min_m: f64) -> Vec<(f64, f64)> {
    let total = chain.length();
    let n = ((total / 5.0).ceil() as usize).max(1);
    let mut out = Vec::new();
    let mut start: Option<f64> = None;
    let mut last = 0.0;
    for i in 0..=n {
        let s = total * i as f64 / n as f64;
        let q = chain.pose_at(s);
        let outside = !mask.contains([q.0, q.1]);
        match (outside, start) {
            (true, None) => start = Some(s),
            (false, Some(a)) => {
                if last - a >= min_m {
                    out.push((a, last));
                }
                start = None;
            }
            _ => {}
        }
        last = s;
    }
    if let Some(a) = start
        && total - a >= min_m
    {
        out.push((a, total));
    }
    out
}
