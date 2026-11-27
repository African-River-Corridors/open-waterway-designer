//! The water stage (spec 0017, ADR 0015, ticket 8): the water level along the design line.
//!
//! [`water`] is a pure function from a [`WaterIn`] to a [`DesignWater`]. It reads no document and
//! no file. A station of the design line finds its level as the stages always have (ADR 0006): it is
//! projected onto the river centreline and the level is read there, so two lines through one place
//! see one water. Chainage is from the river mouth when the river states one (the design line and
//! the centreline are both stored from it); without a mouth, from the start of each line as given.
//!
//! Where the level comes from:
//!
//! - **Pools** (from the optimise stage today, the weirs-and-locks stage later) set the level inside
//!   each pool. A pool carries its design level and, when the stage that made it gives them, a
//!   minimum and a maximum level (issue 532: navigation depth is checked at the minimum, flood
//!   risk at the maximum). The water stage carries them; it does not choose them.
//! - **Given**: the water profile of the inputs (a profile, a pool table, a constant or a fall),
//!   when there are no pools.
//! - **Open river**: outside every pool, when [`BeyondPools::OpenRiver`] says the river's own level
//!   holds there (below the lowest weir).
//!
//! Beyond the ends of the profile that sets it, a level is held flat, as a water profile always is;
//! those points say so (`held_flat`).
//!
//! The answer is points on design-line chainage: one at each station of the volumes walk (`step_m`
//! apart, or a little more, so that they reach the end evenly), and two at each place where
//! the level steps or its source changes (a weir), the lower chainage's level first. Read between
//! points, it is linear; at a step, the upstream level holds, as in a [`WaterProfile`].

use crate::types::{Chainage, Polyline, StageError, WaterProfile};
use owd_core::geom::linear;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The stage's name, as the document stores it.
pub const STAGE: &str = "water";

/// The sample step the stage uses when none is set, in metres of design-line chainage: the volumes
/// stage's station step, so a station of the volumes walk is a point of the answer.
pub const DEFAULT_STEP_M: f64 = owd_core::volumes::sections::DEFAULT_STATION_M;

fn default_step() -> f64 {
    DEFAULT_STEP_M
}

/// One pool: a stretch of river held at one level by the structure at its downstream end.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Pool {
    /// River chainage where the pool starts (its downstream end).
    pub from: Chainage,
    /// River chainage where the pool ends (its upstream end). Not below `from`.
    pub to: Chainage,
    /// The pool's design level, metres above datum.
    pub level_m: f64,
    /// The lowest level the pool is held at: navigation depth is checked here (issue 532).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_level_m: Option<f64>,
    /// The highest level the pool reaches: flood risk is checked here (issue 532).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_level_m: Option<f64>,
}

/// What the level is outside every pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BeyondPools {
    /// The pools' levels, read as one stepped profile and held flat beyond its ends. How the
    /// optimise stage's pools have always been read; the default, so no stored number moves.
    #[default]
    HoldPoolLevel,
    /// The river's own level, from the given profile (open river below the lowest weir).
    OpenRiver,
}

/// What the water stage reads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WaterIn {
    /// The river centreline the levels are read on, from the mouth when one is stated.
    pub centreline: Polyline,
    /// The design line the stages after this one read.
    pub design_line: Polyline,
    /// The water profile of the inputs, on river chainage.
    #[serde(default)]
    pub given: Option<WaterProfile>,
    /// The pools, in order of river chainage. Empty: no pools.
    #[serde(default)]
    pub pools: Vec<Pool>,
    /// What the level is outside every pool.
    #[serde(default)]
    pub beyond_pools: BeyondPools,
    /// The step between points of the answer, metres of design-line chainage.
    #[serde(default = "default_step")]
    pub step_m: f64,
}

