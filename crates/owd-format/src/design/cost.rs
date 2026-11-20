//! The rates table and the cost stage (spec 0017, ticket 12): cost as the engineer's rates times
//! OWD's volumes, by reach and in total.
//!
//! The **rates table** (`inputs.rates`) holds one rate for each item, each as [`Money`]: dredge and
//! dry cut per m³, disposal per m³ of all cut, and haul per m³ per km. The haul distance is the run's
//! (`haul_km`), or that of a stretch of design-line chainage (`haul_by_reach`), which may follow
//! the disposal sites and need not match the reaches. One table serves the whole run
//! (spec 0017, open assumption 3), in one currency.
//!
//! [`cost`] is a pure function from a [`CostIn`] to a [`Cost`]. For each reach of the volumes stage:
//!
//! - dredge = dredge rate × dredge m³
//! - dry cut = dry-cut rate × dry cut m³
//! - disposal = disposal rate × (dredge + dry cut) m³
//! - haul = haul rate × the sum, over the reach's stored stations, of each station's cut (dredge
//!   plus dry cut area × the station step) × the haul distance at its chainage
//!
//! and the reach's total is their sum. The haul is taken station by station so that a haul stretch
//! may start or end inside a reach (QA on PR 571). The totals add the reach figures, item by item, so a reader
//! checks them by adding a column. Each reach figure and the totals have room for the output level
//! they reached (`level`), which the design measures stage sets (ticket 17).
//!
//! **The cut-rate multiplier** (ticket 13). When the volumes hold the cut weighted by the multiplier,
//! dredge and dry cut are priced on the weighted volumes: the multiplier multiplies the cut rate where
//! it applies. Disposal and haul are priced on the plain volumes: the multiplier is a rate of
//! cutting, not of moving the spoil. The cost says which it used ([`Cost::cut_volumes`]).
//!
//! **Structures** (ticket 14). When the run has given structures, each structure's cost (from the
//! structures seam) is a `structures` item of the reach of the pool it holds, and of the totals; every
//! reach then carries the item, zero where it holds no structure. With no structures the item is not
//! written, so a cost reads and writes as before.

use super::run_inputs::OutputLevel;
use super::session::{DesignError, Session};
use super::volumes_stage::ReachVolumes;
use crate::types::{Chainage, Money, Reach, StageError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The stage's name, as the document stores it.
pub const STAGE: &str = "cost";

/// The rates table's key in `inputs`.
pub const INPUT: &str = "rates";

/// The name of the rates table's schema contract.
pub const RATES_CONTRACT: &str = "inputs.rates";

/// A haul distance for one stretch of design-line chainage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HaulStretch {
    /// The stretch of design-line chainage, ends included. Each station in it hauls this distance.
    pub reach: Reach,
    /// The haul distance, km.
    pub km: f64,
}

/// The rates table: one rate per item, in one currency. A rate left out is refused when the cost is
/// worked out, never read as zero.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RatesTable {
    /// Per m³ of dredge (cut in the river).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dredge_per_m3: Option<Money>,
    /// Per m³ of dry cut (cut on land).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dry_cut_per_m3: Option<Money>,
    /// Per m³ of all cut, for its disposal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disposal_per_m3: Option<Money>,
    /// Per m³ of all cut per km hauled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub haul_per_m3_km: Option<Money>,
    /// The haul distance of the run, km: for every station no stretch covers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub haul_km: Option<f64>,
    /// Haul distances of stretches of design-line chainage, not overlapping.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub haul_by_reach: Vec<HaulStretch>,
}

fn refuse(code: &str, message: impl AsRef<str>) -> StageError {
    StageError::new(STAGE, code, message.as_ref())
}

