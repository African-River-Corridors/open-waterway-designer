//! `owd project`: the standard project folder (PROJ-1), and a project folder given where a
//! command takes a design file. The text file `owd-project.json` is read, checked and written by
//! `owd_format::project`, the one model the web app uses too. The folder and file access is here.

use crate::commands::{self, Failure, Result, input_error, say};
use owd_format::design::{NoTerrain, Session, stage_terrain};
use owd_format::project::{self, FOLDERS, PROJECT_FILE, Project};
use serde_json::{Value, json};
use std::path::Path;

fn slashes(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// The text file of a project folder, read and checked.
fn read_project(folder: &Path) -> core::result::Result<Project, String> {
    let file = folder.join(PROJECT_FILE);
    let text = std::fs::read_to_string(&file).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!(
                "`{}` is a folder with no {PROJECT_FILE}: run `owd project init {}` to make one, or give the design file itself",
                slashes(folder),
                slashes(folder)
            )
        } else {
            format!("cannot read {}: {e}", slashes(&file))
        }
    })?;
    Project::parse(&text).map_err(|e| format!("{}: {e}", slashes(&file)))
}

/// The design argument of a command: a design file as given, or, for a project folder, the design
/// file its `owd-project.json` names. Every command that takes a design file reads its argument
/// through this, so a project folder works in its place. The DEM is then found as for any design:
/// by the path the design records, from the design file's own folder.
pub fn design_arg(given: &str) -> core::result::Result<String, String> {
    let folder = Path::new(given);
    if given == "-" || !folder.is_dir() {
        return Ok(given.to_string());
    }
    let project = read_project(folder)?;
    let Some(design) = project.design() else {
        return Err(format!(
            "the project folder `{given}` names no design: add `\"design\": \"design/<name>.owd\"` to its {PROJECT_FILE}"
        ));
    };
    let file = folder.join(design);
    if !file.is_file() {
        return Err(format!(
            "the project folder `{given}` names the design `{design}`, but that file is not there"
        ));
    }
    Ok(file.to_string_lossy().into_owned())
}

/// The names of the files (not folders) in a folder; none when it is not there.
fn files_in(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with('.'))
        .collect();
    names.sort();
    names
}

/// What the folder holds in the standard places.
fn found(folder: &Path) -> project::Found {
    project::find(
        &files_in(&folder.join("design")),
        &files_in(&folder.join("terrain")),
        &files_in(&folder.join("inputs")),
        &files_in(folder),
    )
}

/// `owd project init`: make the folders and the text file. A text file that is there is never
/// overwritten. Files already in the standard places are named in the new text file when there is
/// no choice to make (one design, one DEM).
pub fn init(folder: &str, title: Option<&str>, json_mode: bool) -> Result {
    let dir = Path::new(folder);
    let file = dir.join(PROJECT_FILE);
    if file.exists() {
        return Err(input_error(format!(
            "{} is already there: `owd project init` does not overwrite it. See what it lists with `owd project status {folder}`",
            slashes(&file)
        )));
    }
    if dir.exists() && !dir.is_dir() {
        return Err(input_error(format!("`{folder}` is a file: give a folder")));
    }
    for sub in FOLDERS {
        std::fs::create_dir_all(dir.join(sub))
            .map_err(|e| input_error(format!("cannot make {}: {e}", slashes(&dir.join(sub)))))?;
    }
    let title = match title {
        Some(t) => t.to_string(),
        None => std::fs::canonicalize(dir)
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default(),
    };
    let found = found(dir);
    let mut notes: Vec<String> = Vec::new();
    let project = match found.only(&title) {
        Some(p) => p,
        None => {
            if found.designs.len() > 1 {
                notes.push(format!(
                    "{} design files are here ({}): name one as `design` in {PROJECT_FILE}",
                    found.designs.len(),
                    found.designs.join(", ")
                ));
            }
            if found.dems.len() > 1 {
                notes.push(format!(
                    "{} DEMs are here ({}): name one as `dem` in {PROJECT_FILE}",
                    found.dems.len(),
                    found.dems.join(", ")
                ));
            }
            let one = |list: &[String]| (list.len() == 1).then(|| list[0].clone());
            found
                .project(
                    &title,
                    one(&found.designs).as_deref(),
                    one(&found.dems).as_deref(),
                )
                .map_err(|e| input_error(e.to_string()))?
        }
    };
    // The design records its DEM from its own folder. When the design names a DEM that is not
    // where it says, and the project's DEM is that same file (by its hash), the design is pointed
    // at it; nothing else in the design changes.
    if let (Some(design), Some(dem)) = (project.design(), project.dem())
        && let Some(note) = point_design_at_dem(dir, design, dem)
    {
        notes.push(note);
    }
    std::fs::write(&file, project.to_text())
        .map_err(|e| input_error(format!("cannot write {}: {e}", slashes(&file))))?;
    let mut human = vec![format!(
        "made the project folder {folder}: {PROJECT_FILE}, {}",
        FOLDERS.map(|f| format!("{f}/")).join(", ")
    )];
    for item in project.items() {
        if let Some(p) = &item.path {
            human.push(format!("{}: {p}", item.label));
        }
    }
    if project.design().is_none() {
        human.push(format!(
            "no design yet: make one with `owd new {}/design/<name>.owd --title \"...\"`, then name it as `design` in {PROJECT_FILE}",
            slashes(dir)
        ));
    }
    if project.dem().is_none() {
        human.push(format!(
            "no DEM yet: put the GeoTIFF in {}/terrain/ and name it as `dem` in {PROJECT_FILE}",
            slashes(dir)
        ));
    }
    human.extend(notes.iter().map(|n| format!("note: {n}")));
    say(
        false,
        json_mode,
        &human.join("\n"),
        json!({"ok": true, "folder": folder, "file": slashes(&file), "project": project.to_value(), "notes": notes}),
    );
    Ok(())
}

