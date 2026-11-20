//! The weirs-and-locks stage with given structures, and the structures seam (spec 0017, ticket 14).
//!
//! Weirs and locks are an optional module. The engineer **gives** each structure
//! (`inputs.structures`, mode `given`): its site (a river chainage from the mouth, or a point), the
//! mean water level it holds upstream, its height, its cost, and optionally a name and a kind (weir,
//! lock, or both). The search over candidate sites is ticket 15.
//!
//! [`structures`] is a pure function from a [`StructuresIn`] and a [`StructuresTool`] to a
//! [`StructurePlan`]. It places each structure on the river and on the design line, finds the level
//! below it (the next structure's pool, or the open river from the given water level profile for
//! the lowest), checks that the pool levels step down to the mouth, asks the structures tool for
//! each structure's height, footprint and cost (the **structures seam**), and returns the
//! structures and the pools between them. No hydraulics: a pool is flat at the level the engineer
//! gave, from its structure up to the next one (the top pool to the end of the river).
//!
//! **Minimum and maximum pool levels** (issue 532, the maintainer's rule): each pool's minimum level
//! is its level less an allowance, and its maximum level its level plus an allowance
//! ([`PoolLevelRule`], one pair for the run, written out in the input). Navigation depth is checked
//! at the minimum and flood risk at the maximum ([`pool_checks`]), on the stations the volumes stage
//! stores.
//!
//! **The seam.** OWD sends, for each structure, the site (point and chainages), the upstream and
//! downstream water level and the channel width ([`StructureRequest`]); a tool returns its height,
//! footprint and cost ([`StructureResponse`]). [`GivenTool`] is the built-in pass-through: it
//! returns what the engineer gave (and no footprint). A later Open Tool implements the same trait.

use super::session::{DesignError, Session};
use super::volumes_stage::{SectionProfile, Volumes};
use super::water_stage::{self, DesignWater, Pool, WaterStretch};
use crate::types::{Chainage, Money, Polyline, RiverPolygon, StageError, WaterProfile};
use owd_core::geom::{Pt, linear};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The stage's name, as the document stores it.
pub const STAGE: &str = "structures";

/// The input's key in `inputs`.
pub const INPUT: &str = "structures";

/// The name of the input's schema contract.
pub const CONTRACT: &str = "inputs.structures";

/// The allowance below a pool's level that gives its minimum level, metres, when the input states
/// none. An OWD placeholder for the maintainer to confirm, not a standard.
pub const DEFAULT_BELOW_M: f64 = 0.5;

/// The allowance above a pool's level that gives its maximum level, metres, when the input states
/// none. An OWD placeholder for the maintainer to confirm, not a standard.
pub const DEFAULT_ABOVE_M: f64 = 0.5;

fn default_step() -> f64 {
    water_stage::DEFAULT_STEP_M
}

fn refuse(code: &str, message: impl AsRef<str>) -> StageError {
    StageError::new(STAGE, code, message.as_ref())
}

/// What a structure is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StructureKind {
    /// A weir: it holds the pool; vessels do not pass it.
    #[default]
    Weir,
    /// A lock: vessels pass it.
    Lock,
    /// A weir with a lock beside it.
    WeirAndLock,
}

/// Where a structure is: a river chainage from the mouth, or a point in the project CRS.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Site {
    /// River chainage from the mouth, metres.
    Chainage(Chainage),
    /// A point `[x, y]` in the project CRS. It must lie within half the channel width of the design
    /// line; its river chainage is where it projects onto the river centreline.
    Point([f64; 2]),
}

/// One structure the engineer gives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GivenStructure {
    /// A name for reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Weir, lock, or both. A weir when not given.
    #[serde(default)]
    pub kind: StructureKind,
    /// Where it is.
    pub site: Site,
    /// The mean water level it holds upstream: its pool's design level, metres above datum.
    pub upstream_level_m: f64,
    /// The structure's height, metres.
    pub height_m: f64,
    /// What it costs.
    pub cost: Money,
}

/// How a pool's minimum and maximum levels follow from its level: an allowance below and above,
/// one pair for the run.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PoolLevelRule {
    /// The minimum level is the pool's level less this, metres. Navigation depth is checked there.
    pub below_m: f64,
    /// The maximum level is the pool's level plus this, metres. Flood risk is checked there.
    pub above_m: f64,
}

impl Default for PoolLevelRule {
    fn default() -> Self {
        Self {
            below_m: DEFAULT_BELOW_M,
            above_m: DEFAULT_ABOVE_M,
        }
    }
}

/// How the structures are found. Only `given` today; the search over candidate sites is ticket 15.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StructuresMode {
    /// The engineer gives each structure.
    #[default]
    Given,
}

/// The `inputs.structures` block: the structures and the rule for minimum and maximum pool levels.
/// Absent: no weirs or locks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Structures {
    /// How the structures are found.
    pub mode: StructuresMode,
    /// The given structures, in any order.
    pub items: Vec<GivenStructure>,
    /// The rule for minimum and maximum pool levels. Written out in full when stored, so a later
    /// change of default never changes what a stored design meant.
    #[serde(default)]
    pub pool_levels: PoolLevelRule,
}