/// Where a level comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WaterSource {
    /// The water profile of the inputs.
    Given,
    /// A pool held by a structure.
    Pool,
    /// The river's own level, outside every pool.
    OpenRiver,
    /// Between two pools that do not touch: read straight from the lower pool's level to the upper
    /// one's (how the optimise stage's pools have always been read).
    BetweenPools,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// One point of the answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DesignWaterPoint {
    /// Design-line chainage.
    pub chainage: Chainage,
    /// The river chainage the point projects to, where its level was read.
    pub river_chainage: Chainage,
    /// The design water level, metres above datum.
    pub level_m: f64,
    /// Where the level comes from.
    pub source: WaterSource,
    /// The pool's minimum level, inside a pool that has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_level_m: Option<f64>,
    /// The pool's maximum level, inside a pool that has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_level_m: Option<f64>,
    /// True beyond the ends of the profile that sets the level: the nearer end's level, held flat.
    #[serde(default, skip_serializing_if = "is_false")]
    pub held_flat: bool,
}

/// A stretch of design line whose level comes from one source (and one pool).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WaterStretch {
    /// Design-line chainage where it starts.
    pub from: Chainage,
    /// Design-line chainage where it ends.
    pub to: Chainage,
    /// Where its level comes from.
    pub source: WaterSource,
    /// The pool it lies in: its index in the input's pools.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pool: Option<usize>,
    /// That pool's design level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pool_level_m: Option<f64>,
    /// That pool's minimum level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_level_m: Option<f64>,
    /// That pool's maximum level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_level_m: Option<f64>,
    /// True beyond the ends of the profile that sets the level.
    #[serde(default, skip_serializing_if = "is_false")]
    pub held_flat: bool,
}

/// What the water stage returns: the water level along the design line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DesignWater {
    /// The design line's length, metres.
    pub length_m: f64,
    /// The points, in order of design-line chainage; two at a step.
    pub points: Vec<DesignWaterPoint>,
    /// The stretches, in order, with no gap between them.
    pub stretches: Vec<WaterStretch>,
}

impl DesignWater {
    /// The level at a design-line chainage: linear between points, held flat beyond the ends, the
    /// later point at a step. NaN for an empty answer.
    pub fn level_at(&self, chainage: Chainage) -> f64 {
        let pairs: Vec<[f64; 2]> = self
            .points
            .iter()
            .map(|p| [p.chainage.0, p.level_m])
            .collect();
        WaterProfile::from_pairs(&pairs).level_at(chainage)
    }
}

fn refuse(code: &str, message: &str) -> StageError {
    StageError::new(STAGE, code, message)
}

fn finite(line: &[[f64; 2]]) -> bool {
    line.iter().all(|p| p[0].is_finite() && p[1].is_finite())
}

/// What the level at one place is and where it comes from.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Reading {
    river: f64,
    level: f64,
    source: WaterSource,
    pool: Option<usize>,
    held: bool,
    /// The segment of the profile it was read on: a change of segment between two samples is a
    /// break or a step the answer must show.
    segment: isize,
}

impl Reading {
    /// The same stretch: source, pool and held alike.
    fn same_stretch(&self, o: &Reading) -> bool {
        self.source == o.source && self.pool == o.pool && self.held == o.held
    }
}

/// Where `c` lies on `profile`: -1 before its first point, `n` after its last (both held flat),
/// else the index of the segment it is read on (the last at the last point; the later one at a
/// step, as `WaterProfile::level_at` reads it).
fn segment_of(profile: &WaterProfile, c: f64) -> isize {
    let p = profile.points();
    let n = p.len();
    if c < p[0].chainage.0 {
        return -1;
    }
    if c > p[n - 1].chainage.0 {
        return n as isize;
    }
    let i = p.partition_point(|q| q.chainage.0 <= c);
    (i.saturating_sub(1)).min(n.saturating_sub(2)) as isize
}

/// The input, checked, with the profiles it reads built once.
struct Prepared<'a> {
    input: &'a WaterIn,
    pool_profile: Option<WaterProfile>,
    length_m: f64,
}