impl RatesTable {
    fn rates(&self) -> [(&'static str, &Option<Money>); 4] {
        [
            ("dredge_per_m3", &self.dredge_per_m3),
            ("dry_cut_per_m3", &self.dry_cut_per_m3),
            ("disposal_per_m3", &self.disposal_per_m3),
            ("haul_per_m3_km", &self.haul_per_m3_km),
        ]
    }

    /// The table's one currency: the currency of its rates. Refuses rates in two currencies
    /// (`COST_MIXED_CURRENCY`) and a table with no rate (`COST_RATE_MISSING`).
    pub fn currency(&self) -> Result<String, StageError> {
        let mut found: Option<(&str, &str)> = None;
        for (name, rate) in self.rates() {
            if let Some(m) = rate {
                match found {
                    Some((first_name, first)) if first != m.currency => {
                        return Err(refuse(
                            "COST_MIXED_CURRENCY",
                            format!(
                                "{first_name} is in {first} but {name} is in {}: one rates table \
                                 is in one currency",
                                m.currency
                            ),
                        ));
                    }
                    Some(_) => {}
                    None => found = Some((name, &m.currency)),
                }
            }
        }
        found
            .map(|(_, c)| c.to_string())
            .ok_or_else(|| refuse("COST_RATE_MISSING", "the rates table holds no rate"))
    }

    /// Check what can be checked without the volumes: one currency, no rate or distance below zero
    /// or not finite (`COST_RATE_INVALID`), no two haul stretches over one chainage
    /// (`COST_RATE_INVALID`). A missing rate is refused only by [`cost`], so a table can be built up
    /// a rate at a time.
    pub fn validate(&self) -> Result<(), StageError> {
        if self.rates().iter().any(|(_, r)| r.is_some()) {
            self.currency()?;
        }
        let bad = |what: &str, v: f64| {
            refuse(
                "COST_RATE_INVALID",
                format!("{what} is {v}: a rate or a distance is a finite number, zero or more"),
            )
        };
        for (name, rate) in self.rates() {
            if let Some(m) = rate
                && m.amount < 0.0
            {
                return Err(bad(name, m.amount));
            }
        }
        if let Some(k) = self.haul_km
            && !(k.is_finite() && k >= 0.0)
        {
            return Err(bad("haul_km", k));
        }
        for (i, a) in self.haul_by_reach.iter().enumerate() {
            if !(a.km.is_finite() && a.km >= 0.0) {
                return Err(bad("a haul distance", a.km));
            }
            for b in &self.haul_by_reach[i + 1..] {
                if a.reach.from.0 < b.reach.to.0 && b.reach.from.0 < a.reach.to.0 {
                    return Err(refuse(
                        "COST_RATE_INVALID",
                        format!(
                            "two haul distances cover {} m to {} m",
                            a.reach.from.0.max(b.reach.from.0),
                            a.reach.to.0.min(b.reach.to.0)
                        ),
                    )
                    .at(Chainage(a.reach.from.0.max(b.reach.from.0))));
                }
            }
        }
        Ok(())
    }

    /// The haul distance at a station's chainage: the stretch that holds it (the first, at a
    /// shared end), else the run's. Refuses chainage that no distance covers
    /// (`COST_HAUL_UNCOVERED`, at the station).
    fn haul_km_at(&self, c: Chainage) -> Result<f64, StageError> {
        if let Some(s) = self.haul_by_reach.iter().find(|s| s.reach.contains(c)) {
            return Ok(s.km);
        }
        self.haul_km.ok_or_else(|| {
            refuse(
                "COST_HAUL_UNCOVERED",
                format!(
                    "no haul distance covers chainage {} m: give `haul_km` for the run or a \
                     stretch that covers it",
                    c.0
                ),
            )
            .at(c)
        })
    }
}

/// One stored station's cut, as the cost reads it (the volumes stage's station has more).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CostStation {
    /// Design-line chainage.
    pub chainage_m: f64,
    pub dredge_area_m2: f64,
    pub excav_area_m2: f64,
    /// The weighted areas, when the volumes have them (ticket 13).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weighted_dredge_area_m2: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weighted_excav_area_m2: Option<f64>,
}

/// One structure, as the cost reads it from the stored structure plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CostStructure {
    /// The pool it holds: its index in the water stage's input, as the volumes by reach name it.
    pub pool: usize,
    /// Its design-line chainage.
    pub chainage: Chainage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// What it costs.
    pub cost: Money,
}

/// Which volumes dredge and dry cut were priced on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CutVolumes {
    /// The plain volumes: the run has no cut-rate multiplier.
    #[default]
    Plain,
    /// The volumes weighted by the cut-rate multiplier.
    Weighted,
}

