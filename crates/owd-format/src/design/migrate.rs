//! Moving a format 0.4 document to format 0.5 (spec 0017 ticket 6, ADR 0015): one design line.
//!
//! A 0.4 document held a list of design line options; a 0.5 document holds one design line. When a
//! 0.4 file opens:
//!
//! - its rolling-circle option (else its first option that is not the pool lattice) becomes the
//!   alignment's design line, and its method the alignment's method;
//! - its pool-lattice option becomes the optimise stage's refined line (and, with no other option,
//!   the alignment's line too);
//! - the volumes keep the row of the line the later stages now read;
//! - every other option is not kept, and the notes name each one.
//!
//! The owner's rule (20 November 2026, overturning the first cut of ticket 6): a file whose stages
//! were up to date when it was saved opens up to date. So, before the move, each stage's stored hash
//! is checked against the 0.4 formula over the file as stored ([`fresh_0_4`]). A stage that was
//! fresh then, with every stage before it fresh, is kept fresh: after the move it gets the hash of
//! this format, its numbers as they were ([`Session::stamp_fresh`]). A stage that was stale stays
//! stale: its stored numbers are kept, but no stage reads them as fresh until it runs again. The
//! sizing is unchanged.
//!
//! The mouth (spec 0017, chainage from the mouth) is worked out after the move: see
//! [`Session::old_file_mouth`]. The move itself runs on the raw JSON, before the schema check.

use super::design_line::DesignLine;
use super::hash::hash_value;
use super::session::{FORMAT_0_4, FORMAT_VERSION, OpenNote, Session, Stage};
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

/// What the last 0.4 engine hashed for `stage` (the formula of `Session::dependency_hash` before
/// format 0.5), over `s`'s document as stored. `rule_sets` are the selected sets' content hashes.
fn hash_0_4(s: &Session, stage: Stage, rule_sets: &Value) -> String {
    let doc = s.document();
    let st = |x: Stage| &doc["stages"][x.name()];
    let mut chain = serde_json::Map::new();
    for x in Stage::ALL {
        if x == stage {
            chain.insert(x.name().into(), json!({"settings": st(x)["settings"]}));
            break;
        }
        chain.insert(
            x.name().into(),
            json!({"settings": st(x)["settings"], "results": st(x)["results"]}),
        );
    }
    let mut dep = json!({"stage": stage.name(), "chain": Value::Object(chain)});
    dep["rule_sets"] = rule_sets.clone();
    if let Some(r) = doc["inputs"].get("waterway_reaches") {
        dep["waterway_reaches"] = r.clone();
    }
    if stage != Stage::Sizing {
        dep["inputs"] = doc["inputs"].clone();
        if let Some(dem) = dep["inputs"]["dem"].as_object_mut() {
            dem.remove("path");
        }
        let reads_dem = match stage {
            Stage::Alignment => doc["stages"]["alignment"]["results"]["methods"]
                .as_array()
                .is_some_and(|m| m.iter().any(|x| x == "reroute")),
            _ => true,
        };
        if let Some(inputs) = dep["inputs"].as_object_mut() {
            if !reads_dem {
                inputs.remove("dem");
                inputs.remove("water_level");
            }
            if stage != Stage::Volumes {
                inputs.remove("bed_model");
            }
            inputs.remove("reference_line");
        }
        dep["crs"] = doc["project"]["crs"].clone();
        // The bed axis: the reference line (then stored under the inputs) while it was fresh.
        let line = &doc["inputs"]["reference_line"];
        if stage == Stage::Volumes
            && s.bed_model_value().is_some()
            && line.is_object()
            && line["depends_on"].as_str() == Some(super::reference_line::basis_of(doc).as_str())
        {
            dep["bed_axis"] = line["depends_on"].clone();
        }
    }
    if matches!(stage, Stage::Optimise | Stage::Volumes) {
        dep["options"] = doc["options"].clone();
    }
    hash_value(&dep)
}

