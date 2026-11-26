//! The standard project folder and its small text file, `owd-project.json`.
//!
//! ```text
//! <project folder>/
//!   owd-project.json        the small text file that says what is here
//!   design/<name>.owd       the design file(s)
//!   terrain/<name>.tif      the DEM (GeoTIFF)
//!   inputs/                 source data: centreline, river polygon, bank lines, water levels by pool
//!   exports/                what OWD writes out: GeoJSON, CSV, sheets, reports
//! ```
//!
//! This is the one place that reads, checks and writes the text file. The web app and the CLI both
//! use it. Nothing here touches a disk: the callers list and read the folder.
//!
//! Every path in the file is relative to the folder, with forward slashes. An absolute path, and a
//! path that leaves the folder (`..`), are refused. Every key but `owd_project` is optional. A key
//! this version does not know is kept when the file is written again, and is otherwise ignored.

use serde_json::{Map, Value, json};

/// The name of the text file, at the top of the project folder.
pub const PROJECT_FILE: &str = "owd-project.json";
/// The version of the text file this OWD reads and writes.
pub const PROJECT_VERSION: u64 = 1;
/// The folders of the standard, in the order `owd project init` makes them.
pub const FOLDERS: [&str; 4] = ["design", "terrain", "inputs", "exports"];
/// Where OWD writes its exports, inside the project folder.
pub const EXPORTS: &str = "exports";

/// The source data a project may list under `inputs`, with the words the app and the CLI show and
/// the standard place each is looked for when a folder has no text file yet.
///
/// The last three (BLD-1) are optional files a design names by reference; a new project file
/// writes only the first [`CORE_INPUTS`], so its text is as before.
pub const INPUTS: [(&str, &str, &str); 8] = [
    ("centreline", "Centreline", "inputs/centreline.geojson"),
    (
        "river_polygon",
        "River polygon",
        "inputs/river-polygon.geojson",
    ),
    ("left_bank", "Left bank", "inputs/left-bank.geojson"),
    ("right_bank", "Right bank", "inputs/right-bank.geojson"),
    ("water_pools", "Water levels by pool", "inputs/pools.csv"),
    ("buildings", "Buildings", "inputs/buildings.geojson"),
    (
        "points_of_interest",
        "Points of interest",
        "inputs/points-of-interest.geojson",
    ),
    ("settlements", "Settlements", "inputs/settlements.geojson"),
];

/// The inputs a new project file lists (as null): the first five of [`INPUTS`].
pub const CORE_INPUTS: usize = 5;

/// Why a project file was refused, in plain words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectError(pub String);

impl std::fmt::Display for ProjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ProjectError {}

fn refuse<T>(message: String) -> Result<T, ProjectError> {
    Err(ProjectError(message))
}

/// Check one path of the text file. `key` names it in the message (`dem`, `inputs.centreline`).
pub fn check_path(key: &str, path: &str) -> Result<(), ProjectError> {
    let example = "such as `terrain/reach-a-dem.tif`";
    if path.trim().is_empty() {
        return refuse(format!(
            "`{key}` is empty: give a path inside the project folder, or null for none"
        ));
    }
    if path.contains('\\') {
        return refuse(format!(
            "`{key}` is `{path}`: write the path with forward slashes (`/`), {example}"
        ));
    }
    let bytes = path.as_bytes();
    let drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    if path.starts_with('/') || path.starts_with('~') || drive || path.contains("://") {
        return refuse(format!(
            "`{key}` is `{path}`, an absolute path: give the path from the project folder, {example}"
        ));
    }
    if path.split('/').any(|part| part == "..") {
        return refuse(format!(
            "`{key}` is `{path}`: a path may not leave the project folder (`..`)"
        ));
    }
    if path.split('/').any(|part| part.is_empty() || part == ".") {
        return refuse(format!(
            "`{key}` is `{path}`: write it as plain folder and file names with one `/` between them, {example}"
        ));
    }
    Ok(())
}

fn has_ext(path: &str, exts: &[&str]) -> bool {
    let lower = path.to_ascii_lowercase();
    exts.iter().any(|e| lower.ends_with(e))
}

