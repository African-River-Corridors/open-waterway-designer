//! `owd design-line`: make a line the user already has the design line (ticket C21). It gets the
//! radius check every design line gets; a line under the minimum radius, or one whose footprint
//! enters a no-go area (spec 0017 ticket 13), is saved and marked failed, not refused. With
//! `--strict` a failed check exits 1, as for every command that makes a design line (ticket A13).

use crate::commands::{
    Result, destination, from_design, open, option_checks_outcome, read_vector, say, touched_text,
    write,
};
use owd_format::design::export::failed_checks;

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
    let lines = session.design_lines();
    let option = lines
        .iter()
        .find(|o| o["id"] == id.as_str())
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let c = &option["checks"];
    let measured = c["measured_min_radius_m"]
        .as_f64()
        .map_or("no bend".to_string(), |r| format!("{r:.0} m"));
    let mut human = format!(
        "{id}: minimum radius {measured} on a {:.0} m chord against R_min {:.0} m: {}",
        c["radius_check_chord_m"].as_f64().unwrap_or(f64::NAN),
        c["r_min_m"].as_f64().unwrap_or(f64::NAN),
        if c["radius_ok"] == true {
            "ok"
        } else {
            "FAILS R_min (kept, and reported with the design line)"
        }
    );
    human.push_str(&touched_text(&option));
    let failed = failed_checks(&option);
    if !failed.is_empty() {
        human.push_str(&format!(
            "\nthe design line is saved, marked failed: {}",
            failed.join(", ")
        ));
    }
    say(dest == "-", json_mode, &human, option);
    option_checks_outcome(&failed.join(", "))
}
