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

/// A bank dataset file (ticket B7), read and checked: the bank inputs as GeoJSON texts for the
/// import, in the order left bank, right bank, river polygon, and the `inputs.banks` member.
#[derive(Debug, Clone, PartialEq)]
pub struct BankFile {
    pub items: Vec<(super::geojson::Role, String)>,
    pub banks: Value,
}

impl BankFile {
    /// The items as the import takes them.
    pub fn items(&self) -> Vec<(super::geojson::Role, &str)> {
        self.items.iter().map(|(r, t)| (*r, t.as_str())).collect()
    }
}

/// Read a bank dataset file: one GeoJSON FeatureCollection with a `bank_dataset` member of version
/// one, and a feature for each of `left_bank`, `right_bank` and, optionally, `river_polygon`. The
/// whole file is checked before anything is imported; a refusal names what is wrong.
pub fn read_bank_file(text: &str) -> Result<BankFile, DesignError> {
    use super::geojson::Role;
    let bad = |m: String| DesignError::Input(format!("the bank dataset file: {m}"));
    let root: Value =
        serde_json::from_str(text).map_err(|e| bad(format!("not valid JSON: {e}")))?;
    let meta = root
        .get("bank_dataset")
        .filter(|m| m.is_object())
        .ok_or_else(|| {
            bad("it has no `bank_dataset` member, so it is not a bank dataset".into())
        })?;
    if meta["version"].as_u64() != Some(1) {
        return Err(bad(format!(
            "`bank_dataset.version` is {}; this reader takes version 1",
            meta["version"]
        )));
    }
    // What the source says, checked as the document's schema checks `inputs.banks`.
    let source = match &meta["source"] {
        Value::Null => Value::Null,
        s @ Value::Object(_) => {
            let text_of = |k: &str| {
                s[k].as_str()
                    .map(str::to_string)
                    .ok_or_else(|| bad(format!("`bank_dataset.source.{k}` must be text")))
            };
            let kind = text_of("kind")?;
            if BankSourceKind::from_name(&kind).is_none() {
                return Err(bad(format!(
                    "`bank_dataset.source.kind` is `{kind}`: use file, edit or bank-source"
                )));
            }
            let licence = match &s["licence"] {
                Value::Null => Value::Null,
                Value::String(l) => json!(l),
                _ => {
                    return Err(bad(
                        "`bank_dataset.source.licence` must be text or null".into()
                    ));
                }
            };
            json!({"kind": kind, "name": text_of("name")?, "date": text_of("date")?, "licence": licence})
        }
        _ => return Err(bad("`bank_dataset.source` must be an object or null".into())),
    };
    let accuracy = match &meta["accuracy_m"] {
        Value::Null => Value::Null,
        a => {
            let x = a
                .as_f64()
                .filter(|x| x.is_finite() && *x >= 0.0)
                .ok_or_else(|| {
                    bad("`bank_dataset.accuracy_m` must be zero or more metres, or null".into())
                })?;
            json!(x)
        }
    };
    let mut stretches = Vec::new();
    for (i, c) in meta["low_confidence"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        let num = |k: &str| {
            c[k].as_f64().ok_or_else(|| {
                bad(format!(
                    "low-confidence stretch {} needs its `{k}` in metres",
                    i + 1
                ))
            })
        };
        let (from_m, to_m) = (num("from_m")?, num("to_m")?);
        let reason = c["reason"].as_str().ok_or_else(|| {
            bad(format!(
                "low-confidence stretch {} needs its `reason`",
                i + 1
            ))
        })?;
        stretches.push(json!({"from_m": from_m, "to_m": to_m, "reason": reason}));
    }
    if !meta["low_confidence"].is_null() && !meta["low_confidence"].is_array() {
        return Err(bad("`bank_dataset.low_confidence` must be a list".into()));
    }

    // The features, by role.
    let crs = root.get("crs").cloned();
    let mut found: Vec<(Role, Value)> = Vec::new();
    for f in root["features"].as_array().into_iter().flatten() {
        let role_name = f["properties"]["role"].as_str().unwrap_or("");
        let role = match role_name {
            "left_bank" => Role::LeftBank,
            "right_bank" => Role::RightBank,
            "river_polygon" => Role::RiverPolygon,
            other => {
                return Err(bad(format!(
                    "a feature has the role `{other}`: the roles are left_bank, right_bank and river_polygon"
                )));
            }
        };
        if found.iter().any(|(r, _)| *r == role) {
            return Err(bad(format!("it has `{role_name}` twice")));
        }
        let mut g = f["geometry"].clone();
        if let (Some(c), Some(o)) = (&crs, g.as_object_mut()) {
            o.insert("crs".into(), c.clone());
        }
        found.push((role, g));
    }
    let mut items = Vec::new();
    for role in [Role::LeftBank, Role::RightBank, Role::RiverPolygon] {
        match found.iter().find(|(r, _)| *r == role) {
            Some((_, g)) => items.push((role, g.to_string())),
            None if role == Role::RiverPolygon => {}
            None => return Err(bad(format!("it has no `{}`", role.name()))),
        }
    }
    Ok(BankFile {
        items,
        banks: json!({"source": source, "accuracy_m": accuracy, "low_confidence": stretches}),
    })
}

