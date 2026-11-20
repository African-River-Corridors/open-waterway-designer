//! The value types the stages share (spec 0017, ADR 0015), defined once.
//!
//! Each type that goes into the document derives `Serialize`, `Deserialize` and `JsonSchema`, and
//! writes the same JSON the document already holds: a [`Polyline`] is a list of `[x, y]` pairs, a
//! [`WaterProfile`] a list of `[chainage, level]` pairs, an [`ElevationModel`] the `inputs.dem` entry.
//! The terrain sampler is a function, not a value, so it comes from `owd-core` and is re-exported.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;

pub use owd_core::types::{Refusal, SyncTerrainSampler, TerrainSampler};

/// Distance along a river or a design line, in metres. Chainage 0 is at the river mouth when the river
/// states one (spec 0017); with no mouth stated, it is at the start of the line as it was given.
#[derive(
    Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct Chainage(pub f64);

impl Chainage {
    /// The chainage in metres.
    pub fn metres(self) -> f64 {
        self.0
    }
}

/// A line in the project CRS: `[x, y]` points in order.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct Polyline(pub Vec<[f64; 2]>);

impl Polyline {
    /// The number of points.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// True when there are no points.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The length of the line in metres.
    pub fn length_m(&self) -> f64 {
        owd_core::geom::linear::length(&self.0)
    }
}

/// The river as water: an outer ring and the islands cut out of it. A ring is a [`Polyline`] that is
/// closed by implication (the last point joins the first).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct RiverPolygon {
    /// The outer ring.
    pub shell: Polyline,
    /// Islands: each is dry land inside the shell.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holes: Vec<Polyline>,
}

fn ring_area(ring: &[[f64; 2]]) -> f64 {
    let n = ring.len();
    if n < 3 {
        return 0.0;
    }
    let twice: f64 = (0..n)
        .map(|i| {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum();
    (twice / 2.0).abs()
}

impl RiverPolygon {
    /// The polygon in a GeoJSON `Polygon` geometry: the first ring is the shell, the others are holes.
    /// `None` for anything that is not a `Polygon`. A position that is not two numbers is skipped.
    /// This is the one place the document's river polygon is read.
    pub fn from_geometry(v: &serde_json::Value) -> Option<RiverPolygon> {
        if v["type"] != "Polygon" {
            return None;
        }
        let rings: Vec<Polyline> = v["coordinates"]
            .as_array()?
            .iter()
            .map(|r| {
                Polyline(
                    r.as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|p| Some([p[0].as_f64()?, p[1].as_f64()?]))
                                .collect()
                        })
                        .unwrap_or_default(),
                )
            })
            .collect();
        let (shell, holes) = rings.split_first()?;
        Some(RiverPolygon {
            shell: shell.clone(),
            holes: holes.to_vec(),
        })
    }

    /// The polygon as the core's `Polygon`, holes kept.
    pub fn to_polygon(&self) -> owd_core::geom::Polygon {
        owd_core::geom::Polygon {
            exterior: self.shell.0.clone(),
            holes: self.holes.iter().map(|h| h.0.clone()).collect(),
        }
    }

    /// The water area in square metres: the shell less its holes.
    pub fn area_m2(&self) -> f64 {
        ring_area(&self.shell.0) - self.holes.iter().map(|h| ring_area(&h.0)).sum::<f64>()
    }
}

/// A raster of ground (or bed) heights, by reference: the `inputs.dem` entry of the document. The
/// raster is never embedded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ElevationModel {
    /// Path relative to the document.
    pub path: String,
    /// SHA-256 of the file, in hex.
    pub sha256: String,
    /// The raster's CRS: `EPSG:nnnn`, or `unknown`.
    pub crs: String,
    /// The value that marks no data, if the file has one.
    pub nodata: Option<f64>,
    /// The user marks the raster as holding the river bed below the water level.
    #[serde(default, skip_serializing_if = "is_false")]
    pub holds_bathymetry: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// One point of a water level profile: a river chainage and the level there, in metres. It is
/// written as the pair `[chainage, level]`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(from = "[f64; 2]", into = "[f64; 2]")]
#[schemars(with = "[f64; 2]")]
pub struct WaterPoint {
    /// River chainage.
    pub chainage: Chainage,
    /// Water level in metres above datum.
    pub level_m: f64,
}

impl From<[f64; 2]> for WaterPoint {
    fn from(p: [f64; 2]) -> Self {
        Self {
            chainage: Chainage(p[0]),
            level_m: p[1],
        }
    }
}

impl From<WaterPoint> for [f64; 2] {
    fn from(p: WaterPoint) -> Self {
        [p.chainage.0, p.level_m]
    }
}

/// The water level along a river: points sorted by chainage, read linearly between points and held
/// flat beyond the first and last. Two points at one chainage make a step (a weir): at that chainage
/// the later point holds.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct WaterProfile(Vec<WaterPoint>);

impl WaterProfile {
    /// A profile from `[chainage, level]` pairs, kept in the order given.
    pub fn from_pairs(pairs: &[[f64; 2]]) -> Self {
        Self(pairs.iter().copied().map(WaterPoint::from).collect())
    }

