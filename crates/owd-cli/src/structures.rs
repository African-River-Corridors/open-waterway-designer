//! `owd structures`: given weirs and locks (spec 0017, ticket 14). The file goes to
//! `Session::set_structures_text`, the function the web app calls too. The volumes run places them,
//! stores the plan and saves the pool checks; this command prints them.

use crate::commands::{Result, destination, from_design, input_error, open, say, write};
use owd_format::design::structures::{PoolChecks, StructureKind, StructureSource};
use serde_json::{Value, json};

/// The flags of `owd structures`, as given.
#[derive(Debug, Clone, Default)]
pub struct StructureFlags {
    /// A JSON file of given structures, replacing the stored ones.
    pub given: Option<String>,
    /// Remove the structures.
    pub clear: bool,
}

fn kind(k: StructureKind) -> &'static str {
    match k {
        StructureKind::Weir => "weir",
        StructureKind::Lock => "lock",
        StructureKind::WeirAndLock => "weir and lock",
    }
}

/// Set the structures of a document from the flags; then print the structures, the stored plan and
/// the pool checks. With no flag nothing changes.
pub fn structures(
    doc: &str,
    flags: &StructureFlags,
    output: &Option<String>,
    json_mode: bool,
) -> Result {
    let mut session = open(doc)?;
    let changing = match (&flags.given, flags.clear) {
        (Some(_), true) => return Err(input_error("--clear takes no --given")),
        (Some(path), false) => {
            let text = std::fs::read_to_string(path)
                .map_err(|e| input_error(format!("cannot read {path}: {e}")))?;
            session.set_structures_text(&text).map_err(from_design)?;
            true
        }
        (None, true) => {
            session.set_structures(None).map_err(from_design)?;
            true
        }
        (None, false) => false,
    };
    if changing {
        write(&destination(doc, output), &session.to_text())?;
    }
    let given = session.structures().map_err(from_design)?;
    let plan = session.structure_plan();
    let row = &session.document()["stages"]["volumes"]["results"];
    let checks: Option<PoolChecks> = serde_json::from_value(row["structure_checks"].clone()).ok();
    let mut human = Vec::new();
    match (&given, &plan) {
        (None, _) => human.push("structures: none".to_string()),
        (Some(g), None) => human.push(format!(
            "structures: {} given; run the volumes stage to place them",
            g.items.len()
        )),
        (Some(g), Some(p)) => {
            human.push(format!(
                "structures: {} given; pool levels {:.2} m below and {:.2} m above",
                g.items.len(),
                p.pool_levels.below_m,
                p.pool_levels.above_m
            ));
            for s in &p.structures {
                human.push(format!(
                    "  {} ({}) at river chainage {:.0} m, line chainage {:.0} m: holds {:.2} m \
                     over {:.2} m, lift {:.2} m; height {:.2} m; cost {:.2} {} ({})",
                    s.name
                        .clone()
                        .unwrap_or_else(|| format!("structure {}", s.id + 1)),
                    kind(s.kind),
                    s.river_chainage.0,
                    s.chainage.0,
                    s.upstream_level_m,
                    s.downstream_level_m,
                    s.lift_m,
                    s.height_m,
                    s.cost.amount,
                    s.cost.currency,
                    match s.source {
                        StructureSource::Given => "as given",
                        StructureSource::Seam => "from the structures tool",
                    }
                ));
            }
            for (k, q) in p.pools.iter().enumerate() {
                human.push(format!(
                    "  pool {k}: line chainage {:.0} m to {:.0} m at {:.2} m (minimum {:.2} m, \
                     maximum {:.2} m)",
                    q.from.0, q.to.0, q.level_m, q.min_level_m, q.max_level_m
                ));
            }
        }
    }
    if let Some(c) = &checks {
        human.push(format!(
            "navigation_depth: {} ({} stations checked)",
            if c.navigation_depth.ok { "ok" } else { "FAILS" },
            c.navigation_depth.checked_stations
        ));
        human.push(format!(
            "flood_risk: {} ({} stations checked, {} with no section profile or bank)",
            if c.flood_risk.ok { "ok" } else { "FAILS" },
            c.flood_risk.checked_stations,
            c.flood_risk.unchecked_stations
        ));
        human.extend(c.messages().into_iter().map(|m| format!("  {m}")));
    }
    let stored = session
        .document()
        .pointer("/inputs/structures")
        .cloned()
        .unwrap_or(Value::Null);
    say(
        changing && destination(doc, output) == "-",
        json_mode,
        &human.join("\n"),
        json!({"ok": true, "structures": stored, "plan": plan, "checks": checks}),
    );
    Ok(())
}