/// Whether a file name is a design file (`.owd`).
pub fn is_design(name: &str) -> bool {
    has_ext(name, &[".owd"])
}

/// Whether a file name is a DEM (`.tif` or `.tiff`).
pub fn is_dem(name: &str) -> bool {
    has_ext(name, &[".tif", ".tiff"])
}

/// The text file of one project folder: what the folder holds, by path.
#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    /// The file as read, keys in their order, with the keys this version does not know.
    raw: Map<String, Value>,
}

/// A text key of the file: absent and null are both none.
fn text_of(
    map: &Map<String, Value>,
    key: &str,
    shown: &str,
) -> Result<Option<String>, ProjectError> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(other) => refuse(format!(
            "`{shown}` must be text in quotes, or null for none, not `{other}`"
        )),
    }
}

impl Project {
    /// A new project file with a title and nothing listed.
    pub fn new(title: &str) -> Project {
        let mut inputs = Map::new();
        for (key, _, _) in INPUTS.iter().take(CORE_INPUTS) {
            inputs.insert(key.to_string(), Value::Null);
        }
        let mut raw = Map::new();
        raw.insert("owd_project".into(), json!(PROJECT_VERSION));
        raw.insert("title".into(), json!(title));
        raw.insert("design".into(), Value::Null);
        raw.insert("dem".into(), Value::Null);
        raw.insert("inputs".into(), Value::Object(inputs));
        raw.insert("notes".into(), json!(""));
        Project { raw }
    }

    /// Read and check the text of `owd-project.json`.
    pub fn parse(text: &str) -> Result<Project, ProjectError> {
        let value: Value = match serde_json::from_str(text.trim_start_matches('\u{feff}')) {
            Ok(v) => v,
            Err(e) => {
                return refuse(format!(
                    "{PROJECT_FILE} is not valid JSON (line {}, column {}): check the commas, quotes and brackets there",
                    e.line(),
                    e.column()
                ));
            }
        };
        let Value::Object(raw) = value else {
            return refuse(format!(
                "{PROJECT_FILE} must be one JSON object: `{{ \"owd_project\": 1, ... }}`"
            ));
        };
        match raw.get("owd_project") {
            None => {
                return refuse(format!(
                    "{PROJECT_FILE} must say `\"owd_project\": 1`: without it OWD cannot tell this is a project file"
                ));
            }
            Some(v) => match v.as_u64() {
                Some(PROJECT_VERSION) => {}
                Some(n) if n > PROJECT_VERSION => {
                    return refuse(format!(
                        "this project file is version {n}, and this OWD reads version {PROJECT_VERSION}: use a newer OWD"
                    ));
                }
                _ => {
                    return refuse(format!(
                        "`owd_project` must be the number {PROJECT_VERSION}, not `{v}`"
                    ));
                }
            },
        }
        text_of(&raw, "title", "title")?;
        text_of(&raw, "notes", "notes")?;
        if let Some(design) = text_of(&raw, "design", "design")? {
            check_path("design", &design)?;
            if !is_design(&design) {
                return refuse(format!(
                    "`design` is `{design}`: the design file must end in `.owd`"
                ));
            }
        }
        if let Some(dem) = text_of(&raw, "dem", "dem")? {
            check_path("dem", &dem)?;
            if !is_dem(&dem) {
                return refuse(format!(
                    "`dem` is `{dem}`: the DEM must be a GeoTIFF, ending in `.tif` or `.tiff`"
                ));
            }
        }
        match raw.get("inputs") {
            None | Some(Value::Null) => {}
            Some(Value::Object(inputs)) => {
                for (key, _, _) in INPUTS {
                    let shown = format!("inputs.{key}");
                    if let Some(path) = text_of(inputs, key, &shown)? {
                        check_path(&shown, &path)?;
                    }
                }
            }
            Some(other) => {
                return refuse(format!(
                    "`inputs` must be an object, `{{ \"centreline\": \"inputs/centreline.geojson\", ... }}`, not `{other}`"
                ));
            }
        }
        Ok(Project { raw })
    }

