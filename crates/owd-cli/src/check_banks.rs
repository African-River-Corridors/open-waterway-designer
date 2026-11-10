//! `owd check-banks`: the spec 0013 bank checks on a document's banks, whatever made them (OWD, a
//! person, or another tool). It prints what fails and changes nothing.

use crate::commands::{Failure, Result, input_error, open, say};
use owd_core::banks::check::{CheckParams, assess, lines_cross};
use owd_core::banks::stations::arc;
use owd_format::design::crs::{Crs, Utm};
use owd_format::design::session::side_wholesale_message;
use serde_json::{Value, json};

/// The checks' station step on a document's banks, in metres.
const STATION_M: f64 = 20.0;

fn line(doc: &Value, role: &str) -> Vec<[f64; 2]> {
    doc["inputs"][role]["coordinates"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| Some([p[0].as_f64()?, p[1].as_f64()?]))
        .collect()
}

pub fn check_banks(doc: &str, json_mode: bool) -> Result {
    let session = open(doc)?;
    let d = session.document();
    let (mut left, mut right, mut centre) = (
        line(d, "left_bank"),
        line(d, "right_bank"),
        line(d, "centreline"),
    );
    if left.len() < 2 || right.len() < 2 {
        return Err(input_error("the document has no bank lines to check"));
    }
    if centre.len() < 2 {
        centre = owd_format::design::bank_edit::centreline(&left, &right, STATION_M);
    }
    // The checks are planar, in metres: a WGS84 study is checked in the UTM zone of its middle.
    let epsg = d["project"]["crs"]["epsg"].as_u64().unwrap_or(0) as u32;
    if matches!(Crs::from_epsg(epsg), Ok(Crs::Wgs84)) {
        let mid = left[left.len() / 2];
        let utm = Utm::for_point(mid[0], mid[1]);
        let to = |pts: &[[f64; 2]]| utm.forward_all(pts).map_err(input_error);
        (left, right, centre) = (to(&left)?, to(&right)?, to(&centre)?);
    }
    // Every station's checks (`check_banks` keeps the failing ones), so the share of stations that
    // fail the side check is known.
    let (reference, _, checks, _, _) = assess(
        &left,
        &right,
        &centre,
        &CheckParams::at_step(STATION_M),
        &[],
    );
    let s = arc(&reference);
    let failed: Vec<(f64, owd_core::banks::check::Checks)> = checks
        .iter()
        .enumerate()
        .filter(|(_, c)| c.any())
        .map(|(i, c)| (s[i], *c))
        .collect();
    let side_fails = failed.iter().filter(|(_, c)| c.side).count();
    let wholesale = side_wholesale_message(side_fails, checks.len());
    let cross = lines_cross(&left, &right);
    let rows: Vec<Value> = failed
        .iter()
        .map(|(at, c)| json!({"chainage_m": at, "checks": c.names()}))
        .collect();
    let mut human = String::new();
    // One fact wrong, not a fault at each station: the plain message in place of the list (the
    // JSON keeps it).
    match &wholesale {
        Some(m) => human.push_str(&format!("{m}\n")),
        None => {
            for (at, c) in &failed {
                human.push_str(&format!("{at:>10.0} m  {}\n", c.names().join(", ")));
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
