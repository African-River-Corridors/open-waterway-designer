//! The river stage (spec 0017, ADR 0015): the river inputs turned into one checked geometry.
//!
//! [`river`] is a pure function from a [`RiverIn`] to a [`RiverGeometry`]. It reads no document and
//! no file. Every later stage is to read the geometry, not the inputs; until each one is moved here,
//! the document still holds the inputs and the stages that have not moved read them, but they read
//! the river polygon through the one parse of [`RiverPolygon::from_geometry`], so islands are kept.
//!
//! The bank check lives here only. The import's warnings and `owd check-banks` both come from
//! [`RiverGeometry::bank_checks`]; neither keeps a copy of the check.

use super::bank_pair::runs_against;
use super::session::side_wholesale_message;
use crate::types::{Polyline, RiverPolygon, StageError};
use owd_core::banks::check::{CheckParams, Checks, assess, lines_cross};
use owd_core::banks::stations::arc;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The stage's name, as the document stores it.
pub const STAGE: &str = "river";

/// The checks' station step on a river's banks, in metres.
pub const STATION_M: f64 = 20.0;

/// What the river stage reads: the river inputs and the CRS they are in.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct RiverIn {
    /// The left bank, looking downstream.
    #[serde(default)]
    pub left_bank: Option<Polyline>,
    /// The right bank, looking downstream.
    #[serde(default)]
    pub right_bank: Option<Polyline>,
    /// The river centreline.
    #[serde(default)]
    pub centreline: Option<Polyline>,
    /// The river as water, with its islands.
    #[serde(default)]
    pub river_polygon: Option<RiverPolygon>,
    /// The EPSG code of the coordinates. A geographic CRS (4326) is checked in the UTM zone of the
    /// middle of the left bank.
    #[serde(default)]
    pub crs_epsg: Option<u32>,
}

/// The two banks, both running the way the centreline runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Banks {
    /// The left bank.
    pub left: Polyline,
    /// The right bank.
    pub right: Polyline,
}

/// One station that failed a bank check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BankCheck {
    /// Chainage of the station along the banks' reference line, in metres.
    pub chainage_m: f64,
    /// The names of the checks it failed: `side`, `missing`, `jump`, `width-wide`, `width-narrow`,
    /// `excursion`, `crossing`.
    pub checks: Vec<String>,
}

/// What the river stage returns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RiverGeometry {
    /// The centreline: the one given, or the smoothed line midway between the banks.
    pub centreline: Polyline,
    /// The banks, each turned to run the way the centreline runs; `None` unless both are given.
    pub banks: Option<Banks>,
    /// The river polygon, islands kept.
    pub polygon: Option<RiverPolygon>,
    /// The length of the centreline in metres.
    pub length_m: f64,
    /// How many stations the bank checks looked at (zero without two banks).
    pub stations_checked: u32,
    /// How many of them failed the side check.
    pub side_fail_stations: u32,
    /// The stations that failed a check, in chainage order.
    pub bank_checks: Vec<BankCheck>,
    /// Where the two banks cross, if they do (the checks report it, they never repair it).
    pub banks_cross: Option<[f64; 2]>,
}

fn refuse(code: &str, message: &str) -> StageError {
    StageError::new(STAGE, code, message)
}

fn finite(line: &[[f64; 2]]) -> bool {
    line.iter().all(|p| p[0].is_finite() && p[1].is_finite())
}

fn check_polygon(p: &RiverPolygon) -> Result<(), StageError> {
    for (what, ring) in std::iter::once(("the outer ring", &p.shell))
        .chain(p.holes.iter().map(|h| ("an island", h)))
    {
        if ring.len() < 3 || !finite(&ring.0) {
            return Err(refuse(
                "RIVER_POLYGON_INVALID",
                &format!("{what} of the river polygon needs three finite points to enclose water"),
            ));
        }
    }
    Ok(())
}

/// The planar copies of the lines the checks run on: a geographic study is moved into the UTM zone of
/// the middle of the left bank (or the centreline), because the checks measure in metres.
fn planar(
    epsg: Option<u32>,
    lines: &mut [&mut Vec<[f64; 2]>],
    anchor: Option<[f64; 2]>,
) -> Result<(), StageError> {
    use super::crs::{Crs, Utm};
    if !matches!(epsg.map(Crs::from_epsg), Some(Ok(Crs::Wgs84))) {
        return Ok(());
    }
    let Some(mid) = anchor else { return Ok(()) };
    let utm = Utm::for_point(mid[0], mid[1]);
    for l in lines.iter_mut() {
        **l = utm.forward_all(l).map_err(|e| {
            refuse(
                "RIVER_CRS_UNSUPPORTED",
                &format!("the river cannot be moved into metres to be checked: {e}"),
            )
        })?;
    }
    Ok(())
}