impl Structures {
    /// Check what can be checked without the river: at least one structure; finite levels; a height
    /// above zero; a cost of zero or more, all in one currency; a finite site; allowances of zero
    /// or more.
    pub fn validate(&self) -> Result<(), StageError> {
        if self.items.is_empty() {
            return Err(refuse(
                "WEIRS_NO_STRUCTURES",
                "the structures are given but the list is empty: give at least one, or remove \
                 the structures",
            ));
        }
        let PoolLevelRule { below_m, above_m } = self.pool_levels;
        if !(below_m.is_finite() && below_m >= 0.0 && above_m.is_finite() && above_m >= 0.0) {
            return Err(refuse(
                "WEIRS_POOL_LEVELS_INVALID",
                format!(
                    "the allowances below ({below_m} m) and above ({above_m} m) a pool's level \
                     must be finite numbers, zero or more"
                ),
            ));
        }
        for (i, s) in self.items.iter().enumerate() {
            let who = name_of(i, s.name.as_deref());
            let site_ok = match s.site {
                Site::Chainage(c) => c.0.is_finite(),
                Site::Point(p) => p[0].is_finite() && p[1].is_finite(),
            };
            if !site_ok {
                return Err(refuse(
                    "WEIRS_ITEM_INVALID",
                    format!("{who}: its site must be finite"),
                ));
            }
            if !s.upstream_level_m.is_finite() {
                return Err(refuse(
                    "WEIRS_ITEM_INVALID",
                    format!("{who}: its upstream water level must be a finite number"),
                ));
            }
            if !(s.height_m.is_finite() && s.height_m > 0.0) {
                return Err(refuse(
                    "WEIRS_ITEM_INVALID",
                    format!(
                        "{who}: its height is {} m: it must be above zero",
                        s.height_m
                    ),
                ));
            }
            if s.cost.amount < 0.0 {
                return Err(refuse(
                    "WEIRS_ITEM_INVALID",
                    format!(
                        "{who}: its cost is {}: it must be zero or more",
                        s.cost.amount
                    ),
                ));
            }
        }
        let first = &self.items[0];
        if let Some((i, s)) = self
            .items
            .iter()
            .enumerate()
            .find(|(_, s)| s.cost.currency != first.cost.currency)
        {
            return Err(refuse(
                "WEIRS_MIXED_CURRENCY",
                format!(
                    "{} costs in {} but {} in {}: the structures are priced in one currency",
                    name_of(0, first.name.as_deref()),
                    first.cost.currency,
                    name_of(i, s.name.as_deref()),
                    s.cost.currency
                ),
            ));
        }
        Ok(())
    }
}

fn name_of(i: usize, name: Option<&str>) -> String {
    match name {
        Some(n) => format!("structure `{n}`"),
        None => format!("structure {}", i + 1),
    }
}

/// Read and check the structures input from JSON. A key it does not know, or a value of the wrong
/// type, is refused (`WEIRS_INVALID`).
pub fn read_structures(v: &Value) -> Result<Structures, StageError> {
    let s: Structures = serde_json::from_value(v.clone()).map_err(|e| {
        refuse(
            "WEIRS_INVALID",
            format!("not a structures input (`mode`, `items`, `pool_levels`): {e}"),
        )
    })?;
    s.validate()?;
    Ok(s)
}

/// What the structures stage reads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StructuresIn {
    /// The river centreline, from the mouth when one is stated.
    pub centreline: Polyline,
    /// The design line the later stages read.
    pub design_line: Polyline,
    /// The water level profile of the inputs, on river chainage: the open river below the lowest
    /// structure.
    #[serde(default)]
    pub given_water: Option<WaterProfile>,
    /// The channel width sent to the structures tool, and the tolerance of a site given as a point
    /// (half of it): the sizing's bottom width on a straight, metres.
    pub channel_width_m: f64,
    /// The structures input.
    pub structures: Structures,
    /// The step, metres of design-line chainage, at which the line is searched for each site
    /// before bisection.
    #[serde(default = "default_step")]
    pub step_m: f64,
}

/// What OWD sends the structures tool for one structure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StructureRequest {
    /// The structure's place in the input's list.
    pub id: usize,
    /// The site on the design line, project CRS.
    pub point: [f64; 2],
    /// River chainage from the mouth.
    pub river_chainage: Chainage,
    /// Design-line chainage.
    pub chainage: Chainage,
    /// The water level it holds upstream.
    pub upstream_level_m: f64,
    /// The water level below it.
    pub downstream_level_m: f64,
    /// The channel width there, metres.
    pub channel_width_m: f64,
}

/// What the structures tool returns for one structure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StructureResponse {
    /// The request's `id`.
    pub id: usize,
    /// The structure's height, metres.
    pub height_m: f64,
    /// Its footprint, when the tool gives one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footprint: Option<RiverPolygon>,
    /// What it costs.
    pub cost: Money,
}

/// Where a structure's height, footprint and cost come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StructureSource {
    /// The engineer gave them (the pass-through).
    Given,
    /// A structures tool worked them out.
    Seam,
}