/// What the cost stage reads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CostIn {
    /// The volumes by reach, as the volumes stage stored them.
    pub by_reach: Vec<ReachVolumes>,
    /// The station step, m: each station stands for this length.
    pub station_m: f64,
    /// The stored stations, in order of chainage: each reach holds the next `stations` of them.
    pub stations: Vec<CostStation>,
    /// True when the volumes stage is fresh: nothing it read has changed since it ran.
    pub volumes_fresh: bool,
    /// The rates table, if the run has one.
    #[serde(default)]
    pub rates: Option<RatesTable>,
    /// The structures of the stored structure plan (ticket 14); empty without structures.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub structures: Vec<CostStructure>,
}

/// The cost of each item, and their sum.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CostItems {
    pub dredge: Money,
    pub dry_cut: Money,
    pub disposal: Money,
    pub haul: Money,
    /// The structures (ticket 14): present when the run has structures, zero in a reach with none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structures: Option<Money>,
    /// The sum of the items.
    pub total: Money,
}

/// The cost of one reach of the volumes stage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ReachCost {
    /// Design-line chainage where the reach starts.
    pub from: Chainage,
    /// Design-line chainage where it ends.
    pub to: Chainage,
    /// The reach's cut times its haul distance, summed station by station, m³·km.
    pub haul_m3_km: f64,
    pub items: CostItems,
    /// The output level this figure reached; set by the design measures stage (ticket 17).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<OutputLevel>,
}

/// What the cost stage returns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Cost {
    /// The rates table's currency: every figure is in it.
    pub currency: String,
    /// The cost of each reach, in the volumes' order.
    pub by_reach: Vec<ReachCost>,
    /// Each item added over the reaches.
    pub totals: CostItems,
    /// The output level the totals reached (the lowest reach's); set by ticket 17.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<OutputLevel>,
    /// Which volumes dredge and dry cut were priced on: `weighted` when the volumes hold the cut
    /// weighted by the cut-rate multiplier, else `plain`.
    #[serde(default)]
    pub cut_volumes: CutVolumes,
}

fn money(amount: f64, currency: &str) -> Money {
    Money {
        amount,
        currency: currency.to_string(),
    }
}

fn items(d: f64, x: f64, p: f64, h: f64, s: Option<f64>, currency: &str) -> CostItems {
    CostItems {
        dredge: money(d, currency),
        dry_cut: money(x, currency),
        disposal: money(p, currency),
        haul: money(h, currency),
        structures: s.map(|s| money(s, currency)),
        total: money(d + x + p + h + s.unwrap_or(0.0), currency),
    }
}

