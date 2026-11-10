//! A given design line (ticket C21): a line the user already has, from an earlier study or drawn
//! by hand, added as a design line option with the method `given`. It gets the radius check the
//! rolling circle's option gets; a line under the minimum radius is reported, not refused. The
//! volumes stage, the sections and the exports then read it as any option.

use super::alignment::pts_of;
use super::crs::Crs;
use super::geojson::{self, Role};
use super::session::{DesignError, ENGINE_VERSION, Session, Stage, StageStatus};
use owd_core::alignment::radius_check::{default_check_chord, min_radius_on_chord, radius_ok};
use owd_core::geom::Pt;
use serde_json::{Value, json};

/// The method of a given option, as `options[].method` and `stages.alignment.results.methods`
/// write it.
pub const GIVEN: &str = "given";

/// `given-` and the name in lower case, each run of characters that are not ASCII letters or
/// digits one `-`, and no `-` at either end. `None` when the name has no letter or digit.
fn given_id(name: &str) -> Option<String> {
    let mut slug = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-');
    (!slug.is_empty()).then(|| format!("given-{slug}"))
}

/// The radius check of a line, as the rolling circle's option has it.
fn checks(line: &[Pt], r_min: f64) -> Result<Value, DesignError> {
    let chord = default_check_chord(r_min);
    let measured = min_radius_on_chord(line, chord).map_err(|e| DesignError::Input(e.message))?;
    Ok(json!({
        "r_min_m": r_min,
        "radius_ok": radius_ok(measured, r_min),
        "measured_min_radius_m": measured,
        "radius_check_chord_m": chord,
    }))
}

impl Session {
    /// The minimum radius of a fresh sizing, or why there is none.
    fn sized_r_min(&self) -> Result<f64, DesignError> {
        match self.stage_status(Stage::Sizing) {
            StageStatus::Fresh => {}
            StageStatus::NotRun => {
                return Err(DesignError::Input(
                    "a given design line is checked against the minimum radius of the sizing: \
                     choose a vessel class and size the channel first (`owd size`)"
                        .into(),
                ));
            }
            StageStatus::Stale => {
                return Err(DesignError::Input(
                    "the sizing is stale: run `owd size` (or `owd run`) before giving a design line"
                        .into(),
                ));
            }
        }
        self.doc["stages"]["sizing"]["results"]["min_bend_radius_m"]
            .as_f64()
            .ok_or_else(|| DesignError::Input("the sizing has no minimum radius".into()))
    }

    /// Add a design line from GeoJSON text as an option with the method `given`, or replace
    /// the given option of the same name. Returns the option's id.
    pub fn import_design_line(&mut self, geojson: &str, name: &str) -> Result<String, DesignError> {
        let name = name.trim();
        let id = given_id(name).ok_or_else(|| {
            DesignError::Input("a given design line needs a name with a letter or a digit".into())
        })?;
        let parsed = geojson::parse(Role::Centreline, geojson)
            .map_err(|e| DesignError::Input(format!("the design line: {e}")))?;
        let r_min = self.sized_r_min()?;
        let utm = match self.doc["project"]["crs"]["epsg"]
            .as_u64()
            .and_then(|c| Crs::from_epsg(c as u32).ok())
        {
            Some(Crs::Utm(u)) => u,
            _ => {
                return Err(DesignError::Input(
                    "the design has no project CRS: import the river first (`owd import`)".into(),
                ));
            }
        };
        let geometry = geojson::to_project_crs(&parsed, utm)
            .map_err(|e| DesignError::Input(format!("the design line: {e}")))?;
        let line = pts_of(&geometry).filter(|l| l.len() >= 2).ok_or_else(|| {
            DesignError::Input("the design line needs at least two points".into())
        })?;
        let option = json!({
            "id": id,
            "name": name,
            "method": GIVEN,
            "settings": {},
            "line": {"type": "LineString", "coordinates": line},
            "checks": checks(&line, r_min)?,
            "details": {},
        });

        let before = self.stage_status(Stage::Alignment);
        if !self.doc["options"].is_array() {
            self.doc["options"] = json!([]);
        }
        let options = self.doc["options"]
            .as_array_mut()
            .expect("options is a list");
        match options
            .iter()
            .position(|o| o["method"] == GIVEN && o["id"] == id.as_str())
        {
            Some(i) => options[i] = option,
            None => options.push(option),
        }
        // The alignment stage knows its given lines: `given` is one of the methods it ran. A stage
        // that was fresh or not run is fresh after the import; a stale one stays stale.
        let st = &mut self.doc["stages"]["alignment"];
        let mut ran: Vec<Value> = st["results"]["methods"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if !ran.iter().any(|m| m == GIVEN) {
            ran.push(json!(GIVEN));
        }
        st["results"]["methods"] = json!(ran);
        st["engine_version"] = json!(ENGINE_VERSION);
        if before != StageStatus::Stale {
            let h = self.dependency_hash(Stage::Alignment);
            self.doc["stages"]["alignment"]["depends_on"] = json!(h);
        }
        Ok(id)
    }

    /// Work the checks of every given option out again from the sizing of the moment. Nothing else
    /// of them changes, and none is removed.
    pub(super) fn recheck_given_lines(&mut self) -> Result<(), DesignError> {
        let r_min = self.sized_r_min()?;
        let Some(options) = self.doc["options"].as_array_mut() else {
            return Ok(());
        };
        for o in options.iter_mut().filter(|o| o["method"] == GIVEN) {
            let line = pts_of(&o["line"]).unwrap_or_default();
            o["checks"] = checks(&line, r_min)?;
        }
        Ok(())
    }
}