/// The structures seam: a tool that gives each structure's height, footprint and cost.
pub trait StructuresTool {
    /// One response for each request, by `id`.
    fn price(&self, req: &[StructureRequest]) -> Result<Vec<StructureResponse>, StageError>;

    /// What the plan says its figures come from.
    fn source(&self) -> StructureSource {
        StructureSource::Seam
    }
}

/// The built-in structures tool: it returns the height and cost the engineer gave, and no footprint.
pub struct GivenTool<'a> {
    items: &'a [GivenStructure],
}

impl<'a> GivenTool<'a> {
    /// The pass-through over the given structures.
    pub fn new(items: &'a [GivenStructure]) -> Self {
        Self { items }
    }
}

impl StructuresTool for GivenTool<'_> {
    fn price(&self, req: &[StructureRequest]) -> Result<Vec<StructureResponse>, StageError> {
        req.iter()
            .map(|r| {
                let g = self.items.get(r.id).ok_or_else(|| {
                    refuse("WEIRS_SEAM", format!("no structure {} was given", r.id + 1))
                })?;
                Ok(StructureResponse {
                    id: r.id,
                    height_m: g.height_m,
                    footprint: None,
                    cost: g.cost.clone(),
                })
            })
            .collect()
    }

    fn source(&self) -> StructureSource {
        StructureSource::Given
    }
}

/// One structure, placed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PlacedStructure {
    /// Its place in the input's list.
    pub id: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub kind: StructureKind,
    /// River chainage from the mouth.
    pub river_chainage: Chainage,
    /// Design-line chainage: where the design line's projection onto the river first reaches the
    /// structure's river chainage (where the water stage puts the step).
    pub chainage: Chainage,
    /// Its site on the design line.
    pub point: [f64; 2],
    /// The level it holds upstream (given).
    pub upstream_level_m: f64,
    /// The level below it: the next pool down, or the open river below the lowest.
    pub downstream_level_m: f64,
    /// Upstream less downstream level.
    pub lift_m: f64,
    /// From the structures tool.
    pub height_m: f64,
    /// From the structures tool, when it gives one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footprint: Option<RiverPolygon>,
    /// From the structures tool.
    pub cost: Money,
    /// Where the height, footprint and cost come from.
    pub source: StructureSource,
}

/// One pool: from a structure up to the next one (or to the end of the river).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PlannedPool {
    /// The structure that holds it: its index in the plan's `structures`.
    pub held_by: usize,
    /// River chainage where it starts (at its structure).
    pub river_from: Chainage,
    /// River chainage where it ends.
    pub river_to: Chainage,
    /// Design-line chainage where it starts.
    pub from: Chainage,
    /// Design-line chainage where it ends.
    pub to: Chainage,
    /// Its design level: the upstream level of its structure.
    pub level_m: f64,
    /// Its level less the allowance below.
    pub min_level_m: f64,
    /// Its level plus the allowance above.
    pub max_level_m: f64,
}

/// What the structures stage returns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StructurePlan {
    /// The structures, from the mouth.
    pub structures: Vec<PlacedStructure>,
    /// The pools, from the mouth; pool k is held by structure k.
    pub pools: Vec<PlannedPool>,
    /// The rule the minimum and maximum levels came from.
    pub pool_levels: PoolLevelRule,
    /// The currency of every structure's cost.
    pub currency: String,
}

impl StructurePlan {
    /// The pools as the water stage reads them, on river chainage.
    pub fn water_pools(&self) -> Vec<Pool> {
        self.pools
            .iter()
            .map(|p| Pool {
                from: p.river_from,
                to: p.river_to,
                level_m: p.level_m,
                min_level_m: Some(p.min_level_m),
                max_level_m: Some(p.max_level_m),
            })
            .collect()
    }
}

fn finite(line: &[Pt]) -> bool {
    line.iter().all(|p| p[0].is_finite() && p[1].is_finite())
}

/// Each structure's river chainage, from the mouth: `(input index, chainage)`, sorted. Refuses a
/// site off the river (`WEIRS_SITE_OFF_LINE`) and two structures at one chainage
/// (`WEIRS_SITES_COINCIDE`).
fn on_river(s: &Structures, centreline: &[Pt]) -> Result<Vec<(usize, f64)>, StageError> {
    let river_len = linear::length(centreline);
    let mut at: Vec<(usize, f64)> = Vec::with_capacity(s.items.len());
    for (i, g) in s.items.iter().enumerate() {
        let c = match g.site {
            Site::Chainage(c) => c.0,
            Site::Point(p) => linear::project(centreline, p),
        };
        if !(0.0..=river_len).contains(&c) {
            return Err(refuse(
                "WEIRS_SITE_OFF_LINE",
                format!(
                    "{} is at river chainage {c} m: the river runs from 0 m to {river_len:.1} m",
                    name_of(i, g.name.as_deref())
                ),
            )
            .at(Chainage(c)));
        }
        at.push((i, c));
    }
    at.sort_by(|a, b| a.1.total_cmp(&b.1));
    if let Some(w) = at.windows(2).find(|w| w[1].1 <= w[0].1) {
        return Err(refuse(
            "WEIRS_SITES_COINCIDE",
            format!(
                "{} and {} are both at river chainage {} m: a pool between them would have no \
                 length",
                name_of(w[0].0, s.items[w[0].0].name.as_deref()),
                name_of(w[1].0, s.items[w[1].0].name.as_deref()),
                w[1].1
            ),
        )
        .at(Chainage(w[1].1)));
    }
    Ok(at)
}