/// The cost stage: rates times volumes, by reach and in total.
///
/// Refusals: `COST_NO_RATES` (the run has no rates table: no cost; the run goes on without it),
/// `COST_VOLUMES_STALE` (the volumes were made from inputs that have changed), `COST_NO_VOLUMES` (no
/// volumes by reach are stored), `COST_RATE_MISSING` (a rate, or a reach's haul distance, is not
/// given), `COST_HAUL_UNCOVERED` (a station's chainage that no haul distance covers; at it),
/// `COST_MIXED_CURRENCY` (also a structure in another currency than the rates),
/// `COST_RATE_INVALID`, `COST_VOLUMES_INCONSISTENT` (the stations do not add up to the volumes by
/// reach), and `COST_STRUCTURE_UNPLACED` (a structure holds a pool no reach of the volumes is; at
/// its chainage).
pub fn cost(input: &CostIn) -> Result<Cost, StageError> {
    let rates = input.rates.as_ref().ok_or_else(|| {
        refuse(
            "COST_NO_RATES",
            "the run has no rates table: give one (`owd cost --rates FILE`) for a cost",
        )
    })?;
    if !input.volumes_fresh {
        return Err(refuse(
            "COST_VOLUMES_STALE",
            "the volumes are stale: run the volumes stage again before the cost",
        ));
    }
    if input.by_reach.is_empty() {
        return Err(refuse(
            "COST_NO_VOLUMES",
            "no volumes by reach are stored: run the volumes stage first",
        ));
    }
    rates.validate()?;
    let currency = rates.currency()?;
    let rate = |name: &str, r: &Option<Money>| {
        r.as_ref().map(|m| m.amount).ok_or_else(|| {
            refuse(
                "COST_RATE_MISSING",
                format!("the rates table has no `{name}`: give every rate, zero where it is free"),
            )
        })
    };
    let dredge = rate("dredge_per_m3", &rates.dredge_per_m3)?;
    let dry = rate("dry_cut_per_m3", &rates.dry_cut_per_m3)?;
    let disposal = rate("disposal_per_m3", &rates.disposal_per_m3)?;
    let haul = rate("haul_per_m3_km", &rates.haul_per_m3_km)?;
    let inconsistent = |m: String| refuse("COST_VOLUMES_INCONSISTENT", m);
    // Weighted volumes are priced only when every reach and every station has them.
    let reach_weighted =
        |r: &ReachVolumes| r.weighted_dredge_m3.is_some() && r.weighted_excav_m3.is_some();
    let station_weighted =
        |s: &CostStation| s.weighted_dredge_area_m2.is_some() && s.weighted_excav_area_m2.is_some();
    let weighted = input.by_reach.iter().any(reach_weighted);
    if weighted
        && !(input.by_reach.iter().all(reach_weighted)
            && input.stations.iter().all(station_weighted))
    {
        return Err(inconsistent(
            "some reaches or stations have weighted volumes and others do not: run the volumes \
             stage again"
                .into(),
        ));
    }
    let counted: usize = input.by_reach.iter().map(|r| r.stations).sum();
    if counted != input.stations.len() {
        return Err(refuse(
            "COST_VOLUMES_INCONSISTENT",
            format!(
                "the volumes by reach count {counted} stations but {} are stored: run the volumes \
                 stage again",
                input.stations.len()
            ),
        ));
    }
    // Each structure's cost goes to the first reach of the pool it holds (ticket 14).
    let mut structures_by_reach = vec![0.0; input.by_reach.len()];
    for st in &input.structures {
        if st.cost.currency != currency {
            return Err(refuse(
                "COST_MIXED_CURRENCY",
                format!(
                    "the rates are in {currency} but a structure costs in {}: one cost is in one \
                     currency",
                    st.cost.currency
                ),
            )
            .at(st.chainage));
        }
        let k = input
            .by_reach
            .iter()
            .position(|r| r.pool == Some(st.pool))
            .ok_or_else(|| {
                refuse(
                    "COST_STRUCTURE_UNPLACED",
                    format!(
                        "the structure at {} m holds pool {}, but no reach of the volumes is that \
                         pool: run the volumes stage again",
                        st.chainage.0, st.pool
                    ),
                )
                .at(st.chainage)
            })?;
        structures_by_reach[k] += st.cost.amount;
    }
    let has_structures = !input.structures.is_empty();
    let mut by_reach = Vec::with_capacity(input.by_reach.len());
    let mut sum = [0.0; 4];
    let mut sum_structures = 0.0;
    let mut next = 0;
    for (ri, r) in input.by_reach.iter().enumerate() {
        let stations = &input.stations[next..next + r.stations];
        next += r.stations;
        let mut cut_m3 = 0.0;
        let mut weighted_m3 = 0.0;
        let mut haul_m3_km = 0.0;
        for st in stations {
            let v = (st.dredge_area_m2 + st.excav_area_m2) * input.station_m;
            cut_m3 += v;
            weighted_m3 += (st.weighted_dredge_area_m2.unwrap_or(0.0)
                + st.weighted_excav_area_m2.unwrap_or(0.0))
                * input.station_m;
            haul_m3_km += v * rates.haul_km_at(Chainage(st.chainage_m))?;
        }
        let r_weighted = r.weighted_dredge_m3.unwrap_or(0.0) + r.weighted_excav_m3.unwrap_or(0.0);
        if weighted && (weighted_m3 - r_weighted).abs() > 1e-9 * r_weighted.abs().max(1.0) {
            return Err(inconsistent(format!(
                "the stations of the reach from {} m to {} m hold {weighted_m3} weighted m³ but \
                 the reach holds {r_weighted}: run the volumes stage again",
                r.from.0, r.to.0
            ))
            .at(r.from));
        }
        if (cut_m3 - r.total_m3).abs() > 1e-9 * r.total_m3.abs().max(1.0) {
            return Err(refuse(
                "COST_VOLUMES_INCONSISTENT",
                format!(
                    "the stations of the reach from {} m to {} m hold {cut_m3} m³ but the reach \
                     holds {} m³: run the volumes stage again",
                    r.from.0, r.to.0, r.total_m3
                ),
            )
            .at(r.from));
        }
        let (dredge_m3, dry_m3) = if weighted {
            (
                r.weighted_dredge_m3.unwrap_or(0.0),
                r.weighted_excav_m3.unwrap_or(0.0),
            )
        } else {
            (r.dredge_m3, r.excav_m3)
        };
        let figures = [
            dredge * dredge_m3,
            dry * dry_m3,
            disposal * r.total_m3,
            haul * haul_m3_km,
        ];
        for (s, f) in sum.iter_mut().zip(figures) {
            *s += f;
        }
        let [d, x, p, h] = figures;
        let s = has_structures.then_some(structures_by_reach[ri]);
        sum_structures += structures_by_reach[ri];
        by_reach.push(ReachCost {
            from: r.from,
            to: r.to,
            haul_m3_km,
            items: items(d, x, p, h, s, &currency),
            level: None,
        });
    }
    let [d, x, p, h] = sum;
    Ok(Cost {
        totals: items(
            d,
            x,
            p,
            h,
            has_structures.then_some(sum_structures),
            &currency,
        ),
        currency,
        by_reach,
        level: None,
        cut_volumes: if weighted {
            CutVolumes::Weighted
        } else {
            CutVolumes::Plain
        },
    })
}

