//! Moving a format 0.4 document to format 0.5 (spec 0017 ticket 6, ADR 0015): one design line.
//!
//! A 0.4 document held a list of design line options; a 0.5 document holds one design line. When a
//! 0.4 file opens:
//!
//! - its rolling-circle option (else its first option that is not the pool lattice) becomes the
//!   alignment's design line, and its method the alignment's method;
//! - its pool-lattice option becomes the optimise stage's refined line;
//! - the volumes keep the row of the line the later stages now read;
//! - every other option is not kept, and the notes name each one;
//! - the alignment, optimise and volumes stages that had results are marked stale: their stored
//!   numbers are kept as they were, but no stage reads them as fresh until it is run again. The
//!   sizing is unchanged and stays as it was.
//!
//! No mouth is recorded in a 0.4 file, so no line is turned and no stored chainage changes meaning:
//! chainage still runs from the first point of the stored centreline. The notes say how to state the
//! mouth. The move runs on the raw JSON, before the schema check.

use super::design_line::DesignLine;
use super::hash::hash_value;
use super::session::{FORMAT_0_4, FORMAT_VERSION};
use serde_json::{Value, json};

/// The line of a design line as the document writes it.
fn o_line(l: &DesignLine) -> Value {
    json!({"type": "LineString", "coordinates": l.line.0})
}

/// Mark a stage stale: a dependency hash that no input can give, made from the old one.
fn mark_stale(stage: &mut Value) {
    if !stage["results"].is_null() {
        stage["depends_on"] = json!(hash_value(&json!({
            "moved_from_format": FORMAT_0_4,
            "depends_on": stage["depends_on"],
        })));
    }
}

/// Move `doc` (format 0.4) to format 0.5 in place. Returns what changed, one plain sentence each.
pub fn from_0_4(doc: &mut Value) -> Vec<String> {
    let mut notes = vec![format!(
        "this design was saved in an older format ({FORMAT_0_4} or before); it opens as format \
         {FORMAT_VERSION}, which holds one design line"
    )];
    let options: Vec<Value> = doc
        .as_object_mut()
        .and_then(|o| o.remove("options"))
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    let by_method = |m: &str| options.iter().find(|o| o["method"] == m);
    let lattice = by_method("lattice");
    let chosen = by_method("rolling_circle").or_else(|| {
        options
            .iter()
            .find(|o| o["method"] != "lattice" && o["line"].is_object())
    });

    // The alignment: one line, its method a setting.
    let engine = doc["engine_version"].clone();
    let al = &mut doc["stages"]["alignment"];
    let had_results = !al["results"].is_null();
    match chosen.and_then(DesignLine::from_old_option) {
        Some(line) => {
            if !al["settings"].is_object() {
                al["settings"] = json!({});
            }
            al["settings"]["method"] = json!(line.method);
            if line.method == "given" {
                al["settings"]["given"] = json!({"name": line.name, "line": o_line(&line)});
            }
            al["results"] = json!(line);
            if al["depends_on"].is_null() {
                al["depends_on"] = json!(hash_value(&json!("no hash")));
            }
            if al["engine_version"].is_null() {
                al["engine_version"] = engine;
            }
            mark_stale(&mut doc["stages"]["alignment"]);
        }
        None if had_results => {
            // Results with no line: nothing to keep.
            al["results"] = Value::Null;
            al["depends_on"] = Value::Null;
        }
        None => {}
    }

    // The optimise stage: the lattice's line is its refined line.
    let op = &mut doc["stages"]["optimise"];
    if op["results"].is_object() {
        if let Some(o) = op["results"].as_object_mut() {
            o.remove("option");
        }
        if let Some(line) = lattice.and_then(DesignLine::from_old_option) {
            op["results"]["design_line"] = json!(line);
        }
        mark_stale(&mut doc["stages"]["optimise"]);
    }

    // The volumes: the row of the alignment's line. The optimise stage is stale after the move, so
    // its lattice line is not the line the later stages read until it runs again.
    let reads = chosen;
    let vo = &mut doc["stages"]["volumes"];
    if vo["results"].is_object() {
        let row = vo["results"]["options"].as_array().and_then(|rows| {
            rows.iter()
                .find(|r| reads.is_some_and(|o| r["id"] == o["id"]))
                .cloned()
        });
        match row {
            Some(r) => {
                vo["results"] = r;
                mark_stale(&mut doc["stages"]["volumes"]);
            }
            None => {
                vo["results"] = Value::Null;
                vo["depends_on"] = Value::Null;
            }
        }
    }

    if let Some(line) = chosen {
        notes.push(format!(
            "its design line is `{}` ({})",
            line["id"].as_str().unwrap_or("?"),
            line["name"].as_str().unwrap_or("?")
        ));
    }
    if let Some(l) = lattice {
        notes.push(format!(
            "the pool lattice's line `{}` is kept in the optimise stage; it is the design line again \
             once `owd optimise` (or `owd run`) has run",
            l["id"].as_str().unwrap_or("lattice")
        ));
    }
    // A given line that is not the design line keeps its points in the alignment settings, so the
    // user can make it the design line again (`owd align --method given`). One can be kept there.
    let mut given_kept = None;
    if doc["stages"]["alignment"]["settings"]["given"].is_null()
        && let Some(g) = options
            .iter()
            .filter(|o| o["method"] == "given")
            .find(|o| chosen.is_none_or(|c| c["id"] != o["id"]))
        && let Some(line) = DesignLine::from_old_option(g)
    {
        let st = &mut doc["stages"]["alignment"]["settings"];
        if !st.is_object() {
            *st = json!({});
        }
        st["given"] = json!({"name": line.name, "line": o_line(&line)});
        given_kept = Some(g["id"].clone());
        notes.push(format!(
            "the given line `{}` ({}) is kept in the alignment settings, not as the design line: run \
             `owd align --method given` to make it the design line",
            g["id"].as_str().unwrap_or("?"),
            g["name"].as_str().unwrap_or("?"),
        ));
    }
    for o in &options {
        let kept = chosen.is_some_and(|c| c["id"] == o["id"])
            || lattice.is_some_and(|l| l["id"] == o["id"])
            || given_kept.as_ref().is_some_and(|g| *g == o["id"]);
        if !kept {
            notes.push(format!(
                "the option `{}` ({}) is not kept: one design line per run; run `owd align --method {}` to make it the design line",
                o["id"].as_str().unwrap_or("?"),
                o["name"].as_str().unwrap_or("?"),
                o["method"].as_str().unwrap_or("?").replace('_', "-"),
            ));
        }
    }
    let stale: Vec<&str> = ["alignment", "optimise", "volumes"]
        .into_iter()
        .filter(|s| !doc["stages"][*s]["results"].is_null())
        .collect();
    if !stale.is_empty() {
        notes.push(format!(
            "its {} stage{} {} marked stale: the numbers are kept, but run `owd run` before you use them",
            stale.join(", "),
            if stale.len() == 1 { "" } else { "s" },
            if stale.len() == 1 { "is" } else { "are" },
        ));
    }
    notes.push(
        "no mouth is recorded, so chainage still runs from the first point of the stored \
         centreline: state the mouth with `owd import --mouth downstream` (or `--mouth LON,LAT`)"
            .to_string(),
    );
    doc["format_version"] = json!(FORMAT_VERSION);
    notes
}