/// The level below each structure, sorted from the mouth, after checking the levels step down to
/// the mouth (`WEIRS_LEVELS_NOT_STEPPED`). The lowest's is the given water level at its chainage;
/// without one, `WEIRS_NO_DOWNSTREAM_LEVEL`.
fn downstream_levels(
    s: &Structures,
    at: &[(usize, f64)],
    given: Option<&WaterProfile>,
) -> Result<Vec<f64>, StageError> {
    let level = |k: usize| s.items[at[k].0].upstream_level_m;
    for k in 1..at.len() {
        if level(k) <= level(k - 1) {
            return Err(refuse(
                "WEIRS_LEVELS_NOT_STEPPED",
                format!(
                    "{} at {} m holds {} m, not above the {} m of {} below it: pool levels must \
                     step down to the mouth",
                    name_of(at[k].0, s.items[at[k].0].name.as_deref()),
                    at[k].1,
                    level(k),
                    level(k - 1),
                    name_of(at[k - 1].0, s.items[at[k - 1].0].name.as_deref()),
                ),
            )
            .at(Chainage(at[k].1)));
        }
    }
    let Some(&(i0, c0)) = at.first() else {
        return Ok(vec![]);
    };
    let who = name_of(i0, s.items[i0].name.as_deref());
    let open = match given.filter(|g| !g.is_empty()) {
        Some(g) => g.level_at(Chainage(c0)),
        None => {
            return Err(refuse(
                "WEIRS_NO_DOWNSTREAM_LEVEL",
                format!(
                    "the level below {who}, the lowest structure, is the open river's, read from \
                     the water level profile: there is none. Give a water level"
                ),
            )
            .at(Chainage(c0)));
        }
    };
    if level(0) <= open {
        return Err(refuse(
            "WEIRS_LEVELS_NOT_STEPPED",
            format!(
                "{who} at {c0} m holds {} m, not above the river's own {open} m there: it holds \
                 nothing back",
                level(0)
            ),
        )
        .at(Chainage(c0)));
    }
    let mut down = vec![open];
    down.extend((1..at.len()).map(|k| level(k - 1)));
    Ok(down)
}

/// Check the structures against the river alone: each on the river, no two at one place, levels
/// stepping down to the mouth, and (with a water level profile) the lowest above the open river.
/// For setting the input, before any design line exists.
pub fn check_on_river(
    s: &Structures,
    centreline: &[Pt],
    given: Option<&WaterProfile>,
) -> Result<(), StageError> {
    s.validate()?;
    let at = on_river(s, centreline)?;
    match downstream_levels(s, &at, given) {
        Err(e) if e.code == "WEIRS_NO_DOWNSTREAM_LEVEL" => Ok(()),
        r => r.map(|_| ()),
    }
}