/// The stages of a format 0.4 document (`doc`, as stored) that were fresh under the 0.4 rules, each
/// with every earlier stage that has results fresh too. A Rule Set the engine does not hold yet (one
/// loaded from a file after opening) counts with the hash it was sized with.
pub(super) fn fresh_0_4(doc: &Value) -> Vec<Stage> {
    let s = Session::over(doc.clone());
    let rule_sets = s.set_hashes_as_sized();
    let mut fresh: Vec<Stage> = Vec::new();
    for stage in Stage::ALL {
        let rec = &doc["stages"][stage.name()];
        if rec["results"].is_null() {
            continue;
        }
        let earlier_fresh = Stage::ALL
            .iter()
            .take_while(|x| **x != stage)
            .all(|x| doc["stages"][x.name()]["results"].is_null() || fresh.contains(x));
        if earlier_fresh
            && rec["depends_on"].as_str() == Some(hash_0_4(&s, stage, &rule_sets).as_str())
        {
            fresh.push(stage);
        }
    }
    fresh
}

/// Move `doc` (format 0.4) to format 0.5 in place. `fresh` are the stages [`fresh_0_4`] found fresh.
/// Returns what changed, one plain sentence each, and the stages after sizing that stay fresh; the
/// caller gives those the hash of this format once the document is a session.
pub fn from_0_4(doc: &mut Value, fresh: &[Stage]) -> (Vec<OpenNote>, Vec<Stage>) {
    let mut notes = vec![OpenNote::info(format!(
        "this design was saved in an older format ({FORMAT_0_4} or before); it opens as format \
         {FORMAT_VERSION}, which holds one design line"
    ))];
    let options: Vec<Value> = doc
        .as_object_mut()
        .and_then(|o| o.remove("options"))
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    let by_method = |m: &str| options.iter().find(|o| o["method"] == m);
    let lattice = by_method("lattice");
    // With no other line, the lattice's own line is the design line too, so the file keeps a design
    // line and its volumes.
    let chosen = by_method("rolling_circle")
        .or_else(|| {
            options
                .iter()
                .find(|o| o["method"] != "lattice" && o["line"].is_object())
        })
        .or(lattice);
    let chosen_is_lattice =
        chosen.is_some() && chosen.map(|c| &c["id"]) == lattice.map(|l| &l["id"]);

    // What stays fresh. The optimise stage's line is the lattice's; with no other line, the
    // alignment holds the lattice's line as the line the later stages were made from, fresh while
    // the optimise stage is.
    let had_alignment = !doc["stages"]["alignment"]["results"].is_null();
    let keep_optimise =
        fresh.contains(&Stage::Optimise) && lattice.and_then(DesignLine::from_old_option).is_some();
    let keep_alignment = chosen.and_then(DesignLine::from_old_option).is_some()
        && if chosen_is_lattice {
            keep_optimise
        } else {
            fresh.contains(&Stage::Alignment)
        };
    let mut keep: Vec<Stage> = Vec::new();

    // The alignment: one line, its method a setting.
    let engine = doc["engine_version"].clone();
    let al = &mut doc["stages"]["alignment"];
    match chosen.and_then(DesignLine::from_old_option) {
        Some(line) => {
            if !al["settings"].is_object() {
                al["settings"] = json!({});
            }
            // A lattice line kept as the design line has no alignment method: the setting stays
            // as it was (the rolling circle by default).
            if super::alignment::AlignMethod::from_name(&line.method).is_some() {
                al["settings"]["method"] = json!(line.method);
            }
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
            if keep_alignment && (had_alignment || chosen_is_lattice) {
                keep.push(Stage::Alignment);
            } else {
                mark_stale(&mut doc["stages"]["alignment"]);
            }
        }
        None if had_alignment => {
            // Results with no line: nothing to keep.
            al["results"] = Value::Null;
            al["depends_on"] = Value::Null;
        }
        None => {}
    }

    // The optimise stage: the lattice's line is its refined line.
    // Kept only after the line it was made from.
    let line_kept =
        keep.contains(&Stage::Alignment) || doc["stages"]["alignment"]["results"].is_null();
    let op = &mut doc["stages"]["optimise"];
    if op["results"].is_object() {
        if let Some(o) = op["results"].as_object_mut() {
            o.remove("option");
        }
        if let Some(line) = lattice.and_then(DesignLine::from_old_option) {
            op["results"]["design_line"] = json!(line);
        }
        if keep_optimise && line_kept {
            keep.push(Stage::Optimise);
        } else {
            mark_stale(&mut doc["stages"]["optimise"]);
        }
    }

    // The volumes: the row of the line the later stages read: the lattice's while the optimise
    // stage is fresh, else the alignment's.
    let reads = if keep.contains(&Stage::Optimise) {
        lattice
    } else {
        chosen
    };
    let before_kept = [Stage::Alignment, Stage::Optimise]
        .iter()
        .all(|x| doc["stages"][x.name()]["results"].is_null() || keep.contains(x));
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
                if fresh.contains(&Stage::Volumes) && before_kept {
                    keep.push(Stage::Volumes);
                } else {
                    mark_stale(&mut doc["stages"]["volumes"]);
                }
            }
            None => {
                vo["results"] = Value::Null;
                vo["depends_on"] = Value::Null;
                notes.push(OpenNote::action(
                    "its volumes held no row for the design line, so they are removed: run \
                     `owd volumes` (or `owd run`) to work them out again"
                        .to_string(),
                ));
            }
        }
    }

    if let Some(line) = reads {
        notes.push(OpenNote::info(format!(
            "its design line is `{}` ({})",
            line["id"].as_str().unwrap_or("?"),
            line["name"].as_str().unwrap_or("?")
        )));
    }
    if let Some(l) = lattice
        && !keep.contains(&Stage::Optimise)
    {
        notes.push(OpenNote::action(format!(
            "the pool lattice's line `{}` is kept in the optimise stage; it is the design line again \
             once `owd optimise` (or `owd run`) has run",
            l["id"].as_str().unwrap_or("lattice")
        )));
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
        notes.push(OpenNote::action(format!(
            "the given line `{}` ({}) is kept in the alignment settings, not as the design line: run \
             `owd align --method given` to make it the design line",
            g["id"].as_str().unwrap_or("?"),
            g["name"].as_str().unwrap_or("?"),
        )));
    }
    for o in &options {
        let kept = chosen.is_some_and(|c| c["id"] == o["id"])
            || lattice.is_some_and(|l| l["id"] == o["id"])
            || given_kept.as_ref().is_some_and(|g| *g == o["id"]);
        if !kept {
            notes.push(OpenNote::action(format!(
                "the option `{}` ({}) is not kept: one design line per run; run `owd align --method {}` to make it the design line",
                o["id"].as_str().unwrap_or("?"),
                o["name"].as_str().unwrap_or("?"),
                o["method"].as_str().unwrap_or("?").replace('_', "-"),
            )));
        }
    }
    let names = |stages: &[&str]| -> String {
        match stages {
            [] => String::new(),
            [one] => (*one).to_string(),
            [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
        }
    };
    let with_results: Vec<&str> = ["alignment", "optimise", "volumes"]
        .into_iter()
        .filter(|s| !doc["stages"][*s]["results"].is_null())
        .collect();
    let kept: Vec<&str> = with_results
        .iter()
        .copied()
        .filter(|s| Stage::from_name(s).is_some_and(|st| keep.contains(&st)))
        .collect();
    let stale: Vec<&str> = with_results
        .iter()
        .copied()
        .filter(|s| !kept.contains(s))
        .collect();
    let plural = |n: usize| {
        if n == 1 {
            ("stage", "was")
        } else {
            ("stages", "were")
        }
    };
    if !kept.is_empty() {
        let (word, was) = plural(kept.len());
        notes.push(OpenNote::info(format!(
            "its {} {word} {was} up to date when it was saved, so {} kept fresh, with {} numbers",
            names(&kept),
            if kept.len() == 1 { "it is" } else { "they are" },
            if kept.len() == 1 { "its" } else { "their" },
        )));
    }
    if !stale.is_empty() {
        let (word, was) = plural(stale.len());
        notes.push(OpenNote::action(format!(
            "its {} {word} {was} not up to date when it was saved, so {} marked stale: the numbers \
             are kept, but run `owd run` before you use them",
            names(&stale),
            if stale.len() == 1 {
                "it is"
            } else {
                "they are"
            },
        )));
    }
    doc["format_version"] = json!(FORMAT_VERSION);
    (notes, keep)
}

