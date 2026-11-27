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
use owd_core::alignment::loops;
use owd_core::alignment::radius_check::default_check_chord;
use owd_core::alignment::river_following::choice::{Choice, ChoiceParams};
use owd_core::alignment::river_following::cuts::CutParams;
use owd_core::alignment::river_following::elements::Chain;
use owd_core::alignment::river_following::mask::RiverMask;
use owd_core::alignment::river_following::score::{OneScorer, PlainScorer, StationScorer, Totals};
use owd_core::alignment::river_following::seed::{Seed, SeedAim, SeedParams};
use owd_core::alignment::river_following::shift::{ShiftParams, levels, offset_step};
use owd_core::alignment::river_following::{Outcome, Params, off_river, run};
use owd_core::cut_model::WIDEN_RMAX;
use owd_core::geom::Pt;
use owd_core::geom::linear::{self, ProjectIndex};
use owd_core::geom::overlay::PolygonIndex;
use owd_core::optimise::window::scorer::{
    HalfWidthRule, LevelAt, LineMeasure, SectionSettings, Weights,
};
use owd_core::volumes::sections::SectionCutter;
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
    /// metres; unset: 5 m, or an eighth of the bottom width when that is more. At 5 m a smooth
    /// bend is one arc with straights between; at 2 m it becomes a compound curve of three arcs.
    pub seed_tolerance_m: Option<f64>,
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
            seed_tolerance_m: None,
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
                "seed_tolerance_m" => out.seed_tolerance_m = opt_pos(k, v)?,
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
    /// The seed's tolerance to the centreline.
    pub seed_tolerance_m: f64,
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
        let tol = s.seed_tolerance_m.unwrap_or((bw / 8.0).max(5.0));
        let p = SeedParams::scaled(r_min, s.seed_clip_ratio, 0.0, tol);
        Derived {
            r_min_m: r_min,
            loa_m: loa,
            bottom_width_m: bw,
            seed_tolerance_m: tol,
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
            self.seed_tolerance_m,
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
    /// Each bend: its arc, its radius, its gap to the centreline, and whether it is a tight bend
    /// held at the seed's least radius.
    pub bends: Vec<BendRecord>,
}

/// One bend of the seed, for the app's list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BendRecord {
    /// Seed chainage of the arc, from and to.
    pub from: Chainage,
    pub to: Chainage,
    pub radius_m: f64,
    /// `left` or `right`.
    pub hand: String,
    /// The largest gap of the line from the centreline round the bend, m.
    pub gap_m: f64,
    /// Held at the seed's least radius because the river is tighter (or moved to hold the minimum
    /// straight).
    pub tight: bool,
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

/// The numbers of one way through a stretch (staying in the river, or a cut-through).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Numbers {
    pub length_m: f64,
    /// Dredge (cut below the water level), m³.
    pub wet_m3: f64,
    /// Dry cut, m³.
    pub dry_m3: f64,
    /// Wet and dry, m³.
    pub plain_m3: f64,
    /// The weighted cost, as the scorer weighs it.
    pub cost: f64,
    /// The channel's water surface outside the river polygon, m².
    pub land_m2: f64,
    /// The tallest cut face above the water, m.
    pub face_max_m: f64,
    /// Stations whose face is over `face_max_m` of the settings.
    pub stations_over_face: usize,
}

impl From<&Totals> for Numbers {
    fn from(t: &Totals) -> Self {
        Numbers {
            length_m: t.length_m,
            wet_m3: t.wet_m3,
            dry_m3: t.dry_m3,
            plain_m3: t.plain_m3(),
            cost: t.cost,
            land_m2: t.land_m2,
            face_max_m: t.face_max_m,
            stations_over_face: t.stations_over_face,
        }
    }
}

/// One radius a cut-through was tried at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CutOptionRecord {
    pub radius_m: f64,
    pub length_m: f64,
    pub saved_m: f64,
    /// The river stretch's length over the cut's.
    pub ratio: f64,
    pub numbers: Numbers,
    /// The cut's centre line, for the map.
    pub line: Vec<[f64; 2]>,
}