/// The first design-line chainage whose projection onto the river reaches river chainage `c`:
/// sampled at `step`, then bisected. `None` when no point of the line reaches it.
fn line_chainage(design: &[Pt], centre: &[Pt], c: f64, step: f64) -> Option<f64> {
    let len = linear::length(design);
    let r = |s: f64| {
        let p = linear::interpolate(design, s).unwrap_or([f64::NAN; 2]);
        linear::project(centre, p)
    };
    if r(0.0) >= c {
        return ((r(0.0) - c).abs() <= 1e-6).then_some(0.0);
    }
    let n = (len / step).ceil().max(1.0) as usize;
    let mut lo = 0.0;
    for i in 1..=n {
        let s = (i as f64 * step).min(len);
        if r(s) >= c {
            let mut hi = s;
            for _ in 0..200 {
                let mid = lo + (hi - lo) / 2.0;
                if mid <= lo || mid >= hi {
                    break;
                }
                if r(mid) >= c {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            return Some(hi);
        }
        lo = s;
    }
    None
}

/// The structures stage: the given structures placed on the river and the design line, their
/// levels, and the pools between them, priced by `tool`.
///
/// Refusals (stage `structures`): `WEIRS_NO_RIVER`, `WEIRS_NO_DESIGN_LINE`, `WEIRS_SETTING`, the
/// input's own (`WEIRS_NO_STRUCTURES`, `WEIRS_ITEM_INVALID`, `WEIRS_MIXED_CURRENCY`,
/// `WEIRS_POOL_LEVELS_INVALID`), `WEIRS_SITE_OFF_LINE` (off the river, a chainage the design line's
/// projection never reaches, or a point farther than half the channel width from the line),
/// `WEIRS_SITES_COINCIDE`, `WEIRS_LEVELS_NOT_STEPPED`, `WEIRS_NO_DOWNSTREAM_LEVEL`, and
/// `WEIRS_SEAM` (the tool's answer does not match the request). Each with a place has its river
/// chainage.
pub fn structures(
    input: &StructuresIn,
    tool: &dyn StructuresTool,
) -> Result<StructurePlan, StageError> {
    let centre = &input.centreline.0;
    let design = &input.design_line.0;
    if centre.len() < 2 || !finite(centre) {
        return Err(refuse(
            "WEIRS_NO_RIVER",
            "the structures are placed on the river centreline: it needs two finite points",
        ));
    }
    if design.len() < 2 || !finite(design) {
        return Err(refuse(
            "WEIRS_NO_DESIGN_LINE",
            "the structures are placed on the design line: it needs two finite points",
        ));
    }
    if !(input.step_m.is_finite() && input.step_m > 0.0)
        || !(input.channel_width_m.is_finite() && input.channel_width_m > 0.0)
    {
        return Err(refuse(
            "WEIRS_SETTING",
            format!(
                "the search step ({} m) and the channel width ({} m) must be positive numbers",
                input.step_m, input.channel_width_m
            ),
        ));
    }
    let s = &input.structures;
    s.validate()?;
    let at = on_river(s, centre)?;
    let half = input.channel_width_m / 2.0;
    let mut placed: Vec<(f64, [f64; 2])> = Vec::with_capacity(at.len());
    for &(i, c) in &at {
        let g = &s.items[i];
        let off = |why: String| {
            refuse(
                "WEIRS_SITE_OFF_LINE",
                format!(
                    "{} is off the design line: {why}",
                    name_of(i, g.name.as_deref())
                ),
            )
            .at(Chainage(c))
        };
        if let Site::Point(p) = g.site {
            let d = linear::point_line_distance(p, design);
            if d > half + 1e-9 {
                return Err(off(format!(
                    "its point is {d:.1} m from the line, more than half the channel width \
                     ({half} m)"
                )));
            }
        }
        let sl = line_chainage(design, centre, c, input.step_m).ok_or_else(|| {
            off(format!(
                "the design line runs beside no part of the river at chainage {c} m"
            ))
        })?;
        let p = linear::interpolate(design, sl).unwrap_or([f64::NAN; 2]);
        placed.push((sl, p));
    }
    let down = downstream_levels(s, &at, input.given_water.as_ref())?;

    let requests: Vec<StructureRequest> = at
        .iter()
        .zip(&placed)
        .zip(&down)
        .map(|((&(i, c), &(sl, p)), &d)| StructureRequest {
            id: i,
            point: p,
            river_chainage: Chainage(c),
            chainage: Chainage(sl),
            upstream_level_m: s.items[i].upstream_level_m,
            downstream_level_m: d,
            channel_width_m: input.channel_width_m,
        })
        .collect();
    let responses = tool.price(&requests)?;
    let source = tool.source();
    let mut out = Vec::with_capacity(requests.len());
    for r in &requests {
        let mut found = responses.iter().filter(|x| x.id == r.id);
        let (Some(a), None) = (found.next(), found.next()) else {
            return Err(refuse(
                "WEIRS_SEAM",
                format!(
                    "the structures tool must answer once for each structure: structure {} has \
                     no single answer",
                    r.id + 1
                ),
            )
            .at(r.river_chainage));
        };
        if !(a.height_m.is_finite() && a.height_m > 0.0) || a.cost.amount < 0.0 {
            return Err(refuse(
                "WEIRS_SEAM",
                format!(
                    "the structures tool gave structure {} a height of {} m and a cost of {}: a \
                     height is above zero and a cost zero or more",
                    r.id + 1,
                    a.height_m,
                    a.cost.amount
                ),
            )
            .at(r.river_chainage));
        }
        let g = &s.items[r.id];
        out.push(PlacedStructure {
            id: r.id,
            name: g.name.clone(),
            kind: g.kind,
            river_chainage: r.river_chainage,
            chainage: r.chainage,
            point: r.point,
            upstream_level_m: r.upstream_level_m,
            downstream_level_m: r.downstream_level_m,
            lift_m: r.upstream_level_m - r.downstream_level_m,
            height_m: a.height_m,
            footprint: a.footprint.clone(),
            cost: a.cost.clone(),
            source,
        });
    }
    let currency = out[0].cost.currency.clone();
    if let Some(x) = out.iter().find(|x| x.cost.currency != currency) {
        return Err(refuse(
            "WEIRS_MIXED_CURRENCY",
            format!(
                "the structures are priced in {currency} and {}: one currency",
                x.cost.currency
            ),
        )
        .at(x.river_chainage));
    }
    let river_len = linear::length(centre);
    let line_len = linear::length(design);
    let rule = s.pool_levels;
    let pools = (0..out.len())
        .map(|k| {
            let next = out.get(k + 1);
            let level = out[k].upstream_level_m;
            PlannedPool {
                held_by: k,
                river_from: out[k].river_chainage,
                river_to: next.map_or(Chainage(river_len), |n| n.river_chainage),
                from: out[k].chainage,
                to: next.map_or(Chainage(line_len), |n| n.chainage),
                level_m: level,
                min_level_m: level - rule.below_m,
                max_level_m: level + rule.above_m,
            }
        })
        .collect();
    Ok(StructurePlan {
        structures: out,
        pools,
        pool_levels: rule,
        currency,
    })
}

/// The structures stage with the built-in pass-through: the height and cost the engineer gave.
pub fn given(input: &StructuresIn) -> Result<StructurePlan, StageError> {
    structures(input, &GivenTool::new(&input.structures.items))
}

// ---------------------------------------------------------------------------------------------
// The pool checks: navigation depth at each pool's minimum level, flood risk at its maximum.

/// One stored volumes station, as the pool checks read it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CheckStation {
    /// Design-line chainage.
    pub chainage_m: f64,
    /// The design invert.
    pub invert_m: f64,
    /// The design depth.
    pub depth_m: f64,
    /// The ground on the line (the sample nearest the line), when known.
    #[serde(default)]
    pub bed_m: Option<f64>,
    /// The section profile, at the stations that keep one.
    #[serde(default)]
    pub profile: Option<SectionProfile>,
}

/// What the pool checks read: the stored stations and the water stage's stretches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PoolChecksIn {
    /// The stations, in order of chainage.
    pub stations: Vec<CheckStation>,
    /// The water stage's stretches; a pool's carry its minimum and maximum levels.
    pub stretches: Vec<WaterStretch>,
}

