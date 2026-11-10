//! Play a case's steps on the two surfaces: the command line's functions on a file, and the app
//! shell's `AppState`. Each surface works in its own temporary folder holding a copy of the case's
//! files, so the case folder is never written and a DEM path reads the same on both.

use crate::case::{Case, Step, Vessel};
use owd_cli::commands::{self, AlignFlags, CustomFlags, SizeFlags, VolumesFlags, WaterFlags};
use owd_format::design::{AlignMethod, Role};
use owd_shell::state::AppState;
use serde_json::{Value, json};
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

/// What one surface left: its document (or why a step could not run) and, for each `align` step
/// whose option fails a check, the failed checks read from the document straight after the step.
pub struct Played {
    pub doc: Result<Value, String>,
    pub failed: Vec<Value>,
}

/// The failed checks of the option of `method` in `doc`, as an entry for the report, if any.
fn failed_entry(doc: &Value, step: usize, method: &str) -> Option<Value> {
    let name = AlignMethod::from_name(method)?.name();
    let option = doc["options"]
        .as_array()?
        .iter()
        .find(|o| o["method"] == name)?;
    let checks = owd_format::design::export::failed_checks(option);
    (!checks.is_empty()).then(|| json!({"step": step, "method": name, "checks": checks}))
}

fn read_doc(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("no document: {e}"))?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

/// The command line's side of a case.
pub fn command_line(case: &Case) -> Played {
    let mut failed = Vec::new();
    let doc = command_line_steps(case, &mut failed);
    Played { doc, failed }
}

/// The document the command line leaves after every step.
fn command_line_steps(case: &Case, failed: &mut Vec<Value>) -> Result<Value, String> {
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
                    pools: None,
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
            } => {
                let (vessel, draught, custom) = match vessel {
                    Vessel::Class { id, draught_m } => (Some(id.clone()), *draught_m, None),
                    Vessel::Custom(c) => (
                        None,
                        Some(c.draught_m),
                        Some(CustomFlags {
                            name: c.name.clone(),
                            loa: Some(c.loa_m),
                            beam: Some(c.beam_m),
                            formation: Some(c.formation.clone()),
                            abreast: c.n_abreast,
                            in_line: c.n_in_line,
                            gb_class: c.gb_class.clone(),
                            air_draught: c.air_draught_m,
                            deadweight: c.deadweight_t,
                        }),
                    ),
                };
                commands::size(
                    &doc,
                    SizeFlags {
                        rule_sets: rule_sets.clone(),
                        set_inputs: set_inputs.iter().map(|(k, v)| format!("{k}={v}")).collect(),
                        files: Vec::new(),
                        vessel,
                        draught,
                        custom,
                    },
                    &None,
                    false,
                )
                .map_err(|f| fail(n, f))?
            }
            Step::Align { method } => {
                let done = commands::align(
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
                        use_water: None,
                    },
                    &None,
                    false,
                );
                done.map_err(|f| fail(n, f))?;
                // A saved option that fails a check exits 0 (ticket A13): its failed checks are
                // read from the document.
                failed.extend(
                    read_doc(&work.doc())
                        .ok()
                        .and_then(|d| failed_entry(&d, n, method)),
                );
            }
            Step::Water {
                level_m,
                fall_per_m,
            } => commands::import(
                &doc,
                [
                    ("left_bank", None),
                    ("right_bank", None),
                    ("centreline", None),
                    ("river_polygon", None),
                ],
                None,
                WaterFlags {
                    level: Some(*level_m),
                    slope: *fall_per_m,
                    table: None,
                    pools: None,
                    holds_bathymetry: false,
                },
                &None,
                false,
            )
            .map_err(|f| fail(n, f))?,
            Step::Volumes => {
                let done = commands::volumes(
                    &doc,
                    VolumesFlags {
                        dem: None,
                        model: None,
                        water_level: None,
                        water_slope: None,
                        station: None,
                        maxw: None,
                    },
                    &None,
                    false,
                );
                done.map_err(|f| fail(n, f))?;
            }
            Step::Run => {
                commands::run(&doc, &[], &None, false).map_err(|f| fail(n, f))?;
            }
        }
    }
    read_doc(&work.doc()).map_err(|e| format!("the command line wrote {e}"))
}

/// The app shell's side of a case.
pub fn shell(case: &Case) -> Played {
    let mut failed = Vec::new();
    let doc = shell_steps(case, &mut failed);
    Played { doc, failed }
}

/// The document the app shell leaves after every step.
fn shell_steps(case: &Case, failed: &mut Vec<Value>) -> Result<Value, String> {
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
            } => {
                let ids: Vec<&str> = rule_sets.iter().map(String::as_str).collect();
                let inputs: Vec<(&str, &str)> = set_inputs
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.as_str()))
                    .collect();
                s.set_rule_sets(&ids, &inputs).map_err(at)?;
                match vessel {
                    Vessel::Class { id, draught_m } => s.set_vessel(id, *draught_m).map_err(at)?,
                    Vessel::Custom(c) => s.set_custom_vessel(&c.fields).map_err(at)?,
                }
            }
            Step::Align { method } => {
                let (job, _control) = s.begin_run("alignment", Some(method)).map_err(at)?;
                let outcome = job.execute(&|_| {});
                s.finish_run(outcome).map_err(at)?;
                // Read the same way as on the command line: from the document, after the step.
                let now: Value =
                    serde_json::from_str(&s.save_text_for(&doc)).map_err(|e| at(e.to_string()))?;
                failed.extend(failed_entry(&now, n, method));
            }
            Step::Water {
                level_m,
                fall_per_m,
            } => s.set_water(*level_m, *fall_per_m).map_err(at)?,
            Step::Volumes => {
                let (job, _control) = s.begin_run("volumes", None).map_err(at)?;
                let outcome = job.execute(&|_| {});
                s.finish_run(outcome).map_err(at)?;
            }
            Step::Run => {
                s.run_stale().map_err(at)?;
            }
        }
    }
    let mut s = state.ok_or("the case has no steps")?;
    serde_json::from_str(&s.save_text_for(&doc)).map_err(|e| e.to_string())
}