    /// The text to write as `owd-project.json`: every key that was read, in its order, and the
    /// keys this version does not know, unchanged.
    pub fn to_text(&self) -> String {
        let mut text = serde_json::to_string_pretty(&Value::Object(self.raw.clone()))
            .expect("a JSON object always writes");
        text.push('\n');
        text
    }

    /// The file as a JSON value, for a caller that shows it.
    pub fn to_value(&self) -> Value {
        Value::Object(self.raw.clone())
    }

    fn text(&self, key: &str) -> Option<&str> {
        self.raw.get(key).and_then(Value::as_str)
    }

    /// The project's title, if it has one that is not empty.
    pub fn title(&self) -> Option<&str> {
        self.text("title").filter(|t| !t.trim().is_empty())
    }

    /// The design file, from the project folder.
    pub fn design(&self) -> Option<&str> {
        self.text("design")
    }

    /// The DEM, from the project folder.
    pub fn dem(&self) -> Option<&str> {
        self.text("dem")
    }

    /// The notes, as typed.
    pub fn notes(&self) -> Option<&str> {
        self.text("notes")
    }

    /// One of the listed inputs ([`INPUTS`]), from the project folder.
    pub fn input(&self, key: &str) -> Option<&str> {
        self.raw.get("inputs")?.get(key)?.as_str()
    }

    /// Set the title.
    pub fn set_title(&mut self, title: &str) {
        self.raw.insert("title".into(), json!(title));
    }

    /// Name the design file (`None`: none). The path is checked as [`Project::parse`] checks it.
    pub fn set_design(&mut self, path: Option<&str>) -> Result<(), ProjectError> {
        if let Some(p) = path {
            check_path("design", p)?;
            if !is_design(p) {
                return refuse(format!(
                    "`design` is `{p}`: the design file must end in `.owd`"
                ));
            }
        }
        self.raw.insert("design".into(), json!(path));
        Ok(())
    }

    /// Name the DEM (`None`: none). The path is checked as [`Project::parse`] checks it.
    pub fn set_dem(&mut self, path: Option<&str>) -> Result<(), ProjectError> {
        if let Some(p) = path {
            check_path("dem", p)?;
            if !is_dem(p) {
                return refuse(format!(
                    "`dem` is `{p}`: the DEM must be a GeoTIFF, ending in `.tif` or `.tiff`"
                ));
            }
        }
        self.raw.insert("dem".into(), json!(path));
        Ok(())
    }

    /// Name one of the listed inputs ([`INPUTS`]); `None`: none.
    pub fn set_input(&mut self, key: &str, path: Option<&str>) -> Result<(), ProjectError> {
        if !INPUTS.iter().any(|(k, _, _)| *k == key) {
            return refuse(format!("`{key}` is not an input a project lists"));
        }
        if let Some(p) = path {
            check_path(&format!("inputs.{key}"), p)?;
        }
        let inputs = self
            .raw
            .entry("inputs")
            .or_insert_with(|| Value::Object(Map::new()));
        if !inputs.is_object() {
            *inputs = Value::Object(Map::new());
        }
        inputs[key] = json!(path);
        Ok(())
    }

    /// Every item the file may name, in the order the app and the CLI list them: the key, the
    /// words to show and the path (`None`: not named).
    pub fn items(&self) -> Vec<Item> {
        let mut items = vec![
            Item::new("design", "Design", self.design()),
            Item::new("dem", "DEM", self.dem()),
        ];
        for (key, label, _) in INPUTS {
            items.push(Item::new(key, label, self.input(key)));
        }
        items
    }
}

/// One item a project file may name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// `design`, `dem`, or the key of one of [`INPUTS`].
    pub key: &'static str,
    /// The words the app and the CLI show.
    pub label: &'static str,
    /// The path from the project folder; `None`: the file does not name it.
    pub path: Option<String>,
}

impl Item {
    fn new(key: &'static str, label: &'static str, path: Option<&str>) -> Item {
        Item {
            key,
            label,
            path: path.map(str::to_string),
        }
    }
}

