//! A case folder's `case.json`, read and checked before anything runs. A case that breaks a rule
//! here refuses the whole store: the message names the case, the row and what is wrong.

use serde_json::Value;
use std::path::{Path, PathBuf};

/// One case, checked.
#[derive(Debug)]
pub struct Case {
    pub id: String,
    /// The case folder: step paths are relative to it.
    pub folder: PathBuf,
    pub steps: Vec<Step>,
    pub expect: Vec<Expect>,
}

/// One step, played on both surfaces.
#[derive(Debug)]
pub enum Step {
    New {
        title: String,
        date: String,
    },
    Import {
        left_bank: Option<String>,
        right_bank: Option<String>,
        centreline: Option<String>,
        river_polygon: Option<String>,
        dem: Option<String>,
    },
    Size {
        rule_sets: Vec<String>,
        set_inputs: Vec<(String, String)>,
        vessel: Vessel,
    },
    Align {
        method: String,
    },
}

/// The vessel a `size` step sizes for: a class, or a custom vessel.
#[derive(Debug)]
pub enum Vessel {
    Class { id: String, draught_m: Option<f64> },
    Custom(Custom),
}

/// A custom vessel, as `case.json` gives it. `fields` is the object itself, for the app shell.
#[derive(Debug)]
pub struct Custom {
    pub name: String,
    pub loa_m: f64,
    pub beam_m: f64,
    pub draught_m: f64,
    pub formation: String,
    pub n_abreast: Option<u32>,
    pub n_in_line: Option<u32>,
    pub gb_class: Option<String>,
    pub air_draught_m: Option<f64>,
    pub deadweight_t: Option<f64>,
    pub fields: Value,
}

/// How a value read from the document is turned into a number.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Measure {
    /// The value itself, a number.
    Value,
    /// The number of items of an array.
    Count,
    /// The planar length of a LineString geometry, or of a coordinate list.
    LengthM,
}

#[derive(Debug, Clone, Copy)]
pub enum Tolerance {
    Absolute(f64),
    Relative(f64),
}

#[derive(Debug, Clone, Copy)]
pub enum Rule {
    AtLeast(f64),
    AtMost(f64),
}

/// What a row of `expect` asks.
#[derive(Debug, Clone, Copy)]
pub enum Class {
    Exact(Tolerance),
    SameRecipe(Tolerance),
    Outcome(Tolerance),
    KnownDefect(Rule),
    NotYetPossible,
}

impl Class {
    pub fn name(&self) -> &'static str {
        match self {
            Class::Exact(_) => "exact",
            Class::SameRecipe(_) => "same_recipe",
            Class::Outcome(_) => "outcome",
            Class::KnownDefect(_) => "known_defect",
            Class::NotYetPossible => "not_yet_possible",
        }
    }
}

#[derive(Debug)]
pub struct Expect {
    pub name: String,
    pub pointer: String,
    pub measure: Measure,
    /// The expected value. For a known defect, the reference the tool once gave.
    pub value: Option<f64>,
    pub class: Class,
}

/// Every case folder under `cases` (each holds a `case.json`), in name order, checked. The first
/// broken case refuses the whole store.
pub fn read_store(cases: &Path) -> Result<Vec<Case>, String> {
    let entries = std::fs::read_dir(cases)
        .map_err(|e| format!("cannot read the case folder {}: {e}", cases.display()))?;
    let mut folders: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join("case.json").is_file())
        .collect();
    folders.sort();
    if folders.is_empty() {
        return Err(format!(
            "{} holds no case folder (a folder with a case.json)",
            cases.display()
        ));
    }
    folders.iter().map(|f| read_case(f)).collect()
}

