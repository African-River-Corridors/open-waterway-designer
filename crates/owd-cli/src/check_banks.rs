//! `owd check-banks`: the spec 0013 bank checks on a document's banks, whatever made them (OWD, a
//! person, or another tool). It prints what fails and changes nothing.

use crate::commands::{Failure, Result, input_error, open, say};
use owd_format::design::river::STATION_M;
use serde_json::{Value, json};

pub fn check_banks(doc: &str, json_mode: bool) -> Result {
    let session = open(doc)?;
    let input = session.river_input();
    let has = |l: &Option<owd_format::types::Polyline>| l.as_ref().is_some_and(|l| l.len() >= 2);
    if !has(&input.left_bank) || !has(&input.right_bank) {
        return Err(input_error("the document has no bank lines to check"));
    }
    // The checks are the river stage's, on the same input the import's warnings read (the polygon is
    // not under test): the two cannot differ.
    let river = session
        .bank_check_geometry()
        .map_err(|e| input_error(e.to_string()))?;
    let failed: Vec<(f64, Vec<String>)> = river
        .bank_checks
        .iter()
        .map(|c| (c.chainage_m, c.checks.clone()))
        .collect();
    let cross = river.banks_cross;
    let rows: Vec<Value> = failed
        .iter()
        .map(|(at, c)| json!({"chainage_m": at, "checks": c}))
        .collect();
    let mut human = String::new();
    // One fact wrong, not a fault at each station: the plain message in place of the list (the
    // JSON keeps it).
    match river.wholesale_message() {
        Some(m) => human.push_str(&format!("{m}\n")),
        None => {
            for (at, c) in &failed {
                human.push_str(&format!("{at:>10.0} m  {}\n", c.join(", ")));
            }
        }
    }
    if let Some(x) = cross {
        human.push_str(&format!("the banks cross at ({:.1}, {:.1})\n", x[0], x[1]));
    }
    let ok = failed.is_empty() && cross.is_none();
    human.push_str(&if ok {
        "the banks pass every check".to_string()
    } else {
        format!(
            "{} station(s) of {STATION_M} m fail a check{}",
            failed.len(),
            if cross.is_some() {
                ", and the banks cross"
            } else {
                ""
            }
        )
    });
    say(
        false,
        json_mode,
        &human,
        json!({"ok": ok, "station_m": STATION_M, "failed": rows, "crossing": cross}),
    );
    if ok {
        Ok(())
    } else {
        Err(Failure {
            message: "the banks fail the checks (see above)".into(),
            code: 1,
        })
    }
}
