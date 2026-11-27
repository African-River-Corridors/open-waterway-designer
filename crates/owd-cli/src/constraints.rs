//! `owd constraints`: the run's no-go areas and cut-rate multiplier (spec 0017, ticket 13). The flags
//! go to `Session::set_constraints`, the function the web app calls too.

use crate::commands::{
    Result, destination, from_design, input_error, open, relative_path, say, write,
};
use owd_format::design::constraints::{ConstraintsEdit, MultiplierFile, no_go_from_geojson};
use serde_json::{Value, json};

/// The flags of `owd constraints`, as given.
#[derive(Debug, Clone, Default)]
pub struct ConstraintFlags {
    /// A GeoJSON file of no-go areas, replacing the stored ones.
    pub no_go: Option<String>,
    /// Remove every no-go area.
    pub no_no_go: bool,
    /// A cut-rate multiplier raster (GeoTIFF), replacing the stored one.
    pub multiplier: Option<String>,
    /// Remove the multiplier.
    pub no_multiplier: bool,
}

/// Set the constraints of a document from the flags, or, with no flag, print them.
pub fn constraints(
    doc: &str,
    flags: &ConstraintFlags,
    output: &Option<String>,
    json_mode: bool,
) -> Result {
    let mut session = open(doc)?;
    let no_go = match (&flags.no_go, flags.no_no_go) {
        (Some(_), true) => return Err(input_error("--no-no-go takes no --no-go")),
        (Some(path), false) => {
            let text = std::fs::read_to_string(path)
                .map_err(|e| input_error(format!("cannot read {path}: {e}")))?;
            let v: Value = serde_json::from_str(&text)
                .map_err(|e| input_error(format!("{path} is not JSON: {e}")))?;
            Some(
                no_go_from_geojson(&v)
                    .map_err(|e| input_error(format!("{path}: {}: {}", e.code, e.message)))?,
            )
        }
        (None, true) => Some(Vec::new()),
        (None, false) => None,
    };
    let bytes = match (&flags.multiplier, flags.no_multiplier) {
        (Some(_), true) => return Err(input_error("--no-multiplier takes no --multiplier")),
        (Some(path), false) => {
            Some(std::fs::read(path).map_err(|e| input_error(format!("cannot read {path}: {e}")))?)
        }
        _ => None,
    };
    let rel = flags.multiplier.as_deref().map(|p| relative_path(doc, p));
    let multiplier = match (&bytes, &rel) {
        (Some(b), Some(r)) => Some(Some(MultiplierFile { path: r, bytes: b })),
        _ if flags.no_multiplier => Some(None),
        _ => None,
    };
    let changing = no_go.is_some() || multiplier.is_some();
    let c = if changing {
        let c = session
            .set_constraints(ConstraintsEdit { no_go, multiplier })
            .map_err(from_design)?;
        write(&destination(doc, output), &session.to_text())?;
        Some(c)
    } else {
        session.constraints().map_err(from_design)?
    };
    let c = c.filter(|c| !c.is_empty());
    let human = match &c {
        None => "constraints: none".to_string(),
        Some(c) => {
            let areas = if c.no_go.is_empty() {
                "no no-go areas".to_string()
            } else {
                c.no_go
                    .iter()
                    .map(|a| format!("{} ({})", a.name, a.kind))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let m = c
                .cut_rate_multiplier
                .as_ref()
                .map_or("no cut-rate multiplier".to_string(), |m| {
                    format!("cut-rate multiplier {}", m.path)
                });
            format!("constraints: {areas}; {m}")
        }
    };
    let stored = session
        .document()
        .pointer("/inputs/constraints")
        .cloned()
        .unwrap_or(Value::Null);
    say(
        changing && destination(doc, output) == "-",
        json_mode,
        &human,
        json!({"ok": true, "constraints": stored}),
    );
    Ok(())
}