/// Point a design at the project's DEM when the path it records leads nowhere and the project's
/// DEM is the file whose hash it recorded. Says what it did; `None` when nothing was to do.
fn point_design_at_dem(dir: &Path, design: &str, dem: &str) -> Option<String> {
    let design_file = dir.join(design);
    let text = std::fs::read_to_string(&design_file).ok()?;
    let mut session = Session::open(&text).ok()?;
    let recorded = session.dem_path()?.to_string();
    let want = project::dem_from_design(design, dem);
    if recorded == want
        || commands::doc_dir(&design_file.to_string_lossy())
            .join(&recorded)
            .is_file()
    {
        return None;
    }
    let bytes = std::fs::read(dir.join(dem)).ok()?;
    if stage_terrain(&session, Some(&bytes)).is_err() {
        return None;
    }
    session.set_dem_path(&want);
    std::fs::write(&design_file, session.to_text()).ok()?;
    Some(format!(
        "{design} named its DEM as `{recorded}`, which is not there; it is the project's DEM (the same hash), so the design now names it as `{want}`"
    ))
}

/// `owd project status`: what the text file names, whether each is there, and whether the DEM is
/// the one the design recorded. Exit code 1 when a named file is missing or the DEM differs.
pub fn status(folder: &str, json_mode: bool) -> Result {
    let dir = Path::new(folder);
    if !dir.is_dir() {
        return Err(input_error(format!("`{folder}` is not a folder")));
    }
    let project = read_project(dir).map_err(input_error)?;
    let mut human = vec![format!(
        "project: {}",
        project.title().unwrap_or("(no title)")
    )];
    let mut problems: Vec<String> = Vec::new();
    let mut items: Vec<Value> = Vec::new();
    for item in project.items() {
        let state = match &item.path {
            None => "not named",
            Some(p) if dir.join(p).is_file() => "there",
            Some(_) => "missing",
        };
        human.push(match &item.path {
            Some(p) => format!("{}: {p}: {state}", item.label),
            None => format!("{}: {state}", item.label),
        });
        if state == "missing" {
            problems.push(format!(
                "{} `{}` is missing",
                item.label,
                item.path.as_deref().unwrap_or("?")
            ));
        }
        items
            .push(json!({"key": item.key, "label": item.label, "path": item.path, "state": state}));
    }
    // The DEM against the design: the hash the design recorded, and the path it records.
    let mut dem_check = Value::Null;
    if let Some(design) = project.design()
        && let Ok(text) = std::fs::read_to_string(dir.join(design))
    {
        match Session::open(&text) {
            Err(e) => problems.push(format!("the design `{design}` does not open: {e}")),
            Ok(session) => {
                let dem_bytes = project.dem().and_then(|d| std::fs::read(dir.join(d)).ok());
                let recorded = session.dem_path().map(str::to_string);
                // The terrain rule's one reader compares the file with the recorded hash.
                let check = stage_terrain(&session, dem_bytes.as_deref()).map(|_| ());
                let (word, line) = match (project.dem(), check) {
                    (None, Err(NoTerrain::NotNamed)) => ("no_dem", "DEM hash: the design records no DEM".to_string()),
                    (None, _) => (
                        "not_named",
                        format!(
                            "DEM hash: the design records a DEM (`{}`) but the project names none",
                            recorded.as_deref().unwrap_or("?")
                        ),
                    ),
                    (Some(_), Err(NoTerrain::NotNamed)) => (
                        "not_recorded",
                        "DEM hash: the design records no DEM yet: load the project's DEM (`owd import <folder> --dem FILE`, or Open project in the app and Save)".to_string(),
                    ),
                    (Some(_), Ok(())) => (
                        "matches",
                        "DEM hash: matches the hash the design recorded".to_string(),
                    ),
                    (Some(d), Err(NoTerrain::Changed(_))) => {
                        problems.push(format!(
                            "the DEM `{d}` is not the file the design recorded (the hash differs): the volumes may no longer hold"
                        ));
                        ("changed", format!("DEM hash: DIFFERS from the hash the design recorded for `{}`", recorded.as_deref().unwrap_or("?")))
                    }
                    (Some(_), Err(NoTerrain::Missing(_))) => ("missing", "DEM hash: not checked, the DEM is missing".to_string()),
                };
                human.push(line);
                // Where the design says its DEM is, from its own folder: the commands look there.
                let mut path_ok = Value::Null;
                if let (Some(d), Some(r)) = (project.dem(), recorded.as_deref()) {
                    let want = project::dem_from_design(design, d);
                    let same = project::from_design(design, r).as_deref() == Some(d);
                    path_ok = json!(same);
                    if !same {
                        let line = format!(
                            "the design records its DEM as `{r}`, not the project's `{d}` (`{want}` from the design): the commands look where the design says. Open the project in the app and Save, or run `owd import {folder} --dem {}`",
                            slashes(&dir.join(d))
                        );
                        problems.push(line);
                    }
                }
                dem_check =
                    json!({"hash": word, "recorded_path": recorded, "path_matches": path_ok});
            }
        }
    }
    human.extend(problems.iter().map(|p| format!("problem: {p}")));
    say(
        false,
        json_mode,
        &human.join("\n"),
        json!({
            "title": project.title(),
            "items": items,
            "dem": dem_check,
            "problems": problems,
        }),
    );
    if problems.is_empty() {
        Ok(())
    } else {
        Err(Failure {
            message: format!(
                "{} problem(s) in the project folder {folder}",
                problems.len()
            ),
            code: 1,
        })
    }
}