impl Prepared<'_> {
    /// The pool whose reach holds `c`: the last one, so at a weir the upstream pool holds.
    fn pool_at(&self, c: f64) -> Option<usize> {
        self.input
            .pools
            .iter()
            .rposition(|p| p.from.0 <= c && c <= p.to.0)
    }

    fn read_given(&self, c: f64, source: WaterSource) -> Reading {
        let g = self.input.given.as_ref().expect("checked: a given profile");
        let segment = segment_of(g, c);
        Reading {
            river: c,
            level: g.level_at(Chainage(c)),
            source,
            pool: None,
            held: segment < 0 || segment >= g.len() as isize,
            segment,
        }
    }

    /// The reading at river chainage `c`.
    fn read(&self, c: f64) -> Reading {
        let Some(pp) = &self.pool_profile else {
            return self.read_given(c, WaterSource::Given);
        };
        match self.input.beyond_pools {
            BeyondPools::OpenRiver => match self.pool_at(c) {
                Some(k) => Reading {
                    river: c,
                    level: self.input.pools[k].level_m,
                    source: WaterSource::Pool,
                    pool: Some(k),
                    held: false,
                    segment: 2 * k as isize,
                },
                None => {
                    let mut r = self.read_given(c, WaterSource::OpenRiver);
                    // Open-river segments are counted apart from the pools'.
                    r.segment = -1000 - r.segment;
                    r
                }
            },
            BeyondPools::HoldPoolLevel => {
                let held_at = segment_of(pp, c);
                let held = held_at < 0 || held_at >= pp.len() as isize;
                let pool = if held { None } else { self.pool_at(c) };
                // Pool k is segment 2k and the gap above it 2k + 1, so a change of segment is a
                // change of pool or of gap, never one inside a pool.
                let segment = match (held, pool) {
                    (true, _) => held_at,
                    (false, Some(k)) => 2 * k as isize,
                    (false, None) => {
                        let below = self.input.pools.iter().rposition(|q| q.to.0 < c);
                        2 * below.map_or(0, |k| k as isize) + 1
                    }
                };
                Reading {
                    river: c,
                    level: pp.level_at(Chainage(c)),
                    // Inside the pools' span but in no pool: read straight from one pool's level
                    // to the next one's.
                    source: if held || pool.is_some() {
                        WaterSource::Pool
                    } else {
                        WaterSource::BetweenPools
                    },
                    pool,
                    held,
                    segment,
                }
            }
        }
    }

    /// The river chainage of design-line chainage `s`.
    fn river_chainage(&self, s: f64) -> f64 {
        let at = linear::interpolate(&self.input.design_line.0, s).unwrap_or([f64::NAN; 2]);
        linear::project(&self.input.centreline.0, at)
    }

    fn reading_at(&self, s: f64) -> Reading {
        self.read(self.river_chainage(s))
    }

    fn point(&self, s: f64, r: &Reading) -> DesignWaterPoint {
        let pool = r.pool.map(|k| &self.input.pools[k]);
        DesignWaterPoint {
            chainage: Chainage(s),
            river_chainage: Chainage(r.river),
            level_m: r.level,
            source: r.source,
            min_level_m: pool.and_then(|p| p.min_level_m),
            max_level_m: pool.and_then(|p| p.max_level_m),
            held_flat: r.held,
        }
    }
}

