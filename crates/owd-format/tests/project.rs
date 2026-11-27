//! Label: property
//! The project folder's text file, `owd-project.json` (PROJ-1): it round-trips, keeps the keys it
//! does not know, refuses a path that is absolute or leaves the folder, and says why in plain words.
use owd_format::project::{
    Found, PROJECT_FILE, Project, check_path, dem_from_design, export_path, find, from_design,
};
use serde_json::{Value, json};

const STANDARD: &str = r#"{
  "owd_project": 1,
  "title": "Reach A screening",
  "design": "design/reach-a.owd",
  "dem": "terrain/reach-a-dem.tif",
  "inputs": {
    "centreline": "inputs/centreline.geojson",
    "river_polygon": "inputs/river-polygon.geojson",
    "left_bank": null,
    "right_bank": null,
    "water_pools": "inputs/pools.csv"
  },
  "notes": ""
}
"#;

fn refused(text: &str) -> String {
    Project::parse(text)
        .expect_err("the file is refused")
        .to_string()
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn the_standard_file_reads_and_writes_back_the_same_text() {
    let p = Project::parse(STANDARD).unwrap();
    assert_eq!(p.title(), Some("Reach A screening"));
    assert_eq!(p.design(), Some("design/reach-a.owd"));
    assert_eq!(p.dem(), Some("terrain/reach-a-dem.tif"));
    assert_eq!(p.input("centreline"), Some("inputs/centreline.geojson"));
    assert_eq!(p.input("left_bank"), None);
    assert_eq!(p.input("water_pools"), Some("inputs/pools.csv"));
    assert_eq!(p.notes(), Some(""));
    assert_eq!(p.to_text(), STANDARD);
    assert_eq!(Project::parse(&p.to_text()).unwrap(), p);
}

#[test]
fn every_key_but_owd_project_is_optional() {
    let p = Project::parse(r#"{"owd_project": 1}"#).unwrap();
    assert_eq!(p.title(), None);
    assert_eq!(p.design(), None);
    assert_eq!(p.dem(), None);
    assert!(p.items().iter().all(|i| i.path.is_none()));
    assert_eq!(
        p.items().iter().map(|i| i.key).collect::<Vec<_>>(),
        [
            "design",
            "dem",
            "centreline",
            "river_polygon",
            "left_bank",
            "right_bank",
            "water_pools",
            "buildings",
            "points_of_interest",
            "settlements"
        ]
    );
    // A file with a byte-order mark, as some editors on Windows write, still reads.
    assert!(Project::parse("\u{feff}{\"owd_project\": 1}").is_ok());
}

#[test]
fn unknown_keys_are_kept_on_rewrite_and_ignored() {
    let text = r#"{
  "owd_project": 1,
  "client": {"name": "A client", "ref": 7},
  "design": "design/a.owd",
  "inputs": {"centreline": null, "survey": "inputs/survey.csv"},
  "zz_later": [1, 2, 3]
}"#;
    let mut p = Project::parse(text).unwrap();
    p.set_dem(Some("terrain/a.tif")).unwrap();
    p.set_input("centreline", Some("inputs/c.geojson")).unwrap();
    p.set_title("A new title");
    let again: Value = serde_json::from_str(&p.to_text()).unwrap();
    assert_eq!(again["client"], json!({"name": "A client", "ref": 7}));
    assert_eq!(again["zz_later"], json!([1, 2, 3]));
    assert_eq!(again["inputs"]["survey"], "inputs/survey.csv");
    assert_eq!(again["inputs"]["centreline"], "inputs/c.geojson");
    assert_eq!(again["dem"], "terrain/a.tif");
    assert_eq!(again["title"], "A new title");
    // The keys that were there keep their order; a new key goes to the end.
    let keys: Vec<&String> = again.as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        [
            "owd_project",
            "client",
            "design",
            "inputs",
            "zz_later",
            "dem",
            "title"
        ]
    );
}

#[test]
fn an_absolute_path_is_refused_with_the_key_and_what_to_write() {
    for path in [
        "/data/dem.tif",
        "C:/data/dem.tif",
        "~/dem.tif",
        "file:///data/dem.tif",
    ] {
        let m = refused(&json!({"owd_project": 1, "dem": path}).to_string());
        assert!(m.contains("`dem`"), "{m}");
        assert!(m.contains(path), "{m}");
        assert!(m.contains("absolute path"), "{m}");
        assert!(m.contains("from the project folder"), "{m}");
    }
}

