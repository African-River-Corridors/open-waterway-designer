//! `owd run-inputs`: the run's objective, limits and target level (spec 0017, ticket 11). The flags
//! go to `Session::edit_run_inputs`, the function the app's fields call too.

use crate::commands::{Result, destination, from_design, input_error, open, say, write};
use owd_format::design::run_inputs::RunInputsEdit;
use serde_json::{Value, json};

/// The flags of `owd run-inputs`, as given.
#[derive(Debug, Clone, Default)]
pub struct RunFlags {
    /// `SENSE:MEASURE`, or empty / `none` for none.
    pub objective: Option<String>,
    /// The limits, which together replace the stored ones.
    pub limits: Vec<String>,
    /// Remove every limit.
    pub no_limits: bool,
    /// The target level, or empty for none.
    pub target_level: Option<String>,
}

/// Set the run inputs of a document from the flags, or, with no flag, print them.
pub fn run_inputs(doc: &str, flags: &RunFlags, output: &Option<String>, json_mode: bool) -> Result {
    if flags.no_limits && !flags.limits.is_empty() {
        return Err(input_error("--no-limits takes no --limit"));
    }
    let mut session = open(doc)?;
    let limits = (flags.no_limits || !flags.limits.is_empty()).then_some(flags.limits.as_slice());
    let edit = RunInputsEdit {
        objective: flags.objective.as_deref(),
        limits,
        target_level: flags.target_level.as_deref(),
    };
    let changing = edit.objective.is_some() || edit.limits.is_some() || edit.target_level.is_some();
    let run = if changing {
        let run = session.edit_run_inputs(&edit).map_err(from_design)?;
        write(&destination(doc, output), &session.to_text())?;
        Some(run)
    } else {
        session.run_inputs().map_err(from_design)?
    };
    let run = run.filter(|r| !r.is_empty());
    let target = run.clone().unwrap_or_default().target();
    let human = match &run {
        None => format!(
            "run inputs: none stated (no objective, no limits; the target is {})",
            target.label()
        ),
        Some(r) => {
            let objective = r.objective.as_ref().map_or("none".to_string(), |o| {
                format!("{:?} {}", o.sense, o.measure.name()).to_lowercase()
            });
            let limits = if r.limits.is_empty() {
                "none".to_string()
            } else {
                r.limits
                    .iter()
                    .map(|l| l.to_text())
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            format!(
                "run inputs: objective {objective}; limits {limits}; target {}",
                target.label()
            )
        }
    };
    let stored = session
        .document()
        .pointer("/inputs/run")
        .cloned()
        .unwrap_or(Value::Null);
    say(
        changing && destination(doc, output) == "-",
        json_mode,
        &human,
        json!({"ok": true, "run": stored, "target_level": target}),
    );
    Ok(())
}