/// A cut-through candidate, accepted or refused, with both sets of numbers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CutThroughRecord {
    /// A stable id from the river chainage of its two ends, to 100 m: `cut-AAAAA-BBBBB` in
    /// hectometres. A force list names it.
    pub id: String,
    /// Seed chainage of its two ends.
    pub seed_from: Chainage,
    pub seed_to: Chainage,
    /// River chainage of its two ends.
    pub river_from: Chainage,
    pub river_to: Chainage,
    /// Staying in the river over the stretch.
    pub stay: Numbers,
    /// The radii tried: the cut-through radius, then wider ones.
    pub options: Vec<CutOptionRecord>,
    /// The option offered (an index into `options`).
    pub offered: usize,
    /// True when the offered option passes the rule.
    pub passes: bool,
    pub accepted: bool,
    /// `rule`, `forced on` or `forced off`.
    pub decided_by: String,
    /// Why it was not taken, in words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refused_because: Option<String>,
    /// Design-line chainage of the cut, when accepted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<Chainage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<Chainage>,
}

/// The sideways shift, as the line records it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ShiftRecord {
    pub station_m: f64,
    pub offset_step_m: f64,
    pub reach_m: f64,
    pub max_slope: f64,
    /// The offset at each station, m (+ left).
    pub offsets: Vec<f64>,
    /// The search cost at zero shift, and after.
    pub cost_zero: f64,
    pub cost: f64,
    pub largest_offset_m: f64,
    pub mean_offset_m: f64,
    /// The share of stations moved by at least half an offset step.
    pub share_moved: f64,
    /// True when the end closed at zero offset and zero slope.
    pub closed: bool,
    /// The refinement's passes that moved the line.
    pub refined_passes: usize,
    /// The largest gap between the shifted line and the delivered arcs, m.
    pub tracking_gap_m: f64,
    /// True when the shifted line was built again (the shift moved it and saved cost).
    pub rebuilt: bool,
    /// True when it was built in exact 5 m steps, because its arcs missed it by more than the
    /// tolerance.
    pub tracked: bool,
    /// Stations where the shift was held at zero because the arcs could not follow it there.
    pub held_stations: usize,
    /// True when the shift was not delivered: its arcs could not follow it even held.
    pub dropped: bool,
}

/// The checks of section 7, each read from the line and the polygon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RiverChecks {
    /// The largest heading jump at an element join, rad.
    pub tangency_max_rad: f64,
    /// Opposite arcs either tighter than 2 R_min with a straight shorter than the minimum:
    /// `[chainage, straight length]`.
    pub short_reversals: Vec<[f64; 2]>,
    /// Stretches out of the river (with the margin) at least `off_river_min_m` long that no
    /// accepted cut-through or forced stretch holds: `[from, to]` chainage. Empty: the check
    /// passes. Null when the design has no river polygon.
    pub off_river_unexplained: Option<Vec<[f64; 2]>>,
    /// The line crosses itself.
    pub self_crossing: bool,
    /// The largest and the mean distance of the line from the river centreline, m.
    pub largest_from_centreline_m: f64,
    pub mean_from_centreline_m: f64,
    /// The share of the line's length whose centre is out of the river (with the margin).
    pub share_outside: Option<f64>,
    /// The tracking gap within the tolerance.
    pub tracking_gap_ok: bool,
    /// Forced ids that matched no candidate.
    pub force_unmatched: Vec<String>,
    /// Every check passes.
    pub ok: bool,
}

/// The data check: stretches where the river polygon's middle and the ground's low line are more
/// than half a bottom width apart (spec 8). A warning, never a move of the line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LowLineMismatch {
    /// River chainage, from and to.
    pub river_from: Chainage,
    pub river_to: Chainage,
    /// The largest distance between the two, m.
    pub largest_m: f64,
}

/// What the river-following line records about itself (spec OPT-4, section 6). The window optimiser
/// (OPT-5) reads [`RiverFollowingRecord::permitted_stretches`] from it for its permitted ground.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RiverFollowingRecord {
    /// The settings, every one as used.
    pub settings: Value,
    /// The lengths worked out from the sizing.
    pub derived: Derived,
    /// `volume` (a ground model) or `plan_area` (none).
    pub costed_on: String,
    pub seed: SeedRecord,
    /// Every cut-through candidate, in chainage order.
    pub cut_throughs: Vec<CutThroughRecord>,
    /// The forced stretches of the delivered line, in chainage order.
    pub forced: Vec<ForcedStretch>,
    /// The shift, when it ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shift: Option<ShiftRecord>,
    /// The delivered line's elements.
    pub elements: Vec<ElementRecord>,
    pub checks: RiverChecks,
    /// Where the river polygon and the ground's low line disagree (with a DEM).
    pub low_line_mismatch: Vec<LowLineMismatch>,
    /// Warnings in words.
    pub warnings: Vec<String>,
}