#[test]
fn a_path_that_leaves_the_folder_is_refused() {
    for path in ["../other/a.owd", "design/../../a.owd", "design/.."] {
        let m = refused(&json!({"owd_project": 1, "design": path}).to_string());
        assert!(m.contains("`design`"), "{m}");
        assert!(m.contains("may not leave the project folder"), "{m}");
    }
    let m = refused(r#"{"owd_project": 1, "inputs": {"water_pools": "../pools.csv"}}"#);
    assert!(m.contains("`inputs.water_pools`"), "{m}");
    assert!(m.contains("may not leave the project folder"), "{m}");
    // The setters check as the reader does.
    let mut p = Project::new("T");
    assert!(p.set_dem(Some("../x.tif")).is_err());
    assert!(p.set_design(Some("/x.owd")).is_err());
    assert!(p.set_input("left_bank", Some("../b.geojson")).is_err());
    assert!(
        p.set_input("no_such_input", Some("inputs/b.geojson"))
            .is_err()
    );
}

#[test]
fn the_other_refusals_are_in_plain_words() {
    let cases: [(&str, &str); 11] = [
        ("{ not json", "is not valid JSON (line 1, column"),
        ("[1]", "must be one JSON object"),
        (r#"{"title": "T"}"#, "must say `\"owd_project\": 1`"),
        (r#"{"owd_project": 2}"#, "use a newer OWD"),
        (r#"{"owd_project": "1"}"#, "must be the number 1"),
        (r#"{"owd_project": 1, "title": 7}"#, "`title` must be text"),
        (
            r#"{"owd_project": 1, "dem": "terrain\\a.tif"}"#,
            "forward slashes",
        ),
        (
            r#"{"owd_project": 1, "dem": "terrain/a.png"}"#,
            "must be a GeoTIFF",
        ),
        (
            r#"{"owd_project": 1, "design": "design/a.json"}"#,
            "must end in `.owd`",
        ),
        (r#"{"owd_project": 1, "design": ""}"#, "`design` is empty"),
        (
            r#"{"owd_project": 1, "inputs": []}"#,
            "`inputs` must be an object",
        ),
    ];
    for (text, want) in cases {
        let m = refused(text);
        assert!(m.contains(want), "`{text}` gave `{m}`, wanted `{want}`");
        assert!(!m.contains("serde"), "{m}");
    }
    assert!(check_path("dem", "terrain//a.tif").is_err());
    assert!(check_path("dem", "./terrain/a.tif").is_err());
    assert!(check_path("dem", "terrain/a.tif").is_ok());
}

#[test]
fn a_new_file_lists_every_key_of_the_standard() {
    let p = Project::new("Reach A screening");
    let v: Value = serde_json::from_str(&p.to_text()).unwrap();
    assert_eq!(
        v,
        json!({
            "owd_project": 1,
            "title": "Reach A screening",
            "design": null,
            "dem": null,
            "inputs": {
                "centreline": null, "river_polygon": null, "left_bank": null,
                "right_bank": null, "water_pools": null
            },
            "notes": ""
        })
    );
    assert_eq!(PROJECT_FILE, "owd-project.json");
}

#[test]
fn a_folder_with_no_text_file_is_searched_in_the_standard_places_then_the_top_level() {
    // The standard places win over the top level.
    let found = find(
        &names(&["reach-a.owd", "notes.txt"]),
        &names(&["reach-a-dem.TIF"]),
        &names(&["centreline.geojson", "pools.csv", "other.geojson"]),
        &names(&["old.owd", "old.tif"]),
    );
    assert_eq!(found.designs, ["design/reach-a.owd"]);
    assert_eq!(found.dems, ["terrain/reach-a-dem.TIF"]);
    assert_eq!(
        found.inputs,
        [
            ("centreline", "inputs/centreline.geojson".to_string()),
            ("water_pools", "inputs/pools.csv".to_string())
        ]
    );
    let p = found.only("Reach A").expect("one design and one DEM");
    assert_eq!(p.design(), Some("design/reach-a.owd"));
    assert_eq!(p.dem(), Some("terrain/reach-a-dem.TIF"));
    assert_eq!(p.input("centreline"), Some("inputs/centreline.geojson"));
    assert_eq!(p.input("river_polygon"), None);
    // With nothing in the standard places, the top level is read.
    let top = find(
        &[],
        &[],
        &[],
        &names(&["b.owd", "a.owd", "dem.tiff", "x.txt"]),
    );
    assert_eq!(top.designs, ["a.owd", "b.owd"]);
    assert_eq!(top.dems, ["dem.tiff"]);
    assert_eq!(top.only("T"), None, "two designs: the user picks");
    let picked = top.project("T", Some("b.owd"), Some("dem.tiff")).unwrap();
    assert_eq!(picked.design(), Some("b.owd"));
    // Nothing found.
    assert_eq!(find(&[], &[], &[], &names(&["x.txt"])), Found::default());
    assert_eq!(Found::default().only("T"), None);
}

#[test]
fn the_design_names_its_dem_from_its_own_folder() {
    assert_eq!(
        dem_from_design("design/reach-a.owd", "terrain/reach-a-dem.tif"),
        "../terrain/reach-a-dem.tif"
    );
    assert_eq!(dem_from_design("a.owd", "dem.tif"), "dem.tif");
    assert_eq!(
        dem_from_design("a.owd", "terrain/dem.tif"),
        "terrain/dem.tif"
    );
    assert_eq!(dem_from_design("design/a.owd", "design/dem.tif"), "dem.tif");
    assert_eq!(
        dem_from_design("design/v2/a.owd", "dem.tif"),
        "../../dem.tif"
    );
    // And back: where the path a design records is in the project folder.
    for (design, dem) in [
        ("design/reach-a.owd", "terrain/reach-a-dem.tif"),
        ("a.owd", "dem.tif"),
        ("design/v2/a.owd", "dem.tif"),
        ("design/a.owd", "design/dem.tif"),
    ] {
        assert_eq!(
            from_design(design, &dem_from_design(design, dem)).as_deref(),
            Some(dem)
        );
    }
    assert_eq!(
        from_design("design/a.owd", "../../dem.tif"),
        None,
        "leaves the folder"
    );
    assert_eq!(from_design("design/a.owd", "/data/dem.tif"), None);
}

#[test]
fn an_export_goes_into_exports_and_nowhere_else() {
    assert_eq!(
        export_path("reach-a.geojson").unwrap(),
        "exports/reach-a.geojson"
    );
    for bad in ["", "..", "../a.csv", "a/b.csv", "a\\b.csv"] {
        assert!(export_path(bad).is_err(), "{bad}");
    }
}