impl PoolChecksIn {
    /// What the checks read from the stored volumes and water.
    pub fn from_stored(v: &Volumes, w: &DesignWater) -> Self {
        Self {
            stations: v
                .stations
                .iter()
                .map(|s| CheckStation {
                    chainage_m: s.chainage_m,
                    invert_m: s.invert_m,
                    depth_m: s.depth_m,
                    bed_m: s.bed_m,
                    profile: s.profile.clone(),
                })
                .collect(),
            stretches: w.stretches.clone(),
        }
    }
}

/// A run of stations in one pool that are short of the design depth at its minimum level.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DepthShortfall {
    /// The pool: its index in the water stage's input.
    pub pool: usize,
    /// The first and last station of the run, design-line chainage.
    pub from: f64,
    pub to: f64,
    /// The number of stations in the run.
    pub stations: usize,
    /// The pool's minimum level.
    pub min_level_m: f64,
    /// The design depth at the worst station.
    pub design_depth_m: f64,
    /// The least depth at the minimum level in the run.
    pub least_depth_m: f64,
    /// The design depth less the least depth.
    pub short_by_m: f64,
    /// Where the least depth is.
    pub at: f64,
}

/// Navigation depth at each pool's minimum level.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NavigationDepthCheck {
    /// False when any station in a pool is short.
    pub ok: bool,
    /// The stations in a pool with a minimum level.
    pub checked_stations: usize,
    /// The runs of short stations.
    pub shortfalls: Vec<DepthShortfall>,
    /// What the check reads and what it cannot know.
    pub note: String,
}

/// Which bank.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Left,
    Right,
}

/// A station where a pool's maximum level tops a bank.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FloodFlag {
    /// The pool: its index in the water stage's input.
    pub pool: usize,
    /// Design-line chainage of the station.
    pub chainage_m: f64,
    /// The pool's maximum level.
    pub max_level_m: f64,
    /// The lower bank's ground level.
    pub bank_m: f64,
    /// Which bank is lower.
    pub side: Side,
    /// The maximum level less the bank.
    pub over_by_m: f64,
}

/// Flood risk at each pool's maximum level.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FloodRiskCheck {
    /// False when any station is flagged.
    pub ok: bool,
    /// The stations in a pool with a maximum level whose profile gives a bank.
    pub checked_stations: usize,
    /// The stations in such a pool with no profile, or no bank in it.
    pub unchecked_stations: usize,
    /// The stations whose bank the maximum level tops.
    pub flags: Vec<FloodFlag>,
    /// What the check reads and what it cannot know.
    pub note: String,
}

/// The two pool checks, saved with the volumes when the run has structures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PoolChecks {
    pub navigation_depth: NavigationDepthCheck,
    pub flood_risk: FloodRiskCheck,
}

/// What the navigation depth check reads and cannot know.
pub const NAVIGATION_NOTE: &str = "Reads each stored volumes station inside a pool: the pool's \
minimum level, the design invert, the ground on the line (the one sample nearest the line) and the \
design depth. The bed after the works is the invert, or the ground where it is lower. The depth at \
the minimum level must be at least the design depth. It cannot know the bed across the rest of the \
channel or between stations, siltation, squat or waves, or a level below the stated minimum.";

/// What the flood risk check reads and cannot know.
pub const FLOOD_NOTE: &str = "Reads the stored stations that keep a section profile (one near \
each 500 m and at tight bends; the others are counted as unchecked): the pool's maximum level \
against the bank each side, the first ground outward from the line that is dry today and not cut. \
It cannot know the ground between those stations or beyond the profile's width, flood defences, \
the flow, or backwater above the pool.";

