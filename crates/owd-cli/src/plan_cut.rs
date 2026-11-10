//! `owd plan-cut`: the plan cut of each design line option (ticket C2b), the fairway at the water
//! surface less the river polygon. Plan only: it needs no terrain and no water level.

use crate::commands::{Result, destination, from_design, open, say, write};
use serde_json::{Value, json};

/// The flags of `owd plan-cut`.
#[derive(Debug, Clone, Default)]
pub struct PlanCutFlags {
    /// The smallest piece, in square metres. None: the document's setting, or the default.
    pub min_piece_m2: Option<f64>,
    /// Also write the cut polygons as GeoJSON to this file.
    pub geojson: Option<String>,
    /// With `geojson`: longitude and latitude instead of the project CRS.
    pub wgs84: bool,
    /// The stated height of the proxy volume, in metres. None: the document's setting.
    pub proxy_height_m: Option<f64>,
}

pub fn plan_cut(
    doc: &str,
    flags: PlanCutFlags,
    output: &Option<String>,
    json_mode: bool,
) -> Result {
    let mut session = open(doc)?;
    let mut overrides = serde_json::Map::new();
    if let Some(m) = flags.min_piece_m2 {
        overrides.insert("min_piece_m2".into(), json!(m));
    }
    if let Some(h) = flags.proxy_height_m {
        overrides.insert("proxy_height_m".into(), json!(h));
    }
    session
        .run_plan_cut(&Value::Object(overrides))
        .map_err(from_design)?;
    let dest = destination(doc, output);
    write(&dest, &session.to_text())?;
    if let Some(file) = &flags.geojson {
        let text = session
            .export_plan_cut_geojson(flags.wgs84)
            .map_err(from_design)?;
        write(file, &text)?;
    }
    let results = session.plan_cut_result().cloned().unwrap_or(Value::Null);
    let mut human = vec![results["label"].as_str().unwrap_or("").to_string()];
    for o in results["options"].as_array().into_iter().flatten() {
        let n = |v: &Value| v.as_f64().unwrap_or(f64::NAN);
        human.push(format!(
            "{}: {:.0} m² in {} piece(s) (left {:.0} m², right {:.0} m², largest {:.0} m²; {} piece(s) under the smallest dropped)",
            o["name"].as_str().unwrap_or(o["id"].as_str().unwrap_or("?")),
            n(&o["area_m2"]),
            o["pieces"],
            n(&o["by_bank"]["left_m2"]),
            n(&o["by_bank"]["right_m2"]),
            n(&o["largest_piece_m2"]),
            o["dropped"]["pieces"],
        ));
        if let Some(p) = o.get("proxy_volume") {
            human.push(format!(
                "  proxy volume: {:.0} m³ (plan area × {} m; a proxy, not a terrain volume)",
                n(&p["volume_m3"]),
                p["height_m"]
            ));
        }
    }
    say(dest == "-", json_mode, &human.join("\n"), results);
    Ok(())
}
