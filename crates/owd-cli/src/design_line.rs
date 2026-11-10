//! `owd design-line`: add a design line the user already has as an option (ticket C21). It gets the
//! radius check the other options get; a line under the minimum radius is reported, not refused.

use crate::commands::{Result, destination, from_design, open, read_vector, say, write};

pub fn design_line(
    doc: &str,
    line: &str,
    name: &str,
    output: &Option<String>,
    json_mode: bool,
) -> Result {
    let text = read_vector(line)?;
    let mut session = open(doc)?;
    let id = session
        .import_design_line(&text, name)
        .map_err(from_design)?;
    let dest = destination(doc, output);
    write(&dest, &session.to_text())?;
    let option = session.document()["options"]
        .as_array()
        .and_then(|o| o.iter().find(|o| o["id"] == id.as_str()))
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let c = &option["checks"];
    let measured = c["measured_min_radius_m"]
        .as_f64()
        .map_or("no bend".to_string(), |r| format!("{r:.0} m"));
    let human = format!(
        "{id}: minimum radius {measured} on a {:.0} m chord against R_min {:.0} m: {}",
        c["radius_check_chord_m"].as_f64().unwrap_or(f64::NAN),
        c["r_min_m"].as_f64().unwrap_or(f64::NAN),
        if c["radius_ok"] == true {
            "ok"
        } else {
            "FAILS R_min (kept, and reported with the option)"
        }
    );
    say(dest == "-", json_mode, &human, option);
    Ok(())
}