/// The bank each side of a profile: the first sample outward from the line that is dry today, has
/// ground, and is not cut (no cut face at or below the ground).
fn banks(p: &SectionProfile) -> (Option<f64>, Option<f64>) {
    let natural = |j: usize| -> Option<f64> {
        let t = p.terrain_m.get(j).copied().flatten()?;
        let wet = p.wet.get(j).copied().unwrap_or(false);
        let cut = p
            .cut_face_m
            .get(j)
            .copied()
            .flatten()
            .is_some_and(|c| c <= t);
        (!wet && !cut && t.is_finite()).then_some(t)
    };
    let mut left: Vec<usize> = (0..p.offsets_m.len())
        .filter(|&j| p.offsets_m[j] < 0.0)
        .collect();
    left.sort_by(|&a, &b| p.offsets_m[b].total_cmp(&p.offsets_m[a]));
    let mut right: Vec<usize> = (0..p.offsets_m.len())
        .filter(|&j| p.offsets_m[j] > 0.0)
        .collect();
    right.sort_by(|&a, &b| p.offsets_m[a].total_cmp(&p.offsets_m[b]));
    (
        left.into_iter().find_map(natural),
        right.into_iter().find_map(natural),
    )
}

/// The two pool checks on stored stations: navigation depth at each pool's minimum level and flood
/// risk at its maximum level. A station is in the last stretch that starts at or before it (the
/// upper one at a boundary), as the volumes by reach count it.
pub fn pool_checks(input: &PoolChecksIn) -> PoolChecks {
    let stretch_of =
        |c: f64| -> Option<&WaterStretch> { input.stretches.iter().rev().find(|r| r.from.0 <= c) };
    let mut nav_checked = 0;
    let mut shortfalls: Vec<DepthShortfall> = Vec::new();
    let mut open: Option<DepthShortfall> = None;
    let mut flood_checked = 0;
    let mut unchecked = 0;
    let mut flags = Vec::new();
    for st in &input.stations {
        let r = stretch_of(st.chainage_m);
        let pool = r.and_then(|r| r.pool);
        // Navigation depth at the minimum level.
        let mut short_here = None;
        if let (Some(k), Some(min)) = (pool, r.and_then(|r| r.min_level_m)) {
            nav_checked += 1;
            let bed = match st.bed_m {
                Some(b) if b.is_finite() && b < st.invert_m => b,
                _ => st.invert_m,
            };
            let depth = min - bed;
            if depth < st.depth_m - 1e-9 {
                short_here = Some((k, min, depth));
            }
        }
        match (short_here, open.as_mut()) {
            (Some((k, min, depth)), Some(run)) if run.pool == k => {
                run.to = st.chainage_m;
                run.stations += 1;
                if st.depth_m - depth > run.short_by_m {
                    run.short_by_m = st.depth_m - depth;
                    run.least_depth_m = depth;
                    run.design_depth_m = st.depth_m;
                    run.at = st.chainage_m;
                    run.min_level_m = min;
                }
            }
            (Some((k, min, depth)), _) => {
                shortfalls.extend(open.take());
                open = Some(DepthShortfall {
                    pool: k,
                    from: st.chainage_m,
                    to: st.chainage_m,
                    stations: 1,
                    min_level_m: min,
                    design_depth_m: st.depth_m,
                    least_depth_m: depth,
                    short_by_m: st.depth_m - depth,
                    at: st.chainage_m,
                });
            }
            (None, _) => shortfalls.extend(open.take()),
        }
        // Flood risk at the maximum level.
        if let (Some(k), Some(max)) = (pool, r.and_then(|r| r.max_level_m)) {
            let bank = st
                .profile
                .as_ref()
                .map(banks)
                .and_then(|(l, r)| match (l, r) {
                    (Some(l), Some(r)) if r < l => Some((r, Side::Right)),
                    (Some(l), _) => Some((l, Side::Left)),
                    (None, Some(r)) => Some((r, Side::Right)),
                    (None, None) => None,
                });
            match bank {
                None => unchecked += 1,
                Some((b, side)) => {
                    flood_checked += 1;
                    if max > b + 1e-9 {
                        flags.push(FloodFlag {
                            pool: k,
                            chainage_m: st.chainage_m,
                            max_level_m: max,
                            bank_m: b,
                            side,
                            over_by_m: max - b,
                        });
                    }
                }
            }
        }
    }
    shortfalls.extend(open);
    PoolChecks {
        navigation_depth: NavigationDepthCheck {
            ok: shortfalls.is_empty(),
            checked_stations: nav_checked,
            shortfalls,
            note: NAVIGATION_NOTE.into(),
        },
        flood_risk: FloodRiskCheck {
            ok: flags.is_empty(),
            checked_stations: flood_checked,
            unchecked_stations: unchecked,
            flags,
            note: FLOOD_NOTE.into(),
        },
    }
}