/// A stretch of the line, for the window optimiser's permitted ground (OPT-5 section 3.16).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PermittedStretch {
    /// The cut-through's or the forced stretch's id.
    pub id: String,
    /// `cut_through` (a forced seam at each end; room: `cut_through_room_m`) or `forced` (room:
    /// half the bottom width).
    pub kind: String,
    pub from: Chainage,
    pub to: Chainage,
    /// The room either side of the line the optimiser may use, m.
    pub room_m: f64,
}

impl RiverFollowingRecord {
    /// The stretches of the line out of the river the method chose (an accepted cut-through) or was
    /// forced into, each with the room the window optimiser may use about it, in chainage order.
    pub fn permitted_stretches(&self) -> Vec<PermittedStretch> {
        let mut out: Vec<PermittedStretch> = self
            .cut_throughs
            .iter()
            .filter(|c| c.accepted)
            .filter_map(|c| {
                Some(PermittedStretch {
                    id: c.id.clone(),
                    kind: "cut_through".into(),
                    from: c.from?,
                    to: c.to?,
                    room_m: self.derived.cut_through_room_m,
                })
            })
            .chain(self.forced.iter().map(|f| PermittedStretch {
                id: f.id.clone(),
                kind: "forced".into(),
                from: f.from,
                to: f.to,
                room_m: self.derived.bottom_width_m / 2.0,
            }))
            .collect();
        out.sort_by(|a, b| a.from.0.total_cmp(&b.from.0));
        out
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

/// The river-following line of `input`. The cut-throughs and the shift are priced on the ground
/// when `terrain` is given (else on plan area: the channel's water surface outside the river).
pub fn river_following(
    input: &AlignmentIn,
    terrain: Option<TerrainSampler>,
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
    let mut warnings = Vec::new();
    let polygon = input.river.polygon.as_ref().map(|p| p.to_polygon());
    let mask = polygon
        .as_ref()
        .map(|poly| RiverMask::new(std::slice::from_ref(poly), s.in_river_margin_m));
    let water = polygon
        .as_ref()
        .map(|poly| RiverMask::new(std::slice::from_ref(poly), 0.0));
    if mask.is_none() {
        warnings.push(
            "the design has no river polygon: the in-river tests are off, so no stretch out of \
             the river is found or recorded"
                .to_string(),
        );
    }
    let index = ProjectIndex::new(centre);
    let river_at = |q: Pt| index.project(q);
    // The water level at a point: the river's profile at the point's river chainage.
    let profile = input.river.water_profile.as_ref().filter(|p| !p.is_empty());
    let level = |q: Pt| profile.map_or(0.0, |p| p.level_at(Chainage(river_at(q))));
    let ground_fn = terrain.map(|t| move |q: Pt| t(&[q[0]], &[q[1]])[0]);
    if ground_fn.is_some() && profile.is_none() {
        return Err(refuse(
            "ALIGN_NO_WATER",
            "the river-following line costs its cut-throughs on the ground and needs a water \
             level: set one with `owd import --water-level LEVEL`, or run it without a DEM",
        ));
    }
    let ground_dyn: Option<&dyn Fn(Pt) -> f64> =
        ground_fn.as_ref().map(|g| g as &dyn Fn(Pt) -> f64);
    let shape = input.size.shape().ok();
    let widening = |r: f64| shape.map_or(0.0, |sh| sh.bend_widening_m(r.max(r_min)));
    // With a ground model: the one scorer, the volumes stage's own section (OPT-5 part 1). With
    // none: plan area, the channel's water surface outside the river.
    let vs = input.volumes.clone().unwrap_or_default();
    let river_index = PolygonIndex::new(polygon.as_slice());
    let owned = shape.cloned();
    let rule = HalfWidthRule::new(d.bottom_width_m / 2.0, WIDEN_RMAX, move |r: f64| {
        owned.as_ref().map_or(0.0, |sh| sh.bend_widening_m(r))
    });
    let level_sync = |q: Pt| level(q);
    let measure = match terrain {
        Some(t) => Some(LineMeasure::new(
            SectionCutter::new(
                &river_index,
                input.size.design_depth_m,
                &vs.cut_model,
                vs.wet_slope,
                vs.maxw_m,
                vs.dx_m,
                None,
            )
            .map_err(|e| refuse("ALIGN_SETTING", e))?,
            t,
            &rule,
            LevelAt::ByPoint(&level_sync),
            Weights::default(),
            &SectionSettings {
                model: vs.cut_model.clone(),
                depth_m: input.size.design_depth_m,
                wet_slope: vs.wet_slope,
                maxw_m: vs.maxw_m,
                dx_m: vs.dx_m,
                daylight_run_m: None,
            },
            vs.station_m,
        )),
        None => None,
    };
    let one = measure.as_ref().map(OneScorer::new);
    let plan = PlainScorer {
        ground: None,
        water: &level,
        depth_m: input.size.design_depth_m,
        bottom_m: d.bottom_width_m,
        side_slope: vs.wet_slope,
        widening: &widening,
        river: water.as_ref(),
        wet_weight: 0.0,
        dry_weight: 0.0,
        land_weight: 1.0,
    };
    let scorer: &dyn StationScorer = match &one {
        Some(o) => o,
        None => &plan,
    };
    let p = d.params(s);
    let out = run(
        centre,
        mask.as_ref(),
        &p,
        scorer,
        &widening,
        &s.cut_force,
        &river_at,
    );
    if !out.seed.settled {
        warnings.push(format!(
            "the seed is up to {:.1} m from the river centreline outside the bends tighter than \
             R_min (the tolerance is {:.1} m)",
            out.seed.gap_m, d.seed_tolerance_m
        ));
    }
    for id in &out.unmatched {
        warnings.push(format!(
            "the forced cut-through `{id}` matches no candidate now (the river or R_min changed): \
             it was not applied"
        ));
    }
    if ground_dyn.is_none() && !out.choices.is_empty() {
        warnings.push(
            "no DEM: cut-throughs are your choice. They are listed and costed on plan area, and \
             only those forced on are taken"
                .to_string(),
        );
    }
    if out.shift_dropped {
        warnings.push(
            "the sideways shift was not delivered: the arcs could not follow the shifted line \
             within twice the tolerance, so the line is the seed with its cut-throughs"
                .to_string(),
        );
    } else if out.shift_held > 0 {
        warnings.push(format!(
            "the sideways shift was held at zero at {} station(s) where the arcs could not follow it",
            out.shift_held
        ));
    }
    if out.tracking_gap_m > d.seed_tolerance_m && !out.shift_dropped {
        warnings.push(format!(
            "the delivered arcs are up to {:.1} m from the shifted line (the tolerance is {:.1} m)",
            out.tracking_gap_m, d.seed_tolerance_m
        ));
    }
    let chain = out.line.clone();
    let line = chain.points();
    // The cut-throughs, with their chainage on the delivered line.
    let delivered = ProjectIndex::new(&line);
    let on_line = |c: &Chain, a: f64| {
        let q = c.pose_at(a);
        delivered.project([q.0, q.1])
    };
    let cut_throughs: Vec<CutThroughRecord> = out
        .choices
        .iter()
        .map(|c| cut_record(c, &out.after_cuts, &on_line))
        .collect();
    let forced = forced_stretches(&out, &chain, centre, mask.as_ref(), &d, &cut_throughs);
    let low_line_mismatch = match (ground_dyn, polygon.as_ref()) {
        (Some(g), Some(_)) => low_line_mismatch(centre, water.as_ref(), g, &d),
        _ => Vec::new(),
    };
    if !low_line_mismatch.is_empty() {
        warnings.push(format!(
            "the river polygon's middle is more than half a bottom width from the ground's low \
             line along {} stretch(es) (the first from {:.0} m to {:.0} m of the river): the line \
             follows the polygon",
            low_line_mismatch.len(),
            low_line_mismatch[0].river_from.0,
            low_line_mismatch[0].river_to.0
        ));
    }
    let shift = out.shift.as_ref().map(|(_, sh)| {
        let sp = p.shift.as_ref().expect("the shift ran with its settings");
        let n = sh.offsets.len().max(1) as f64;
        ShiftRecord {
            station_m: sp.station_m,
            offset_step_m: sp.offset_step_m,
            reach_m: sp.reach_m,
            max_slope: sp.max_slope,
            offsets: sh.offsets.clone(),
            cost_zero: sh.cost_zero,
            cost: sh.cost,
            largest_offset_m: sh.offsets.iter().map(|x| x.abs()).fold(0.0, f64::max),
            mean_offset_m: sh.offsets.iter().map(|x| x.abs()).sum::<f64>() / n,
            share_moved: sh
                .offsets
                .iter()
                .filter(|x| x.abs() >= 0.5 * sp.offset_step_m)
                .count() as f64
                / n,
            closed: sh.closed,
            refined_passes: sh.refined,
            tracking_gap_m: out.tracking_gap_m,
            rebuilt: out.rebuild.is_some(),
            tracked: out.tracked,
            held_stations: out.shift_held,
            dropped: out.shift_dropped,
        }
    });
    let checks = river_checks(
        &chain,
        &line,
        centre,
        mask.as_ref(),
        &d,
        &cut_throughs,
        &forced,
        out.tracking_gap_m <= d.seed_tolerance_m,
        &out.unmatched,
    );
    let costed_on = if ground_dyn.is_some() {
        "volume"
    } else {
        "plan_area"
    };
    let record = RiverFollowingRecord {
        settings: s.full(),
        derived: d.clone(),
        costed_on: costed_on.into(),
        seed: seed_record(&out.seed),
        cut_throughs,
        forced,
        shift,
        elements: element_records(&chain),
        checks: checks.clone(),
        low_line_mismatch,
        warnings: warnings.clone(),
    };
    let mut all = radius_checks(&line, r_min, Some(d.check_chord_m))?;
    all["river_following_ok"] = json!(checks.ok);
    let m = super::alignment::AlignMethod::RiverFollowing;
    let mut line_out = finish(
        m.option_id(),
        m.option_name(),
        m.name(),
        s.full(),
        line,
        all,
        json!({
            "element_count": chain.elements.len(),
            "arc_count": chain.elements.iter().filter(|e| e.is_arc()).count(),
            "seed_gap_m": out.seed.gap_m,
            "cut_through_candidates": record.cut_throughs.len(),
            "cut_throughs_accepted": record.cut_throughs.iter().filter(|c| c.accepted).count(),
            // The stretches the window optimiser (OPT-5) reads from its starting line's details:
            // design-line chainage, from the typed record's permitted_stretches().
            "cut_throughs": permitted_json(&record, "cut_through"),
            "forced_stretches": permitted_json(&record, "forced"),
            "costed_on": costed_on,
            "warnings": warnings,
        }),
        &input.river,
    );
    line_out.river_following = Some(record);
    Ok(line_out)
}

impl Derived {
    /// The core's settings.
    fn params(&self, s: &RiverFollowingSettings) -> Params {
        let seed = self.seed_params(s);
        let shift = s.shift.then(|| {
            let mut sp = ShiftParams::scaled(
                self.r_min_m,
                self.r_shift_m,
                self.cut_through_room_m,
                self.off_river_min_m,
            );
            if let Some(h) = s.shift_station_m {
                sp.station_m = h;
                sp.offset_step_m = offset_step(h, self.r_min_m);
            }
            if let Some(x) = s.shift_offset_step_m {
                sp.offset_step_m = x;
            }
            if let Some(x) = s.shift_reach_m {
                sp.reach_m = x;
            }
            sp.max_slope = s.shift_max_slope;
            sp.same_curves = s.same_curves;
            sp.refine = s.shift_refine;
            sp.w_turn = s.turn_weight;
            sp.levels = levels(self.r_min_m, s.widening_levels as usize);
            sp
        });
        Params {
            cut: CutParams::scaled(
                self.r_min_m,
                self.r_cut_m,
                self.cut_min_save_m,
                s.cut_min_ratio,
                s.cut_max_arc_turn_deg,
                self.min_straight_m,
            ),
            choice: ChoiceParams {
                min_save_m: self.cut_min_save_m,
                min_gain: s.cut_min_gain,
                face_max_m: s.face_max_m,
                step_m: (self.r_min_m / 12.0).clamp(10.0, 25.0),
            },
            shift,
            off_river_min_m: self.off_river_min_m,
            forced_room_m: self.bottom_width_m / 2.0,
            seed,
        }
    }
}

/// The permitted stretches of one kind as the window optimiser reads them: `{id, from_m, to_m,
/// room_m}`.
fn permitted_json(record: &RiverFollowingRecord, kind: &str) -> Value {
    Value::Array(
        record
            .permitted_stretches()
            .into_iter()
            .filter(|p| p.kind == kind)
            .map(|p| json!({"id": p.id, "from_m": p.from.0, "to_m": p.to.0, "room_m": p.room_m}))
            .collect(),
    )
}

fn cut_record(c: &Choice, after: &Chain, on_line: &dyn Fn(&Chain, f64) -> f64) -> CutThroughRecord {
    let k = &c.candidate;
    CutThroughRecord {
        id: k.id.clone(),
        seed_from: Chainage(k.s_from),
        seed_to: Chainage(k.s_to),
        river_from: Chainage(k.river_from),
        river_to: Chainage(k.river_to),
        stay: Numbers::from(&c.stay),
        options: k
            .options
            .iter()
            .zip(&c.options)
            .enumerate()
            .map(|(i, (o, t))| CutOptionRecord {
                radius_m: o.radius_m,
                length_m: o.length_m,
                saved_m: k.saved_m(i),
                ratio: k.ratio(i),
                numbers: Numbers::from(t),
                line: o.line.clone(),
            })
            .collect(),
        offered: c.best,
        passes: c.passes,
        accepted: c.accepted,
        decided_by: c.decided_by.as_str().into(),
        refused_because: c.refused_because.clone(),
        from: c.line_from.map(|a| Chainage(on_line(after, a))),
        to: c.line_to.map(|b| Chainage(on_line(after, b))),
    }
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
        bends: sd
            .circles
            .iter()
            .zip(&sd.arcs)
            .zip(&sd.circle_gaps)
            .map(|((c, a), g)| BendRecord {
                from: Chainage(a.0),
                to: Chainage(a.1),
                radius_m: c.r,
                hand: if c.hand > 0 { "left" } else { "right" }.into(),
                gap_m: *g,
                tight: c.tight || c.moved,
            })
            .collect(),
    }
}

/// The farthest the chain is from the centreline between two chainages.
fn farthest(chain: &Chain, centre: &[Pt], from: f64, to: f64) -> f64 {
    let n = (((to - from) / 5.0).ceil() as usize).max(1);
    (0..=n)
        .map(|i| {
            let q = chain.pose_at(from + (to - from) * i as f64 / n as f64);
            linear::point_line_distance([q.0, q.1], centre)
        })
        .fold(0.0, f64::max)
}

/// The forced stretches of the delivered line: the seed's tight bends (where the delivered line
/// passes them), then the stretches out of the river (with the margin) at least `off_river_min_m`
/// long that no accepted cut-through or tight bend holds.
fn forced_stretches(
    out: &Outcome,
    chain: &Chain,
    centre: &[Pt],
    mask: Option<&RiverMask>,
    d: &Derived,
    cuts: &[CutThroughRecord],
) -> Vec<ForcedStretch> {
    let line = chain.points();
    let idx = ProjectIndex::new(&line);
    let on_line = |c: &Chain, a: f64| {
        let q = c.pose_at(a);
        idx.project([q.0, q.1])
    };
    let river_at = |s: f64| {
        let q = chain.pose_at(s);
        linear::project(centre, [q.0, q.1])
    };
    let inside_cut = |a: f64, b: f64| {
        cuts.iter().any(|c| match (c.from, c.to) {
            (Some(f), Some(t)) => a < t.0 && b > f.0,
            _ => false,
        })
    };
    let mut res: Vec<ForcedStretch> = out
        .seed
        .tight
        .iter()
        .filter_map(|t| {
            let (a, b) = (
                on_line(&out.seed.chain, t.from),
                on_line(&out.seed.chain, t.to),
            );
            let (a, b) = (a.min(b), a.max(b));
            // A tight bend a cut-through took away is no longer on the line.
            if inside_cut(a, b) || b - a < 1.0 {
                return None;
            }
            let (ra, rb) = (river_at(a), river_at(b));
            Some(ForcedStretch {
                id: format!("forced-{}", hm(0.5 * (ra + rb))),
                kind: if t.reason == "tight" {
                    "tight_bend"
                } else {
                    "straight"
                }
                .into(),
                from: Chainage(a),
                to: Chainage(b),
                river_from: Chainage(ra.min(rb)),
                river_to: Chainage(ra.max(rb)),
                river_radius_m: Some(t.river_radius_m),
                farthest_from_centreline_m: farthest(chain, centre, a, b),
            })
        })
        .collect();
    if let Some(m) = mask {
        for (a, b) in off_river(chain, m, d.off_river_min_m) {
            if inside_cut(a, b) {
                continue;
            }
            if let Some(t) = res.iter_mut().find(|t| a <= t.to.0 && b >= t.from.0) {
                t.from = Chainage(t.from.0.min(a));
                t.to = Chainage(t.to.0.max(b));
                let (ra, rb) = (river_at(t.from.0), river_at(t.to.0));
                t.river_from = Chainage(ra.min(rb));
                t.river_to = Chainage(ra.max(rb));
                t.farthest_from_centreline_m = farthest(chain, centre, t.from.0, t.to.0);
                continue;
            }
            let (ra, rb) = (river_at(a), river_at(b));
            res.push(ForcedStretch {
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
    res.sort_by(|x, y| x.from.0.total_cmp(&y.from.0));
    res
}

/// The checks of spec section 7, read from the delivered line and the polygon.
#[allow(clippy::too_many_arguments)]
pub fn river_checks(
    chain: &Chain,
    line: &[Pt],
    centre: &[Pt],
    mask: Option<&RiverMask>,
    d: &Derived,
    cuts: &[CutThroughRecord],
    forced: &[ForcedStretch],
    tracking_gap_ok: bool,
    unmatched: &[String],
) -> RiverChecks {
    let (tangency, _) = chain.largest_join_jump();
    let short: Vec<[f64; 2]> = chain
        .short_reversals(d.tight_reverse_r_m, d.min_straight_m)
        .into_iter()
        .map(|(a, l)| [a, l])
        .collect();
    let unexplained = mask.map(|m| off_river_unexplained(line, m, d, cuts, forced));
    let index = ProjectIndex::new(centre);
    let dist: Vec<f64> = line
        .iter()
        .map(|q| {
            let c = index.project(*q);
            let p = linear::interpolate(centre, c).unwrap_or(*q);
            libm::hypot(q[0] - p[0], q[1] - p[1])
        })
        .collect();
    let n = dist.len().max(1) as f64;
    let share_outside = mask.map(|m| {
        let out = line.iter().filter(|q| !m.contains(**q)).count();
        out as f64 / n
    });
    let self_crossing = loops::is_self_intersecting(line);
    let ok = tangency <= 1e-6
        && short.is_empty()
        && unexplained.as_ref().is_none_or(Vec::is_empty)
        && !self_crossing
        && tracking_gap_ok
        && unmatched.is_empty();
    RiverChecks {
        tangency_max_rad: tangency,
        short_reversals: short,
        off_river_unexplained: unexplained,
        self_crossing,
        largest_from_centreline_m: dist.iter().copied().fold(0.0, f64::max),
        mean_from_centreline_m: dist.iter().sum::<f64>() / n,
        share_outside,
        tracking_gap_ok,
        force_unmatched: unmatched.to_vec(),
        ok,
    }
}

/// The stretches of `line` (its own points, read every 5 m) out of the river with the margin for
/// at least `off_river_min_m` that no accepted cut-through and no forced stretch holds.
pub fn off_river_unexplained(
    line: &[Pt],
    mask: &RiverMask,
    d: &Derived,
    cuts: &[CutThroughRecord],
    forced: &[ForcedStretch],
) -> Vec<[f64; 2]> {
    let total = linear::length(line);
    let n = ((total / 5.0).ceil() as usize).max(1);
    let held = |a: f64, b: f64| {
        cuts.iter()
            .filter(|c| c.accepted)
            .filter_map(|c| Some((c.from?.0, c.to?.0)))
            .chain(forced.iter().map(|f| (f.from.0, f.to.0)))
            .any(|(f, t)| a >= f - 5.0 && b <= t + 5.0)
    };
    let mut out = Vec::new();
    let mut start: Option<f64> = None;
    let mut last = 0.0;
    for i in 0..=n {
        let s = total * i as f64 / n as f64;
        let q = linear::interpolate(line, s).unwrap_or([f64::NAN; 2]);
        let outside = !mask.contains(q);
        match (outside, start) {
            (true, None) => start = Some(s),
            (false, Some(a)) => {
                if last - a >= d.off_river_min_m && !held(a, last) {
                    out.push([a, last]);
                }
                start = None;
            }
            _ => {}
        }
        last = s;
    }
    if let Some(a) = start
        && total - a >= d.off_river_min_m
        && !held(a, total)
    {
        out.push([a, total]);
    }
    out
}

/// The data check (spec 8): along the river every R_min / 3, the ground's lowest point across the
/// river (out to the bank and a bottom width beyond) against the centreline; stretches where they
/// are more than half a bottom width apart.
fn low_line_mismatch(
    centre: &[Pt],
    water: Option<&RiverMask>,
    ground: &dyn Fn(Pt) -> f64,
    d: &Derived,
) -> Vec<LowLineMismatch> {
    let total = linear::length(centre);
    let step = (d.r_min_m / 3.0).max(10.0);
    let n = ((total / step).ceil() as usize).max(1);
    let half = d.bottom_width_m / 2.0;
    let mut flagged: Vec<(f64, f64)> = Vec::new();
    for i in 0..=n {
        let s = total * i as f64 / n as f64;
        let (a, b) = ((s - 1.0).max(0.0), (s + 1.0).min(total));
        let pa = linear::interpolate(centre, a).unwrap_or([0.0; 2]);
        let pb = linear::interpolate(centre, b).unwrap_or([0.0; 2]);
        let p = linear::interpolate(centre, s).unwrap_or([0.0; 2]);
        let th = libm::atan2(pb[1] - pa[1], pb[0] - pa[0]);
        let nrm = [-libm::sin(th), libm::cos(th)];
        // Out to where the river ends on each side, and a bottom width beyond.
        let reach = |sgn: f64| {
            let mut o = 0.0;
            while o < 2000.0 {
                let q = [p[0] + sgn * o * nrm[0], p[1] + sgn * o * nrm[1]];
                if water.is_some_and(|w| !w.contains(q)) {
                    break;
                }
                o += 2.0;
            }
            o + d.bottom_width_m
        };
        let (lo, hi) = (reach(-1.0), reach(1.0));
        // The low line: the middle of the samples at the lowest level (a flat bed has no one
        // lowest point).
        let mut samples = Vec::new();
        let mut o = -lo;
        while o <= hi {
            let z = ground([p[0] + o * nrm[0], p[1] + o * nrm[1]]);
            if z.is_finite() {
                samples.push((z, o));
            }
            o += 2.0;
        }
        let zmin = samples.iter().map(|x| x.0).fold(f64::INFINITY, f64::min);
        let low: Vec<f64> = samples
            .iter()
            .filter(|x| x.0 <= zmin + 0.05)
            .map(|x| x.1)
            .collect();
        let off = if low.is_empty() {
            0.0
        } else {
            0.5 * (low[0] + low[low.len() - 1])
        };
        flagged.push((s, off.abs()));
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < flagged.len() {
        if flagged[i].1 <= half {
            i += 1;
            continue;
        }
        let a = i;
        let mut big: f64 = 0.0;
        while i < flagged.len() && flagged[i].1 > half {
            big = big.max(flagged[i].1);
            i += 1;
        }
        out.push(LowLineMismatch {
            river_from: Chainage(flagged[a].0),
            river_to: Chainage(flagged[i - 1].0),
            largest_m: big,
        });
    }
    out
}