/// What a folder with no text file holds in the standard places.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Found {
    /// Design files: `design/*.owd`, or the top level's when `design/` has none.
    pub designs: Vec<String>,
    /// DEMs: `terrain/*.tif|tiff`, or the top level's when `terrain/` has none.
    pub dems: Vec<String>,
    /// The listed inputs found under their standard names, as `(key, path)`.
    pub inputs: Vec<(&'static str, String)>,
}

impl Found {
    /// A project file for this folder when there is no choice to make: at most one design and at
    /// most one DEM, and at least one of them. `None`: the user must pick, or nothing was found.
    pub fn only(&self, title: &str) -> Option<Project> {
        if self.designs.len() > 1 || self.dems.len() > 1 {
            return None;
        }
        if self.designs.is_empty() && self.dems.is_empty() {
            return None;
        }
        self.project(
            title,
            self.designs.first().map(String::as_str),
            self.dems.first().map(String::as_str),
        )
        .ok()
    }

    /// A project file that names the chosen design and DEM, and every input found.
    pub fn project(
        &self,
        title: &str,
        design: Option<&str>,
        dem: Option<&str>,
    ) -> Result<Project, ProjectError> {
        let mut p = Project::new(title);
        p.set_design(design)?;
        p.set_dem(dem)?;
        for (key, path) in &self.inputs {
            p.set_input(key, Some(path))?;
        }
        Ok(p)
    }
}

/// Look in the standard places. The caller lists three folders by file name (no folders in the
/// lists): `design/`, `terrain/`, `inputs/` and the top level of the project folder.
pub fn find(
    design_dir: &[String],
    terrain_dir: &[String],
    inputs_dir: &[String],
    top: &[String],
) -> Found {
    let pick = |standard: &[String], folder: &str, is: fn(&str) -> bool| -> Vec<String> {
        let mut found: Vec<String> = standard
            .iter()
            .filter(|n| is(n))
            .map(|n| format!("{folder}/{n}"))
            .collect();
        if found.is_empty() {
            found = top.iter().filter(|n| is(n)).cloned().collect();
        }
        found.sort();
        found
    };
    let inputs = INPUTS
        .iter()
        .filter_map(|(key, _, standard)| {
            let name = standard.strip_prefix("inputs/")?;
            inputs_dir
                .iter()
                .any(|n| n == name)
                .then(|| (*key, standard.to_string()))
        })
        .collect();
    Found {
        designs: pick(design_dir, "design", is_design),
        dems: pick(terrain_dir, "terrain", is_dem),
        inputs,
    }
}

/// How a design names a DEM of its project: the path from the design file's own folder, as the
/// design file records it. `design/a.owd` and `terrain/d.tif` give `../terrain/d.tif`. Both paths
/// are from the project folder.
pub fn dem_from_design(design: &str, dem: &str) -> String {
    let base: Vec<&str> = design.split('/').collect();
    let base = &base[..base.len().saturating_sub(1)];
    let target: Vec<&str> = dem.split('/').collect();
    let common = base
        .iter()
        .zip(&target)
        .take_while(|(a, b)| a == b)
        .count()
        .min(target.len().saturating_sub(1));
    let mut parts: Vec<&str> = vec![".."; base.len() - common];
    parts.extend(&target[common..]);
    parts.join("/")
}

/// Where a path the design records (from the design file's own folder) is in the project folder.
/// `None`: it is absolute, or it leaves the project folder.
pub fn from_design(design: &str, recorded: &str) -> Option<String> {
    if recorded.starts_with('/') || recorded.contains('\\') || recorded.contains(':') {
        return None;
    }
    let mut parts: Vec<&str> = design.split('/').collect();
    parts.pop();
    for part in recorded.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            name => parts.push(name),
        }
    }
    Some(parts.join("/"))
}

/// A name for a file OWD writes into `exports/`: the path from the project folder. A name with a
/// folder in it is refused, so nothing is written outside `exports/`.
pub fn export_path(name: &str) -> Result<String, ProjectError> {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name == "." || name == ".." {
        return refuse(format!(
            "`{name}` is not a file name OWD can write into `{EXPORTS}/`"
        ));
    }
    Ok(format!("{EXPORTS}/{name}"))
}