/// Read and check a rates table from JSON.
pub fn read_rates(v: &Value) -> Result<RatesTable, StageError> {
    let t: RatesTable = serde_json::from_value(v.clone())
        .map_err(|e| refuse("COST_RATE_INVALID", format!("not a rates table: {e}")))?;
    t.validate()?;
    Ok(t)
}

fn input_error(e: StageError) -> DesignError {
    DesignError::Input(format!("{}: {}", e.code, e.message))
}

impl Session {
    /// The rates table the document holds, or `None` when it holds none.
    pub fn rates(&self) -> Result<Option<RatesTable>, DesignError> {
        match self.doc["inputs"].get(INPUT) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => read_rates(v).map(Some).map_err(input_error),
        }
    }

    /// Store the rates table, checked, or remove it with `None`. No stored stage reads it, so none
    /// goes stale. Nothing changes when it is refused.
    pub fn set_rates(&mut self, rates: Option<RatesTable>) -> Result<(), DesignError> {
        if let Some(r) = &rates {
            r.validate().map_err(input_error)?;
        }
        let inputs = self.doc["inputs"]
            .as_object_mut()
            .ok_or_else(|| DesignError::Input("the document has no inputs".into()))?;
        match rates {
            None => {
                inputs.shift_remove(INPUT);
            }
            Some(r) => {
                inputs.insert(
                    INPUT.into(),
                    serde_json::to_value(&r).expect("a rates table serialises"),
                );
            }
        }
        Ok(())
    }

    /// What the cost stage reads from this document: the volumes stage's stored volumes by reach,
    /// whether that stage is fresh, and the rates table.
    pub fn cost_input(&self) -> Result<CostIn, DesignError> {
        let sections = &self.doc["stages"]["volumes"]["results"]["sections"];
        let read = |key: &str| -> Result<Value, DesignError> {
            Ok(if sections.is_null() {
                Value::Array(vec![])
            } else {
                sections[key].clone()
            })
        };
        let bad = |e: serde_json::Error| {
            DesignError::Input(format!(
                "the volumes stage's stored volumes do not read: {e}. Run the volumes stage again"
            ))
        };
        let by_reach: Vec<ReachVolumes> = serde_json::from_value(read("by_reach")?).map_err(bad)?;
        let stations: Vec<CostStation> = serde_json::from_value(read("stations")?).map_err(bad)?;
        Ok(CostIn {
            by_reach,
            station_m: sections["station_m"].as_f64().unwrap_or(0.0),
            stations,
            volumes_fresh: self.stage_status(super::Stage::Volumes)
                == super::session::StageStatus::Fresh,
            rates: self.rates()?,
            structures: self
                .structure_plan()
                .map(|p| {
                    p.structures
                        .into_iter()
                        .enumerate()
                        .map(|(k, s)| CostStructure {
                            pool: k,
                            chainage: s.chainage,
                            name: s.name,
                            cost: s.cost,
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
    }
}