impl Session {
    /// A session over a raw document, for reading it with the session's own helpers before it is
    /// checked. Not for callers outside the move.
    pub(super) fn over(doc: Value) -> Session {
        let mut s = Session::new("", "");
        s.doc = doc;
        s
    }

    /// True when `stage` has results and its stored hash is the one it would have now, with a Rule
    /// Set not loaded yet taken as the one the design was sized with.
    pub(super) fn fresh_as_sized(&self, stage: Stage) -> bool {
        let st = &self.doc["stages"][stage.name()];
        !st["results"].is_null()
            && st["depends_on"].as_str()
                == Some(
                    self.dependency_hash_with(stage, self.set_hashes_as_sized())
                        .as_str(),
                )
    }

    /// Give each of `stages` the hash of what it depends on now: fresh, its numbers as they are.
    /// Only for stages known to be up to date (moved from an older format, or turned to run from
    /// the mouth with their results).
    pub(super) fn stamp_fresh(&mut self, stages: &[Stage]) {
        for stage in Stage::ALL.into_iter().filter(|s| stages.contains(s)) {
            if self.doc["stages"][stage.name()]["results"].is_null() {
                continue;
            }
            let h = self.dependency_hash_with(stage, self.set_hashes_as_sized());
            self.doc["stages"][stage.name()]["depends_on"] = json!(h);
        }
    }