fn read_case(folder: &Path) -> Result<Case, String> {
    let file = folder.join("case.json");
    let at = file.display().to_string();
    let text = std::fs::read_to_string(&file).map_err(|e| format!("{at}: {e}"))?;
    let doc: Value = serde_json::from_str(&text).map_err(|e| format!("{at}: not JSON: {e}"))?;
    let id = doc["id"]
        .as_str()
        .ok_or_else(|| format!("{at}: `id` must be text"))?
        .to_string();
    for key in ["study", "source_repository", "source_commit"] {
        if !doc[key].is_string() {
            return Err(format!("case `{id}`: `{key}` must be text"));
        }
    }
    let steps = doc["steps"]
        .as_array()
        .ok_or_else(|| format!("case `{id}`: `steps` must be a list"))?
        .iter()
        .enumerate()
        .map(|(i, s)| step(s).map_err(|e| format!("case `{id}`, step {}: {e}", i + 1)))
        .collect::<Result<Vec<_>, _>>()?;
    let expect = doc["expect"]
        .as_array()
        .ok_or_else(|| format!("case `{id}`: `expect` must be a list"))?
        .iter()
        .enumerate()
        .map(|(i, r)| expect(r, i, &id))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Case {
        id,
        folder: folder.to_path_buf(),
        steps,
        expect,
    })
}

fn text(v: &Value, key: &str) -> Result<String, String> {
    v[key]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("`{key}` must be text"))
}

fn optional_text(v: &Value, key: &str) -> Result<Option<String>, String> {
    match &v[key] {
        Value::Null => Ok(None),
        Value::String(s) => Ok(Some(s.clone())),
        _ => Err(format!("`{key}` must be text")),
    }
}

