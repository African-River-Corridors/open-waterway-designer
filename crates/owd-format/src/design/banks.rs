//! The banks of a design as one dataset (draft ADR 0016, banks are a dataset): the two bank lines,
//! the river polygon with its holes, and what the bank source says about them (who made them, how
//! accurate they are, where it is unsure), with a hash of all of it.
//!
//! No format break: the lines stay in `inputs.left_bank`, `inputs.right_bank` and
//! `inputs.river_polygon`; the rest is the optional member `inputs.banks`. Reading the dataset
//! changes nothing. No stage reads it yet.

use super::hash::sha256_hex;
use super::session::{DesignError, Session};
use owd_core::geom::{Polygon, Pt};
use serde_json::{Value, json};

/// Who made the banks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BankSourceKind {
    /// A file the user gave (`"file"`).
    File,
    /// An edit in the app (`"edit"`).
    Edit,
    /// A bank source (`"bank-source"`).
    BankSource,
}

impl BankSourceKind {
    /// The name the document uses.
    pub fn name(self) -> &'static str {
        match self {
            BankSourceKind::File => "file",
            BankSourceKind::Edit => "edit",
            BankSourceKind::BankSource => "bank-source",
        }
    }

    /// The kind a document name stands for.
    pub fn from_name(s: &str) -> Option<BankSourceKind> {
        match s {
            "file" => Some(BankSourceKind::File),
            "edit" => Some(BankSourceKind::Edit),
            "bank-source" => Some(BankSourceKind::BankSource),
            _ => None,
        }
    }
}

/// Where the banks came from: the kind, a name, the date (`YYYY-MM-DD`) and the licence, if any.
#[derive(Debug, Clone, PartialEq)]
pub struct BankSource {
    pub kind: BankSourceKind,
    pub name: String,
    pub date: String,
    pub licence: Option<String>,
}

/// A stretch of river chainage, in metres, where the source is unsure of the banks.
#[derive(Debug, Clone, PartialEq)]
pub struct LowConfidence {
    pub from_m: f64,
    pub to_m: f64,
    pub reason: String,
}

/// The banks of a design, in the project CRS.
#[derive(Debug, Clone, PartialEq)]
pub struct BankDataset {
    /// `inputs.left_bank`.
    pub left: Vec<Pt>,
    /// `inputs.right_bank`.
    pub right: Vec<Pt>,
    /// `inputs.river_polygon`, with its holes.
    pub river: Option<Polygon>,
    pub source: Option<BankSource>,
    pub accuracy_m: Option<f64>,
    pub low_confidence: Vec<LowConfidence>,
    /// SHA-256 (64 hex characters) of every other part: any change to them gives a new hash.
    pub hash: String,
}

fn points(v: &Value) -> Option<Vec<Pt>> {
    v.as_array()?
        .iter()
        .map(|p| Some([p.get(0)?.as_f64()?, p.get(1)?.as_f64()?]))
        .collect()
}

fn polygon(v: &Value) -> Option<Polygon> {
    let rings: Vec<Vec<Pt>> = v["coordinates"]
        .as_array()?
        .iter()
        .map(points)
        .collect::<Option<_>>()?;
    let (exterior, holes) = rings.split_first()?;
    Some(Polygon {
        exterior: exterior.clone(),
        holes: holes.to_vec(),
    })
}

fn source(v: &Value) -> Option<BankSource> {
    Some(BankSource {
        kind: BankSourceKind::from_name(v["kind"].as_str()?)?,
        name: v["name"].as_str()?.to_string(),
        date: v["date"].as_str()?.to_string(),
        licence: v["licence"].as_str().map(str::to_string),
    })
}

/// The hash of a dataset's parts, from one fixed JSON layout of them.
fn hash_of(d: &BankDataset) -> String {
    let content = json!({
        "left": d.left,
        "right": d.right,
        "river": d.river.as_ref().map(|r| json!({"exterior": r.exterior, "holes": r.holes})),
        "source": d.source.as_ref().map(|s| json!({
            "kind": s.kind.name(), "name": s.name, "date": s.date, "licence": s.licence,
        })),
        "accuracy_m": d.accuracy_m,
        "low_confidence": d.low_confidence.iter().map(|c| json!({
            "from_m": c.from_m, "to_m": c.to_m, "reason": c.reason,
        })).collect::<Vec<_>>(),
    });
    sha256_hex(content.to_string().as_bytes())
}

impl Session {
    /// The banks of the design as one dataset, read from the document. `None` when the document
    /// lacks a bank. Nothing changes.
    pub fn bank_dataset(&self) -> Option<BankDataset> {
        let inputs = &self.doc["inputs"];
        let left = points(&inputs["left_bank"]["coordinates"])?;
        let right = points(&inputs["right_bank"]["coordinates"])?;
        let banks = &inputs["banks"];
        let low_confidence = banks["low_confidence"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|c| {
                        Some(LowConfidence {
                            from_m: c["from_m"].as_f64()?,
                            to_m: c["to_m"].as_f64()?,
                            reason: c["reason"].as_str()?.to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut d = BankDataset {
            left,
            right,
            river: polygon(&inputs["river_polygon"]),
            source: source(&banks["source"]),
            accuracy_m: banks["accuracy_m"].as_f64(),
            low_confidence,
            hash: String::new(),
        };
        d.hash = hash_of(&d);
        Some(d)
    }

    /// Record the accuracy of the bank lines in metres (`None`: unknown) at `inputs.banks`. A new
    /// value makes the stages after sizing stale, through the dependency hash that covers the
    /// inputs. The same value again changes nothing, and `None` on a document with no
    /// `inputs.banks` adds no member. A negative value, or one that is not finite, is refused and
    /// changes nothing.
    pub fn set_bank_accuracy(&mut self, accuracy_m: Option<f64>) -> Result<(), DesignError> {
        if let Some(a) = accuracy_m
            && !(a.is_finite() && a >= 0.0)
        {
            return Err(DesignError::Input(format!(
                "the bank accuracy must be zero or more metres, not {a}"
            )));
        }
        let banks = &self.doc["inputs"]["banks"];
        if banks["accuracy_m"].as_f64() == accuracy_m {
            return Ok(());
        }
        if !banks.is_object() {
            self.doc["inputs"]["banks"] = json!({});
        }
        self.doc["inputs"]["banks"]["accuracy_m"] = json!(accuracy_m);
        Ok(())
    }
}