fn check(input: &WaterIn) -> Result<(), StageError> {
    if input.centreline.len() < 2 || !finite(&input.centreline.0) {
        return Err(refuse(
            "WATER_NO_RIVER",
            "the water level is read on the river centreline: it needs two finite points",
        ));
    }
    if input.design_line.len() < 2 || !finite(&input.design_line.0) {
        return Err(refuse(
            "WATER_NO_DESIGN_LINE",
            "the water level is read along the design line: it needs two finite points",
        ));
    }
    if !(input.step_m.is_finite() && input.step_m > 0.0) {
        return Err(refuse(
            "WATER_SETTING",
            &format!(
                "the step between points must be a positive number of metres, not {}",
                input.step_m
            ),
        ));
    }
    if let Some(g) = &input.given {
        let p = g.points();
        if p.iter()
            .any(|q| !(q.chainage.0.is_finite() && q.level_m.is_finite()))
        {
            return Err(refuse(
                "WATER_PROFILE_INVALID",
                "every point of the water level profile must be a finite chainage and level",
            ));
        }
        if let Some(w) = p.windows(2).find(|w| w[1].chainage.0 < w[0].chainage.0) {
            return Err(refuse(
                "WATER_PROFILE_INVALID",
                &format!(
                    "the chainages of the water level profile must not decrease: {} m follows {} m",
                    w[1].chainage.0, w[0].chainage.0
                ),
            )
            .at(w[1].chainage));
        }
    }
    for p in &input.pools {
        if !(p.from.0.is_finite() && p.to.0.is_finite() && p.level_m.is_finite())
            || p.to.0 < p.from.0
        {
            return Err(refuse(
                "WATER_POOL_INVALID",
                &format!(
                    "a pool needs finite chainages and level, and cannot end ({} m) before it \
                     starts ({} m)",
                    p.to.0, p.from.0
                ),
            )
            .at(p.from));
        }
        let lo = p.min_level_m.unwrap_or(p.level_m);
        let hi = p.max_level_m.unwrap_or(p.level_m);
        if !(lo.is_finite() && hi.is_finite() && lo <= p.level_m && p.level_m <= hi) {
            return Err(refuse(
                "WATER_POOL_LEVELS_INVALID",
                &format!(
                    "the pool from {} m holds {} m: its design level must lie between its \
                     minimum ({lo} m) and its maximum ({hi} m) level",
                    p.from.0, p.level_m
                ),
            )
            .at(p.from));
        }
    }
    if let Some(w) = input.pools.windows(2).find(|w| w[1].from.0 < w[0].to.0) {
        return Err(refuse(
            "WATER_POOLS_OVERLAP",
            &format!(
                "the pool from {} m starts before the one below it ends ({} m)",
                w[1].from.0, w[0].to.0
            ),
        )
        .at(w[1].from));
    }
    let given = input.given.as_ref().is_some_and(|g| !g.is_empty());
    if input.pools.is_empty() && !given {
        return Err(refuse(
            "WATER_NO_LEVEL",
            "there is no water level: give a water level profile, a pool table, a constant or a \
             fall, or pools",
        ));
    }
    if !input.pools.is_empty() && input.beyond_pools == BeyondPools::OpenRiver && !given {
        return Err(refuse(
            "WATER_NO_OPEN_RIVER_LEVEL",
            "outside the pools the river is open, and its level is read from the given water \
             level profile: there is none",
        ));
    }
    Ok(())
}

/// The design-line chainage in `(lo, hi]` where the reading first differs from the one at `lo`:
/// bisection to the last representable step.
fn boundary(p: &Prepared, mut lo: f64, mut hi: f64, at_lo: &Reading) -> f64 {
    let key = |r: &Reading| (r.segment, r.source, r.pool, r.held);
    let k0 = key(at_lo);
    for _ in 0..200 {
        let mid = lo + (hi - lo) / 2.0;
        if mid <= lo || mid >= hi {
            break;
        }
        if key(&p.reading_at(mid)) == k0 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    hi
}

fn prepare(input: &WaterIn) -> Prepared<'_> {
    let pool_profile = (!input.pools.is_empty()).then(|| {
        let pairs: Vec<[f64; 2]> = input
            .pools
            .iter()
            .flat_map(|p| [[p.from.0, p.level_m], [p.to.0, p.level_m]])
            .collect();
        WaterProfile::from_pairs(&pairs)
    });
    Prepared {
        input,
        pool_profile,
        length_m: input.design_line.length_m(),
    }
}

