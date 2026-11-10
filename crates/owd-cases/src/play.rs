//! Play a case's steps on the two surfaces: the command line's functions on a file, and the app
//! shell's `AppState`. Each surface works in its own temporary folder holding a copy of the case's
//! files, so the case folder is never written and a DEM path reads the same on both.

use crate::case::{Case, Step};
use owd_cli::commands::{self, AlignFlags, SizeFlags, WaterFlags};
use owd_format::design::Role;
use owd_shell::state::AppState;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// A temporary folder, removed when dropped.
pub struct Work(pub PathBuf);

impl Work {
    pub fn new(case: &Case, surface: &str) -> Result<Work, String> {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("owd-cases-{}-{n}-{surface}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        copy_dir(&case.folder, &dir).map_err(|e| format!("cannot copy the case: {e}"))?;
        Ok(Work(dir))
    }

    /// The document's file: the same name on both surfaces.
    pub fn doc(&self) -> PathBuf {
        self.0.join("case.owd")
    }
}

impl Drop for Work {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), dest)?;
        }
    }
    Ok(())
}

fn path_in(work: &Work, rel: &Option<String>) -> Option<String> {
    rel.as_ref()
        .map(|r| work.0.join(r).to_string_lossy().into_owned())
}

/// The document the command line leaves after every step.
pub fn command_line(case: &Case) -> Result<Value, String> {
    let work = Work::new(case, "cli")?;
    let doc = work.doc().to_string_lossy().into_owned();
    let fail = |n: usize, f: commands::Failure| format!("step {n}: {}", f.message);
    for (i, step) in case.steps.iter().enumerate() {
        let n = i + 1;
        match step {
            Step::New { title, date } => {
                commands::new(&doc, title, Some(date), false).map_err(|f| fail(n, f))?
            }
            Step::Import {
                left_bank,
                right_bank,
                centreline,
                river_polygon,
                dem,
            } => commands::import(
                &doc,
                [
                    ("left_bank", path_in(&work, left_bank)),
                    ("right_bank", path_in(&work, right_bank)),
                    ("centreline", path_in(&work, centreline)),
                    ("river_polygon", path_in(&work, river_polygon)),
                ],
                path_in(&work, dem),
                WaterFlags {
                    level: None,
                    slope: None,
                    table: None,
                    holds_bathymetry: false,
                },
                &None,
                false,
            )
            .map_err(|f| fail(n, f))?,
            Step::Size {
                rule_sets,
                set_inputs,
                vessel,
                draught_m,
            } => commands::size(
                &doc,
                SizeFlags {
                    rule_sets: rule_sets.clone(),
                    set_inputs: set_inputs.iter().map(|(k, v)| format!("{k}={v}")).collect(),
                    files: Vec::new(),
                    vessel: Some(vessel.clone()),
                    draught: *draught_m,
                    custom: None,
                },
                &None,
                false,
            )
            .map_err(|f| fail(n, f))?,
            Step::Align { method } => commands::align(
                &doc,
                method,
                AlignFlags {
                    step: None,
                    passes: None,
                    chord: None,
                    dem: None,
                    water_level: None,
                    water_slope: None,
                    corridors: None,
                    fairness_weight: None,
                },
                &None,
                false,
            )
            .map_err(|f| fail(n, f))?,
        }
    }
    let text = std::fs::read_to_string(work.doc())
        .map_err(|e| format!("the command line wrote no document: {e}"))?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

/// The document the app shell leaves after every step.
pub fn shell(case: &Case) -> Result<Value, String> {
    let work = Work::new(case, "shell")?;
    let doc = work.doc();
    let mut state: Option<AppState> = None;
    // Provenance records are dated with the case's own date, not the clock, so a run repeats.
    let mut date = String::new();
    for (i, step) in case.steps.iter().enumerate() {
        let n = i + 1;
        let at = |e: String| format!("step {n}: {e}");
        if let Step::New { title, date: d } = step {
            // As the app: a new design, saved, then held with its path.
            let text = AppState::new_design(title, d).save_text_for(&doc);
            std::fs::write(&doc, &text).map_err(|e| at(e.to_string()))?;
            state = Some(AppState::open(&doc, &text, None).map_err(at)?);
            date = d.clone();
            continue;
        }
        let s = state
            .as_mut()
            .ok_or_else(|| at("the first step must be `new`".into()))?;
        match step {
            Step::New { .. } => unreachable!("handled above"),
            Step::Import {
                left_bank,
                right_bank,
                centreline,
                river_polygon,
                dem,
            } => {
                let mut items = Vec::new();
                for (role, rel) in [
                    (Role::LeftBank, left_bank),
                    (Role::RightBank, right_bank),
                    (Role::Centreline, centreline),
                    (Role::RiverPolygon, river_polygon),
                ] {
                    if let Some(rel) = rel {
                        let bytes = std::fs::read(work.0.join(rel))
                            .map_err(|e| at(format!("cannot read {rel}: {e}")))?;
                        items.push((
                            role,
                            owd_shell::input::vector_text(rel, bytes, None).map_err(at)?,
                        ));
                    }
                }
                if !items.is_empty() {
                    s.load_inputs(&items, &date).map_err(at)?;
                }
                if let Some(rel) = dem {
                    let file = work.0.join(rel);
                    let bytes =
                        std::fs::read(&file).map_err(|e| at(format!("cannot read {rel}: {e}")))?;
                    s.load_dem(&file, bytes, &date).map_err(at)?;
                }
            }
            Step::Size {
                rule_sets,
                set_inputs,
                vessel,
                draught_m,
            } => {
                let ids: Vec<&str> = rule_sets.iter().map(String::as_str).collect();
                let inputs: Vec<(&str, &str)> = set_inputs
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.as_str()))
                    .collect();
                s.set_rule_sets(&ids, &inputs).map_err(at)?;
                s.set_vessel(vessel, *draught_m).map_err(at)?;
            }
            Step::Align { method } => {
                let (job, _control) = s.begin_run("alignment", Some(method)).map_err(at)?;
                let outcome = job.execute(&|_| {});
                s.finish_run(outcome).map_err(at)?;
            }
        }
    }
    let mut s = state.ok_or("the case has no steps")?;
    serde_json::from_str(&s.save_text_for(&doc)).map_err(|e| e.to_string())
}