/// The river stage: the inputs turned into one checked geometry.
pub fn river(input: &RiverIn) -> Result<RiverGeometry, StageError> {
    if let Some(p) = &input.river_polygon {
        check_polygon(p)?;
    }
    let both = match (&input.left_bank, &input.right_bank) {
        (Some(l), Some(r)) if l.len() >= 2 && r.len() >= 2 => Some((l.0.clone(), r.0.clone())),
        _ => None,
    };
    let given = input.centreline.as_ref().filter(|c| c.len() >= 2);
    if given.is_none() && both.is_none() {
        return Err(refuse(
            "RIVER_NO_GEOMETRY",
            "the river needs a centreline or both bank lines",
        ));
    }
    // The centreline the design reads.
    let centreline = match (given, &both) {
        (Some(c), _) => c.0.clone(),
        (None, Some((l, r))) => owd_core::alignment::centreline::build_centreline(l, r, 20.0, 5, 2)
            .map_err(|m| refuse("RIVER_NO_GEOMETRY", &m))?,
        (None, None) => unreachable!("checked above"),
    };

    // Each bank runs the way the centreline runs (or, with no centreline, the way the left bank runs).
    let banks = both.map(|(left, mut right)| {
        let reference = given.map_or(&left[..], |c| &c.0[..]);
        let mut left = left.clone();
        if runs_against(&left, reference) {
            left.reverse();
        }
        if runs_against(&right, reference) {
            right.reverse();
        }
        Banks {
            left: Polyline(left),
            right: Polyline(right),
        }
    });

    // The checks, in metres.
    let (mut length_line, mut seed_line) = (centreline.clone(), given.map(|c| c.0.clone()));
    let mut report = (0_u32, 0_u32, Vec::new(), None);
    if let Some(b) = &banks {
        let (mut left, mut right) = (b.left.0.clone(), b.right.0.clone());
        let anchor = left.get(left.len() / 2).copied();
        {
            let mut all: Vec<&mut Vec<[f64; 2]>> = vec![&mut left, &mut right, &mut length_line];
            if let Some(s) = seed_line.as_mut() {
                all.push(s);
            }
            planar(input.crs_epsg, &mut all, anchor)?;
        }
        report = run_checks(&left, &right, seed_line.as_deref());
    } else {
        let anchor = length_line.get(length_line.len() / 2).copied();
        planar(input.crs_epsg, &mut [&mut length_line], anchor)?;
    }
    let (stations_checked, side_fail_stations, bank_checks, banks_cross) = report;
    Ok(RiverGeometry {
        length_m: owd_core::geom::linear::length(&length_line),
        centreline: Polyline(centreline),
        banks,
        polygon: input.river_polygon.clone(),
        stations_checked,
        side_fail_stations,
        bank_checks,
        banks_cross,
    })
}

/// The bank checks of two banks in metres: the seed is the centreline, or the mid-line of the banks.
fn run_checks(
    left: &[[f64; 2]],
    right: &[[f64; 2]],
    seed: Option<&[[f64; 2]]>,
) -> (u32, u32, Vec<BankCheck>, Option<[f64; 2]>) {
    let centre = match seed {
        Some(c) => c.to_vec(),
        None => super::bank_edit::centreline(left, right, STATION_M),
    };
    // Every station's checks, so the share that fails the side check is known.
    let (reference, _, checks, _, _) =
        assess(left, right, &centre, &CheckParams::at_step(STATION_M), &[]);
    let s = arc(&reference);
    let failed: Vec<BankCheck> = checks
        .iter()
        .enumerate()
        .filter(|(_, c): &(usize, &Checks)| c.any())
        .map(|(i, c)| BankCheck {
            chainage_m: s[i],
            checks: c.names().iter().map(|n| n.to_string()).collect(),
        })
        .collect();
    let side = checks.iter().filter(|c| c.side).count();
    (
        checks.len() as u32,
        side as u32,
        failed,
        lines_cross(left, right),
    )
}

impl RiverGeometry {
    /// The plain message for a side check that fails at most stations, when it does.
    pub fn wholesale_message(&self) -> Option<String> {
        side_wholesale_message(
            self.side_fail_stations as usize,
            self.stations_checked as usize,
        )
    }

    /// The bank checks as the import reports them: one line per failed check (none when every check
    /// passes). With `report_crossing`, banks that cross are reported too, with the first crossing.
    pub fn warnings(&self, report_crossing: bool) -> Vec<String> {
        let mut out = Vec::new();
        // One fact wrong (the direction or the names), not hundreds: one plain message first.
        let wholesale = self.wholesale_message();
        out.extend(wholesale.clone());
        for name in [
            "side",
            "missing",
            "jump",
            "width-wide",
            "width-narrow",
            "excursion",
            "crossing",
        ] {
            if name == "side" && wholesale.is_some() {
                continue;
            }
            let at: Vec<f64> = self
                .bank_checks
                .iter()
                .filter(|c| c.checks.iter().any(|n| n == name))
                .map(|c| c.chainage_m)
                .collect();
            if let (Some(first), Some(last)) = (
                at.iter().copied().reduce(f64::min),
                at.iter().copied().reduce(f64::max),
            ) {
                let n = at.len();
                out.push(format!(
                    "the banks fail the `{name}` check at {n} station{} of {STATION_M} m, between \
                     {first:.0} m and {last:.0} m: run `owd check-banks` for the list",
                    if n == 1 { "" } else { "s" }
                ));
            }
        }
        if report_crossing && let Some(x) = self.banks_cross {
            out.push(format!(
                "the left bank and the right bank cross at ({:.1}, {:.1}): run `owd check-banks` \
                 for the list",
                x[0], x[1]
            ));
        }
        out
    }
}