/// The water stage: the water level along the design line, with where each level comes from.
pub fn water(input: &WaterIn) -> Result<DesignWater, StageError> {
    check(input)?;
    let prepared = prepare(input);
    let p = &prepared;
    // The volumes walk's stations (`station_frame` in the core): `floor(length / step) + 1`
    // chainages evenly from 0 to the length, so each station is a point of the answer.
    let n = (p.length_m / input.step_m).floor() as usize + 1;
    let samples: Vec<f64> = if n < 2 {
        vec![0.0, p.length_m]
    } else {
        let every = p.length_m / (n - 1) as f64;
        let mut v: Vec<f64> = (0..n).map(|i| i as f64 * every).collect();
        v[n - 1] = p.length_m;
        v
    };

    let mut points: Vec<DesignWaterPoint> = Vec::new();
    let mut readings: Vec<Reading> = Vec::new();
    let push = |points: &mut Vec<DesignWaterPoint>, readings: &mut Vec<Reading>, s, r: Reading| {
        points.push(p.point(s, &r));
        readings.push(r);
    };
    let mut prev: Option<(f64, Reading)> = None;
    for &s in &samples {
        if prev.is_some_and(|(ps, _)| s <= ps) {
            continue;
        }
        let r = p.reading_at(s);
        if let Some((ps, pr)) = prev {
            let changed =
                (pr.segment, pr.source, pr.pool, pr.held) != (r.segment, r.source, r.pool, r.held);
            if changed {
                // A break, a step or a change of source between two samples: show it where it is.
                let b = boundary(p, ps, s, &pr);
                let below = p.reading_at(b - (b - ps) * 1e-12);
                let above = p.reading_at(b);
                let mut lower = below;
                lower.river = above.river;
                let same = lower.same_stretch(&above) && lower.level == above.level;
                if !same {
                    // The lower side's level, at the place of the change.
                    push(&mut points, &mut readings, b, lower);
                }
                if b < s {
                    push(&mut points, &mut readings, b, above);
                }
            }
        }
        push(&mut points, &mut readings, s, r);
        prev = Some((s, r));
    }

    // The stretches: runs of one source, pool and holding.
    let mut stretches: Vec<WaterStretch> = Vec::new();
    for (pt, r) in points.iter().zip(&readings) {
        match stretches.last_mut() {
            Some(last)
                if last.source == r.source && last.pool == r.pool && last.held_flat == r.held =>
            {
                last.to = pt.chainage;
            }
            _ => {
                if let Some(last) = stretches.last_mut() {
                    last.to = pt.chainage;
                }
                let pool = r.pool.map(|k| &input.pools[k]);
                stretches.push(WaterStretch {
                    from: pt.chainage,
                    to: pt.chainage,
                    source: r.source,
                    pool: r.pool,
                    pool_level_m: pool.map(|q| q.level_m),
                    min_level_m: pool.and_then(|q| q.min_level_m),
                    max_level_m: pool.and_then(|q| q.max_level_m),
                    held_flat: r.held,
                });
            }
        }
    }
    Ok(DesignWater {
        length_m: p.length_m,
        points,
        stretches,
    })
}

/// The level at design-line chainage `s` of `input`, read as the stage reads it (by projection onto
/// the river centreline). For the stages that walk the line at their own stations, so a station
/// and the answer's point at the same chainage give the same level. NaN for an input the stage
/// refuses.
pub fn level_on_line(input: &WaterIn, s: f64) -> f64 {
    match level_reader(input) {
        Ok(read) => read(s),
        Err(_) => f64::NAN,
    }
}

/// The level along the design line of `input` as a function of design-line chainage, read as the
/// stage reads it; refused by code as the stage refuses. The volumes walk reads its levels through
/// it, so a station and the stored point at its chainage have one level (one path).
pub fn level_reader(input: &WaterIn) -> Result<impl Fn(f64) -> f64 + '_, StageError> {
    check(input)?;
    let p = prepare(input);
    Ok(move |s: f64| p.reading_at(s).level)
}