impl PoolChecks {
    /// The names of the checks that fail: `navigation_depth`, `flood_risk`.
    pub fn failed(&self) -> Vec<&'static str> {
        let mut f = Vec::new();
        if !self.navigation_depth.ok {
            f.push("navigation_depth");
        }
        if !self.flood_risk.ok {
            f.push("flood_risk");
        }
        f
    }

    /// Each finding in plain words, with its chainage.
    pub fn messages(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .navigation_depth
            .shortfalls
            .iter()
            .map(|s| {
                format!(
                    "navigation_depth: pool {} from {:.0} m to {:.0} m, {} station(s): at its \
                     minimum level {:.2} m the depth is {:.2} m at {:.0} m, {:.2} m short of the \
                     design depth {:.2} m",
                    s.pool,
                    s.from,
                    s.to,
                    s.stations,
                    s.min_level_m,
                    s.least_depth_m,
                    s.at,
                    s.short_by_m,
                    s.design_depth_m
                )
            })
            .collect();
        out.extend(self.flood_risk.flags.iter().map(|f| {
            format!(
                "flood_risk: pool {} at {:.0} m: its maximum level {:.2} m tops the {} bank \
                 ({:.2} m) by {:.2} m",
                f.pool,
                f.chainage_m,
                f.max_level_m,
                match f.side {
                    Side::Left => "left",
                    Side::Right => "right",
                },
                f.bank_m,
                f.over_by_m
            )
        }));
        out
    }
}

/// The pool checks a volumes row holds that fail (`navigation_depth`, `flood_risk`). Empty when the
/// row has none (a run with no structures).
pub fn failed_checks(volumes_row: &Value) -> Vec<&'static str> {
    let c = &volumes_row["structure_checks"];
    let mut f = Vec::new();
    if c["navigation_depth"]["ok"] == false {
        f.push("navigation_depth");
    }
    if c["flood_risk"]["ok"] == false {
        f.push("flood_risk");
    }
    f
}

// ---------------------------------------------------------------------------------------------
// The document.

fn input_error(e: StageError) -> DesignError {
    DesignError::Input(format!("{}: {}", e.code, e.message))
}

impl Session {
    /// The structures the document gives, or `None` when it gives none.
    pub fn structures(&self) -> Result<Option<Structures>, DesignError> {
        match self.doc["inputs"].get(INPUT) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => read_structures(v).map(Some).map_err(input_error),
        }
    }

    /// Store the structures, checked against the river when it is imported (each on it, no two at
    /// one place, levels stepping down to the mouth), or remove them with `None`. The volumes go
    /// stale; no stage before them reads the structures. Nothing changes when they are refused.
    pub fn set_structures(&mut self, s: Option<Structures>) -> Result<(), DesignError> {
        if let Some(s) = &s {
            s.validate().map_err(input_error)?;
            if let Ok(centre) = self.river_centreline() {
                check_on_river(s, &centre, self.input_water_profile().as_ref())
                    .map_err(input_error)?;
            }
        }
        let inputs = self.doc["inputs"]
            .as_object_mut()
            .ok_or_else(|| DesignError::Input("the document has no inputs".into()))?;
        match s {
            None => {
                inputs.shift_remove(INPUT);
            }
            Some(s) => {
                inputs.insert(
                    INPUT.into(),
                    serde_json::to_value(&s).expect("the structures serialise"),
                );
            }
        }
        Ok(())
    }

    /// Set the structures from JSON text: the one function the command line and the app call.
    /// Empty text removes them.
    pub fn set_structures_text(&mut self, text: &str) -> Result<(), DesignError> {
        if text.trim().is_empty() {
            return self.set_structures(None);
        }
        let v: Value = serde_json::from_str(text).map_err(|e| {
            input_error(refuse(
                "WEIRS_INVALID",
                format!("the structures are not JSON: {e}"),
            ))
        })?;
        let s = read_structures(&v).map_err(input_error)?;
        self.set_structures(Some(s))
    }

    /// What the structures stage reads for `design_line`, or `None` when the document gives no
    /// structures.
    pub(super) fn structures_input_on(
        &self,
        design_line: Polyline,
        centreline: Polyline,
    ) -> Result<Option<StructuresIn>, DesignError> {
        let Some(s) = self.structures()? else {
            return Ok(None);
        };
        let size = self.channel_size()?;
        let step_m = self
            .volumes_settings(&Value::Null)
            .map_or(water_stage::DEFAULT_STEP_M, |v| v.station_m);
        Ok(Some(StructuresIn {
            centreline,
            design_line,
            given_water: self.input_water_profile(),
            channel_width_m: size.bottom_width_straight_m,
            structures: s,
            step_m,
        }))
    }

    /// The structure plan on `design_line` with the built-in pass-through, or `None` when the
    /// document gives no structures.
    pub(super) fn structure_plan_on(
        &self,
        design_line: Polyline,
        centreline: Polyline,
    ) -> Result<Option<(StructuresIn, StructurePlan)>, DesignError> {
        match self.structures_input_on(design_line, centreline)? {
            None => Ok(None),
            Some(i) => {
                let plan = given(&i).map_err(input_error)?;
                Ok(Some((i, plan)))
            }
        }
    }

    /// The stored structure plan, as the volumes run last made it. `None` before it has run, or
    /// with no structures.
    pub fn structure_plan(&self) -> Option<StructurePlan> {
        let r = &self.doc["stages"][STAGE]["results"];
        r.is_object()
            .then(|| serde_json::from_value(r.clone()).ok())
            .flatten()
    }
}
