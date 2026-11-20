//! `owd cost`: the rates table and the cost of the stored volumes (spec 0017, ticket 12).

use crate::commands::{Result, destination, from_design, input_error, open, say, write};
use owd_format::design::cost::{cost as cost_stage, read_rates};
use serde_json::json;

/// Store a rates table (when given), then print the cost of the design's stored volumes. A rates
/// table is saved even when no cost can be worked out yet; a refusal exits 2 with its code.
pub fn cost(doc: &str, rates: Option<&str>, output: &Option<String>, json_mode: bool) -> Result {
    let mut session = open(doc)?;
    let dest = destination(doc, output);
    if let Some(path) = rates {
        let text = std::fs::read_to_string(path)
            .map_err(|e| input_error(format!("cannot read {path}: {e}")))?;
        let v: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| input_error(format!("{path}: not JSON: {e}")))?;
        let table = read_rates(&v)
            .map_err(|e| input_error(format!("{path}: {}: {}", e.code, e.message)))?;
        session.set_rates(Some(table)).map_err(from_design)?;
        write(&dest, &session.to_text())?;
    }
    let input = session.cost_input().map_err(from_design)?;
    let c = cost_stage(&input).map_err(|e| input_error(e.to_string()))?;
    let mut human = format!("cost ({}), by reach:", c.currency);
    for r in &c.by_reach {
        human.push_str(&format!(
            "\n  {:.0} m to {:.0} m: dredge {:.2}, dry cut {:.2}, disposal {:.2}, haul {:.2} ({:.0} m³·km); total {:.2}",
            r.from.0,
            r.to.0,
            r.items.dredge.amount,
            r.items.dry_cut.amount,
            r.items.disposal.amount,
            r.items.haul.amount,
            r.haul_m3_km,
            r.items.total.amount
        ));
    }
    human.push_str(&format!(
        "\n  total {:.2} {}",
        c.totals.total.amount, c.currency
    ));
    say(
        rates.is_some() && dest == "-",
        json_mode,
        &human,
        json!({"ok": true, "cost": c}),
    );
    Ok(())
}
