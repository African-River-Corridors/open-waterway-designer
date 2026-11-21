//! The project folder (PROJ-1) as the app sees it: the text file `owd-project.json` read, and a
//! folder with no text file searched, both through the one model in `owd_format::project`. The
//! caller lists and reads the folder; nothing here touches a disk.

use owd_format::project::{self, Found, Project};
use serde_json::{Value, json};

fn shown(p: &Project) -> Value {
    json!({
        "title": p.title(),
        "design": p.design(),
        "dem": p.dem(),
        "notes": p.notes(),
        "items": p.items().iter().map(|i| json!({"key": i.key, "label": i.label, "path": i.path})).collect::<Vec<_>>(),
    })
}

/// The text of `owd-project.json`, read and checked: `{title, design, dem, notes, items: [{key,
/// label, path}]}`. A refusal is in plain words.
pub fn read(text: &str) -> Result<Value, String> {
    Project::parse(text)
        .map(|p| shown(&p))
        .map_err(|e| e.to_string())
}

fn names(listing: &Value, key: &str) -> Vec<String> {
    listing[key]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|n| n.as_str().map(str::to_string))
        .collect()
}

fn found_in(listing: &Value) -> Found {
    project::find(
        &names(listing, "design"),
        &names(listing, "terrain"),
        &names(listing, "inputs"),
        &names(listing, "top"),
    )
}

/// What a folder with no text file holds. `listing` is `{design: [names], terrain: [names],
/// inputs: [names], top: [names]}`, the files of the three standard folders and of the top level.
/// The answer: `{designs: [paths], dems: [paths], inputs: [{key, label, path}], clear}`; `clear`
/// is true when there is no choice to make (at most one design and one DEM, and at least one).
pub fn find(listing: &Value) -> Value {
    let found = found_in(listing);
    let label = |key: &str| {
        project::INPUTS
            .iter()
            .find(|(k, _, _)| *k == key)
            .map_or("", |(_, label, _)| *label)
    };
    json!({
        "designs": found.designs,
        "dems": found.dems,
        "inputs": found.inputs.iter().map(|(k, p)| json!({"key": k, "label": label(k), "path": p})).collect::<Vec<_>>(),
        "clear": found.only("").is_some(),
    })
}

/// The text of a new `owd-project.json` for a folder that has none: the chosen design and DEM
/// (each a path from the folder, or none) and every input found in `listing`.
pub fn adopt(
    listing: &Value,
    title: &str,
    design: Option<&str>,
    dem: Option<&str>,
) -> Result<String, String> {
    found_in(listing)
        .project(title, design, dem)
        .map(|p| p.to_text())
        .map_err(|e| e.to_string())
}

/// Where a file OWD writes out goes in the project folder: `exports/<name>`.
pub fn export_path(name: &str) -> Result<String, String> {
    project::export_path(name).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_is_read_searched_and_adopted_through_the_one_model() {
        let listing = json!({
            "design": ["reach-a.owd"], "terrain": ["reach-a-dem.tif"],
            "inputs": ["centreline.geojson"], "top": ["readme.txt"],
        });
        let f = find(&listing);
        assert_eq!(f["designs"], json!(["design/reach-a.owd"]));
        assert_eq!(f["dems"], json!(["terrain/reach-a-dem.tif"]));
        assert_eq!(f["inputs"][0]["label"], "Centreline");
        assert_eq!(f["clear"], true);
        let text = adopt(
            &listing,
            "Reach A",
            Some("design/reach-a.owd"),
            Some("terrain/reach-a-dem.tif"),
        )
        .unwrap();
        let p = read(&text).unwrap();
        assert_eq!(p["title"], "Reach A");
        assert_eq!(p["design"], "design/reach-a.owd");
        assert_eq!(
            p["items"][1],
            json!({"key": "dem", "label": "DEM", "path": "terrain/reach-a-dem.tif"})
        );
        assert_eq!(p["items"][2]["path"], "inputs/centreline.geojson");
        // Two designs: not clear. Nothing: not clear.
        assert_eq!(find(&json!({"top": ["a.owd", "b.owd"]}))["clear"], false);
        assert_eq!(find(&json!({}))["clear"], false);
        // A refusal is the model's plain words.
        assert!(
            read(r#"{"owd_project": 1, "dem": "/x.tif"}"#)
                .unwrap_err()
                .contains("absolute path")
        );
        assert!(adopt(&listing, "T", Some("../a.owd"), None).is_err());
        assert_eq!(export_path("a.csv").unwrap(), "exports/a.csv");
        assert!(export_path("../a.csv").is_err());
    }
}