impl Session {
    /// Replace `inputs.banks` whole with what a bank dataset file says about the banks.
    pub fn set_bank_metadata(&mut self, banks: &Value) {
        self.doc["inputs"]["banks"] = banks.clone();
    }

    /// Import a bank dataset file: the two banks, the river polygon and what the source says, in
    /// one go, through [`Session::import`]. The whole file is read first; a refusal changes
    /// nothing.
    pub fn import_bank_dataset(
        &mut self,
        text: &str,
    ) -> Result<super::session::ImportReport, DesignError> {
        let file = read_bank_file(text)?;
        let mut s = self.clone();
        let report = s.import(&file.items())?;
        s.set_bank_metadata(&file.banks);
        *self = s;
        Ok(report)
    }

    /// The bank dataset file of the design: in the project CRS with its `crs` member, or in
    /// longitude and latitude with none when `wgs84`. Refused when the design has no banks.
    pub fn export_bank_dataset(&self, wgs84: bool) -> Result<String, DesignError> {
        use super::crs::Crs;
        let d = self.bank_dataset().ok_or_else(|| {
            DesignError::Input("the design has no banks: there is no bank dataset to export".into())
        })?;
        let epsg = self.doc["project"]["crs"]["epsg"]
            .as_u64()
            .map(|c| c as u32);
        let utm = match epsg.and_then(|c| Crs::from_epsg(c).ok()) {
            Some(Crs::Utm(u)) => Some(u),
            _ => None,
        };
        let pts = |line: &[Pt]| -> Result<Value, DesignError> {
            if !wgs84 {
                return Ok(json!(line));
            }
            let u =
                utm.ok_or_else(|| DesignError::Input("the design has no project CRS".into()))?;
            let ll = line
                .iter()
                .map(|p| u.inverse(p[0], p[1]).map_err(DesignError::Input))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(json!(ll))
        };
        let feature = |role: &str, kind: &str, coordinates: Value| {
            json!({
                "type": "Feature",
                "properties": {"role": role},
                "geometry": {"type": kind, "coordinates": coordinates}
            })
        };
        let mut features = vec![
            feature("left_bank", "LineString", pts(&d.left)?),
            feature("right_bank", "LineString", pts(&d.right)?),
        ];
        if let Some(r) = &d.river {
            let mut rings = vec![pts(&r.exterior)?];
            for h in &r.holes {
                rings.push(pts(h)?);
            }
            features.push(feature("river_polygon", "Polygon", json!(rings)));
        }
        let source = d.source.as_ref().map_or(Value::Null, |s| {
            json!({"kind": s.kind.name(), "name": s.name, "date": s.date, "licence": s.licence})
        });
        let mut fc = serde_json::Map::new();
        fc.insert("type".into(), json!("FeatureCollection"));
        if !wgs84 {
            fc.insert(
                "crs".into(),
                json!({"type": "name", "properties": {"name": format!("urn:ogc:def:crs:EPSG::{}", epsg.unwrap_or(0))}}),
            );
        }
        fc.insert(
            "bank_dataset".into(),
            json!({
                "version": 1,
                "source": source,
                "accuracy_m": d.accuracy_m,
                "low_confidence": d.low_confidence.iter().map(|c| json!({
                    "from_m": c.from_m, "to_m": c.to_m, "reason": c.reason,
                })).collect::<Vec<_>>(),
            }),
        );
        fc.insert("features".into(), Value::Array(features));
        let mut t =
            serde_json::to_string_pretty(&Value::Object(fc)).expect("a JSON value serialises");
        t.push('\n');
        Ok(t)
    }
}