    /// The profile as `[chainage, level]` pairs.
    pub fn to_pairs(&self) -> Vec<[f64; 2]> {
        self.0.iter().copied().map(<[f64; 2]>::from).collect()
    }

    /// The points.
    pub fn points(&self) -> &[WaterPoint] {
        &self.0
    }

    /// The number of points.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// True when there is no point.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The level at `chainage`. NaN for an empty profile or a NaN chainage.
    pub fn level_at(&self, chainage: Chainage) -> f64 {
        let c = chainage.0;
        let (Some(first), Some(last)) = (self.0.first(), self.0.last()) else {
            return f64::NAN;
        };
        if c.is_nan() {
            return f64::NAN;
        }
        if c < first.chainage.0 {
            return first.level_m;
        }
        // The last point at or before the chainage: a step takes its later level.
        let i = self.0.partition_point(|p| p.chainage.0 <= c) - 1;
        if i + 1 >= self.0.len() {
            return last.level_m;
        }
        let (a, b) = (self.0[i], self.0[i + 1]);
        a.level_m + (b.level_m - a.level_m) * (c - a.chainage.0) / (b.chainage.0 - a.chainage.0)
    }
}

/// A stretch of river or design line between two chainages.
///
/// A reach is checked when it is made (`Reach::new`) and again when it is read from JSON.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, JsonSchema)]
pub struct Reach {
    /// The chainage where the reach starts.
    pub from: Chainage,
    /// The chainage where the reach ends. Not below `from` (JSON Schema cannot state a relation
    /// between two fields, so the reader checks it).
    pub to: Chainage,
}

impl<'de> Deserialize<'de> for Reach {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            from: Chainage,
            to: Chainage,
        }
        let r = Raw::deserialize(d)?;
        Reach::new(r.from, r.to).map_err(serde::de::Error::custom)
    }
}

impl Reach {
    /// A reach from `from` to `to`; refused when `to` is below `from` or either is not finite.
    pub fn new(from: Chainage, to: Chainage) -> Result<Self, String> {
        if !(from.0.is_finite() && to.0.is_finite()) {
            return Err("a reach needs finite chainages".into());
        }
        if to.0 < from.0 {
            return Err(format!(
                "a reach cannot end ({} m) before it starts ({} m)",
                to.0, from.0
            ));
        }
        Ok(Self { from, to })
    }

    /// The length in metres.
    pub fn length_m(&self) -> f64 {
        self.to.0 - self.from.0
    }

    /// True when `c` is in the reach, ends included.
    pub fn contains(&self, c: Chainage) -> bool {
        self.from.0 <= c.0 && c.0 <= self.to.0
    }
}

/// An amount of money and its currency.
///
/// An amount is checked when it is made (`Money::new`) and again when it is read from JSON.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Money {
    /// The amount, in the currency's main unit.
    pub amount: f64,
    /// ISO 4217 code: three capital letters.
    #[schemars(regex(pattern = r"^[A-Z]{3}$"))]
    pub currency: String,
}

impl<'de> Deserialize<'de> for Money {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            amount: f64,
            currency: String,
        }
        let r = Raw::deserialize(d)?;
        Money::new(r.amount, &r.currency).map_err(serde::de::Error::custom)
    }
}

impl Money {
    /// An amount in a currency; refused when the amount is not finite or the code is not three
    /// capital letters.
    pub fn new(amount: f64, currency: &str) -> Result<Self, String> {
        if !amount.is_finite() {
            return Err("an amount of money must be a finite number".into());
        }
        if currency.len() != 3 || !currency.bytes().all(|b| b.is_ascii_uppercase()) {
            return Err(format!(
                "a currency code is three capital letters, not `{currency}`"
            ));
        }
        Ok(Self {
            amount,
            currency: currency.to_string(),
        })
    }
}

/// Why a stage refused: the stage, a stable code, a message and, where it applies, the chainage.
/// It is the one error every stage returns (ADR 0015); a stage's `code` never changes once published.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StageError {
    /// The stage's name, as the document stores it.
    pub stage: String,
    /// A stable code, upper case with underscores, such as `RIVER_NO_GEOMETRY`.
    pub code: String,
    /// What went wrong, in plain words.
    pub message: String,
    /// Where on the river or line it went wrong, if it has a place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<Chainage>,
}

impl StageError {
    /// An error with no place.
    pub fn new(stage: &str, code: &str, message: &str) -> Self {
        Self {
            stage: stage.into(),
            code: code.into(),
            message: message.into(),
            at: None,
        }
    }

    /// The same error at a chainage.
    pub fn at(mut self, chainage: Chainage) -> Self {
        self.at = Some(chainage);
        self
    }
}

impl fmt::Display for StageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}: {}", self.stage, self.code, self.message)?;
        if let Some(c) = self.at {
            write!(f, " (at {} m)", c.0)?;
        }
        Ok(())
    }
}

impl std::error::Error for StageError {}