    /// Make the design lines of the kept stages whole 0.5 lines: their bends and their cut-offs,
    /// worked out from the stored line as a run works them out. With `sanity`, the line's design
    /// sanity check too (after a turn, its chainages count from the other end). The line, its
    /// checks and every stored number are kept.
    pub(super) fn finish_moved_lines(&mut self, kept: &[Stage], sanity: bool) {
        let river = self.river_geometry().ok();
        let water: Vec<owd_core::geom::Polygon> = river
            .as_ref()
            .and_then(|g| g.polygon.as_ref())
            .map(|p| vec![p.to_polygon()])
            .unwrap_or_default();
        let centre: Vec<[f64; 2]> = river.map(|g| g.centreline.0).unwrap_or_default();
        let finish = |v: &mut Value| {
            let Ok(mut line) = serde_json::from_value::<DesignLine>(v.clone()) else {
                return;
            };
            let r_min = line.r_min_m();
            let chord = line.checks["radius_check_chord_m"]
                .as_f64()
                .unwrap_or_else(|| owd_core::alignment::radius_check::default_check_chord(r_min));
            line.bends = super::design_line::bends(&line.line.0, r_min, chord);
            if !water.is_empty() {
                line.cut_offs = super::design_line::cut_offs(&line.line.0, &water, &centre);
            }
            if sanity {
                line.sanity = super::design_line::sanity(&line.line.0, chord, r_min);
            }
            *v = json!(line);
        };
        if kept.contains(&Stage::Alignment) {
            finish(&mut self.doc["stages"]["alignment"]["results"]);
        }
        if kept.contains(&Stage::Optimise)
            && self.doc["stages"]["optimise"]["results"]["design_line"].is_object()
        {
            finish(&mut self.doc["stages"]["optimise"]["results"]["design_line"]);
        }
    }
}