fn step(s: &Value) -> Result<Step, String> {
    let obj = s
        .as_object()
        .filter(|o| o.len() == 1)
        .ok_or("a step is an object with one key")?;
    let (kind, v) = obj.iter().next().expect("one key");
    match kind.as_str() {
        "new" => Ok(Step::New {
            title: text(v, "title")?,
            date: text(v, "date")?,
        }),
        "import" => Ok(Step::Import {
            left_bank: optional_text(v, "left_bank")?,
            right_bank: optional_text(v, "right_bank")?,
            centreline: optional_text(v, "centreline")?,
            river_polygon: optional_text(v, "river_polygon")?,
            dem: optional_text(v, "dem")?,
        }),
        "size" => {
            let rule_sets = match &v["rule_sets"] {
                Value::Null => Vec::new(),
                Value::Array(a) => a
                    .iter()
                    .map(|x| x.as_str().map(str::to_string))
                    .collect::<Option<Vec<_>>>()
                    .ok_or("`rule_sets` must be a list of text")?,
                _ => return Err("`rule_sets` must be a list of text".into()),
            };
            let set_inputs = match &v["set_inputs"] {
                Value::Null => Vec::new(),
                Value::Object(o) => o
                    .iter()
                    .map(|(k, x)| match x {
                        Value::String(s) => Some((k.clone(), s.clone())),
                        Value::Number(n) => Some((k.clone(), n.to_string())),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()
                    .ok_or("each set input must be text or a number")?,
                _ => return Err("`set_inputs` must be an object".into()),
            };
            let draught_m = match &v["draught_m"] {
                Value::Null => None,
                x => Some(x.as_f64().ok_or("`draught_m` must be a number")?),
            };
            let vessel = match (&v["vessel"], &v["custom_vessel"]) {
                (Value::Null, Value::Null) => {
                    return Err("a `size` step needs `vessel` or `custom_vessel`".into());
                }
                (Value::Null, c) => {
                    if draught_m.is_some() {
                        return Err(
                            "`draught_m` goes inside `custom_vessel`: the custom vessel carries its draught"
                                .into(),
                        );
                    }
                    Vessel::Custom(custom(c)?)
                }
                (_, Value::Null) => Vessel::Class {
                    id: text(v, "vessel")?,
                    draught_m,
                },
                _ => {
                    return Err(
                        "give `vessel` or `custom_vessel`, not both: a custom vessel is not a class"
                            .into(),
                    );
                }
            };
            Ok(Step::Size {
                rule_sets,
                set_inputs,
                vessel,
            })
        }
        "align" => Ok(Step::Align {
            method: text(v, "method")?,
        }),
        other => Err(format!(
            "`{other}` is not a step: use new, import, size or align"
        )),
    }
}

/// A custom vessel object: its required and optional members, by the names `case.json` uses.
fn custom(c: &Value) -> Result<Custom, String> {
    if !c.is_object() {
        return Err("`custom_vessel` must be an object".into());
    }
    let need = |k: &str| {
        c[k].as_f64()
            .ok_or_else(|| format!("`custom_vessel.{k}` must be a number"))
    };
    let maybe = |k: &str| match &c[k] {
        Value::Null => Ok(None),
        x => x
            .as_f64()
            .map(Some)
            .ok_or_else(|| format!("`custom_vessel.{k}` must be a number")),
    };
    let count = |k: &str| match &c[k] {
        Value::Null => Ok(None),
        x => x
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .map(Some)
            .ok_or_else(|| format!("`custom_vessel.{k}` must be a whole number")),
    };
    let text_of = |k: &str| {
        c[k].as_str()
            .map(str::to_string)
            .ok_or_else(|| format!("`custom_vessel.{k}` must be text"))
    };
    Ok(Custom {
        name: text_of("name")?,
        loa_m: need("loa_m")?,
        beam_m: need("beam_m")?,
        draught_m: need("draught_m")?,
        formation: text_of("formation")?,
        n_abreast: count("n_abreast")?,
        n_in_line: count("n_in_line")?,
        gb_class: match &c["gb_class"] {
            Value::Null => None,
            _ => Some(text_of("gb_class")?),
        },
        air_draught_m: maybe("air_draught_m")?,
        deadweight_t: maybe("deadweight_t")?,
        fields: c.clone(),
    })
}

/// A number, or a boolean as 1 (true) or 0 (false).
pub fn number(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_bool().map(|b| if b { 1.0 } else { 0.0 }))
}

fn expect(r: &Value, i: usize, id: &str) -> Result<Expect, String> {
    let name = r["name"]
        .as_str()
        .ok_or_else(|| format!("case `{id}`: expected value {} has no `name`", i + 1))?
        .to_string();
    let at = format!("case `{id}`, expected value `{name}`");
    // Every expected value says where it comes from. No source, no case.
    match r["source"].as_str() {
        Some(s) if !s.trim().is_empty() => {}
        _ => {
            return Err(format!(
                "{at}: it has no `source`: say where the value comes from"
            ));
        }
    }
    let pointer = r["read"]["pointer"]
        .as_str()
        .ok_or_else(|| format!("{at}: `read.pointer` must be text"))?
        .to_string();
    let measure = match r["read"]["measure"].as_str() {
        None if r["read"]["measure"].is_null() => Measure::Value,
        Some("count") => Measure::Count,
        Some("length_m") => Measure::LengthM,
        _ => return Err(format!("{at}: `read.measure` must be count or length_m")),
    };
    let tolerance = || -> Result<Tolerance, String> {
        let t = &r["tolerance"];
        match (number(&t["absolute"]), number(&t["relative"])) {
            (Some(a), None) if a >= 0.0 => Ok(Tolerance::Absolute(a)),
            (None, Some(x)) if x >= 0.0 => Ok(Tolerance::Relative(x)),
            _ => Err(format!(
                "{at}: needs `tolerance`: {{\"absolute\": x}} or {{\"relative\": x}}, x not negative"
            )),
        }
    };
    let class_name = r["class"]
        .as_str()
        .ok_or_else(|| format!("{at}: `class` must be text"))?;
    let class = match class_name {
        "exact" => Class::Exact(tolerance()?),
        "same_recipe" => Class::SameRecipe(tolerance()?),
        "outcome" => Class::Outcome(tolerance()?),
        "known_defect" => {
            let rule = &r["rule"];
            match (number(&rule["at_least"]), number(&rule["at_most"])) {
                (Some(x), None) => Class::KnownDefect(Rule::AtLeast(x)),
                (None, Some(x)) => Class::KnownDefect(Rule::AtMost(x)),
                _ => {
                    return Err(format!(
                        "{at}: a known defect needs `rule`: {{\"at_least\": x}} or {{\"at_most\": x}}"
                    ));
                }
            }
        }
        "not_yet_possible" => Class::NotYetPossible,
        other => {
            return Err(format!(
                "{at}: `{other}` is not a class: use exact, same_recipe, outcome, known_defect or not_yet_possible"
            ));
        }
    };
    let value = number(&r["value"]);
    let needs_value = matches!(
        class,
        Class::Exact(_) | Class::SameRecipe(_) | Class::Outcome(_)
    );
    if needs_value && value.is_none() {
        return Err(format!("{at}: `value` must be a number"));
    }
    Ok(Expect {
        name,
        pointer,
        measure,
        value,
        class,
    })
}
