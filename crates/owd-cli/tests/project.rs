//! `owd project init`, `owd project status`, and a project folder given where a command takes a
//! design file (PROJ-1). The data is the synthetic web example.
//!
//! Label: property

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn owd(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_owd"))
        .args(args)
        .output()
        .expect("owd runs")
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn code(o: &Output) -> i32 {
    o.status.code().expect("exited normally")
}

fn work_dir(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("owd-project-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn example(file: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/web")
        .join(file)
}

fn json(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// A folder with the example design in `design/` and its DEM in `terrain/`, and no text file.
fn folder_with_example(name: &str) -> PathBuf {
    let d = work_dir(name);
    std::fs::create_dir_all(d.join("design")).unwrap();
    std::fs::create_dir_all(d.join("terrain")).unwrap();
    std::fs::copy(example("river-example.owd"), d.join("design/reach-a.owd")).unwrap();
    std::fs::copy(example("river-dem.tif"), d.join("terrain/reach-a-dem.tif")).unwrap();
    d
}

#[test]
fn init_makes_the_folders_and_the_text_file_and_never_overwrites_one() {
    let d = work_dir("init").join("reach-a");
    let folder = d.to_str().unwrap();
    let o = owd(&["project", "init", folder, "--title", "Reach A screening"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    for sub in ["design", "terrain", "inputs", "exports"] {
        assert!(d.join(sub).is_dir(), "{sub}/ is made");
    }
    let file = d.join("owd-project.json");
    let p = json(&file);
    assert_eq!(p["owd_project"], 1);
    assert_eq!(p["title"], "Reach A screening");
    assert_eq!(p["design"], Value::Null);
    assert_eq!(p["dem"], Value::Null);
    assert_eq!(p["inputs"]["water_pools"], Value::Null);
    assert!(out(&o).contains("no design yet"), "{}", out(&o));

    // A second init is refused, and the file is as it was.
    std::fs::write(&file, "{\"owd_project\": 1, \"title\": \"mine\"}\n").unwrap();
    let again = owd(&["project", "init", folder, "--title", "Other"]);
    assert_eq!(code(&again), 2);
    assert!(
        err(&again).contains("does not overwrite"),
        "{}",
        err(&again)
    );
    assert_eq!(json(&file)["title"], "mine");

    // With no --title, the folder's name is the title.
    let e = work_dir("init-named").join("lower-reach");
    let o = owd(&["project", "init", e.to_str().unwrap(), "--json"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert_eq!(json(&e.join("owd-project.json"))["title"], "lower-reach");
    let said: Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(said["ok"], true);
}

#[test]
fn init_names_what_is_in_the_standard_places_and_leaves_the_dem_path_right() {
    let d = folder_with_example("adopt");
    let folder = d.to_str().unwrap();
    let before = json(&d.join("design/reach-a.owd"));
    assert_eq!(before["inputs"]["dem"]["path"], "river-dem.tif");

    let o = owd(&["project", "init", folder]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    let p = json(&d.join("owd-project.json"));
    assert_eq!(p["design"], "design/reach-a.owd");
    assert_eq!(p["dem"], "terrain/reach-a-dem.tif");
    // The design names its DEM from its own folder; nothing else in it changed.
    let mut after = json(&d.join("design/reach-a.owd"));
    assert_eq!(after["inputs"]["dem"]["path"], "../terrain/reach-a-dem.tif");
    assert!(
        out(&o).contains("../terrain/reach-a-dem.tif"),
        "{}",
        out(&o)
    );
    after["inputs"]["dem"]["path"] = before["inputs"]["dem"]["path"].clone();
    assert_eq!(after, before);
    // No folder path is written into the design.
    let text = std::fs::read_to_string(d.join("design/reach-a.owd")).unwrap();
    assert!(!text.contains(folder));

    // Two designs: init names neither and says so.
    let two = folder_with_example("adopt-two");
    std::fs::copy(example("river-example.owd"), two.join("design/reach-b.owd")).unwrap();
    let o = owd(&["project", "init", two.to_str().unwrap()]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    let p = json(&two.join("owd-project.json"));
    assert_eq!(p["design"], Value::Null);
    assert_eq!(p["dem"], "terrain/reach-a-dem.tif");
    assert!(out(&o).contains("2 design files are here"), "{}", out(&o));
}

#[test]
fn status_lists_what_is_named_and_checks_the_dem_hash_against_the_design() {
    let d = folder_with_example("status");
    let folder = d.to_str().unwrap();
    assert_eq!(code(&owd(&["project", "init", folder])), 0);
    std::fs::write(d.join("inputs/pools.csv"), "chainage_m,level_m\n0,2\n").unwrap();
    let file = d.join("owd-project.json");
    let mut p = json(&file);
    p["inputs"]["water_pools"] = "inputs/pools.csv".into();
    p["inputs"]["centreline"] = "inputs/centreline.geojson".into();
    std::fs::write(&file, p.to_string()).unwrap();

    // The centreline is named but not there: exit code 1, and each item has its state.
    let o = owd(&["project", "status", folder]);
    assert_eq!(code(&o), 1, "{}", out(&o));
    let said = out(&o);
    assert!(said.contains("Design: design/reach-a.owd: there"), "{said}");
    assert!(
        said.contains("DEM: terrain/reach-a-dem.tif: there"),
        "{said}"
    );
    assert!(
        said.contains("Centreline: inputs/centreline.geojson: missing"),
        "{said}"
    );
    assert!(
        said.contains("Water levels by pool: inputs/pools.csv: there"),
        "{said}"
    );
    assert!(said.contains("Left bank: not named"), "{said}");
    assert!(
        said.contains("DEM hash: matches the hash the design recorded"),
        "{said}"
    );

    // All there: exit code 0.
    p["inputs"]["centreline"] = Value::Null;
    std::fs::write(&file, p.to_string()).unwrap();
    let o = owd(&["project", "status", folder, "--json"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    let v: Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(v["dem"]["hash"], "matches");
    assert_eq!(v["dem"]["path_matches"], true);
    assert_eq!(v["items"][1]["state"], "there");
    assert_eq!(v["problems"].as_array().unwrap().len(), 0);

    // `--full` hashes every byte as well (COG-1): the design recorded a full hash, and it matches.
    let o = owd(&["project", "status", folder, "--json", "--full"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    let v: Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(v["dem"]["method"], "full");
    assert_eq!(v["dem"]["full"], "matches");
    assert_eq!(
        v["dem"]["full_sha256"],
        json(&d.join("design/reach-a.owd"))["inputs"]["dem"]["sha256"]
    );

    // Another DEM under the same name: the hash differs, exit code 1.
    let dem = d.join("terrain/reach-a-dem.tif");
    let mut bytes = std::fs::read(&dem).unwrap();
    let n = bytes.len();
    bytes[n - 1] ^= 0xff;
    std::fs::write(&dem, &bytes).unwrap();
    let o = owd(&["project", "status", folder]);
    assert_eq!(code(&o), 1);
    assert!(out(&o).contains("DEM hash: DIFFERS"), "{}", out(&o));

    // The DEM gone: missing, exit code 1.
    std::fs::remove_file(&dem).unwrap();
    let o = owd(&["project", "status", folder, "--json"]);
    assert_eq!(code(&o), 1);
    let v: Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(v["items"][1]["state"], "missing");
    assert_eq!(v["dem"]["hash"], "missing");
}

#[test]
fn status_says_when_the_design_looks_for_its_dem_somewhere_else() {
    // The DEM is beside the design as well: the design finds that one, so init leaves the design
    // alone and status says the design does not look at the project's DEM.
    let d = folder_with_example("elsewhere");
    std::fs::copy(example("river-dem.tif"), d.join("design/river-dem.tif")).unwrap();
    let folder = d.to_str().unwrap();
    assert_eq!(code(&owd(&["project", "init", folder])), 0);
    assert_eq!(
        json(&d.join("design/reach-a.owd"))["inputs"]["dem"]["path"],
        "river-dem.tif"
    );
    let o = owd(&["project", "status", folder]);
    assert_eq!(code(&o), 1);
    assert!(
        out(&o).contains("the design records its DEM as `river-dem.tif`, not the project's `terrain/reach-a-dem.tif`"),
        "{}",
        out(&o)
    );
}

#[test]
fn a_command_given_the_project_folder_reads_the_design_it_names_and_finds_the_dem() {
    let d = folder_with_example("command");
    let folder = d.to_str().unwrap();
    assert_eq!(code(&owd(&["project", "init", folder])), 0);
    let design = d.join("design/reach-a.owd");

    // `owd status`: the same answer for the folder as for the design file, and no DEM warning.
    let by_folder = owd(&["status", folder, "--json"]);
    let by_file = owd(&["status", design.to_str().unwrap(), "--json"]);
    assert_eq!(code(&by_folder), 0, "{}", err(&by_folder));
    assert_eq!(out(&by_folder), out(&by_file));
    let v: Value = serde_json::from_str(&out(&by_folder)).unwrap();
    assert_eq!(
        v["warnings"].as_array().unwrap().len(),
        0,
        "the DEM is found"
    );
    assert_eq!(v["stages"]["volumes"], "fresh");
    assert_eq!(code(&owd(&["validate", folder])), 0);

    // A command that writes: the design in the folder is the file written.
    let o = owd(&["size", folder, "--draught", "2.6"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    let v: Value = serde_json::from_str(&out(&owd(&["status", folder, "--json"]))).unwrap();
    assert_eq!(v["stages"]["alignment"], "stale");
    // An export reads the folder too.
    let o = owd(&["export", folder, "--csv"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(!out(&o).is_empty());
}

#[test]
fn a_folder_that_is_not_a_project_is_refused_in_plain_words() {
    let d = work_dir("refused");
    let folder = d.to_str().unwrap();
    let o = owd(&["status", folder]);
    assert_eq!(code(&o), 2);
    assert!(
        err(&o).contains("is a folder with no owd-project.json"),
        "{}",
        err(&o)
    );
    assert!(err(&o).contains("owd project init"), "{}", err(&o));

    std::fs::write(
        d.join("owd-project.json"),
        r#"{"owd_project": 1, "design": "../a.owd"}"#,
    )
    .unwrap();
    for args in [vec!["status", folder], vec!["project", "status", folder]] {
        let o = owd(&args);
        assert_eq!(code(&o), 2, "{args:?}");
        assert!(
            err(&o).contains("may not leave the project folder"),
            "{}",
            err(&o)
        );
    }

    std::fs::write(
        d.join("owd-project.json"),
        r#"{"owd_project": 1, "design": "design/a.owd"}"#,
    )
    .unwrap();
    let o = owd(&["status", folder]);
    assert_eq!(code(&o), 2);
    assert!(
        err(&o).contains("but that file is not there"),
        "{}",
        err(&o)
    );
    // `owd project status` still lists it, as missing.
    let o = owd(&["project", "status", folder]);
    assert_eq!(code(&o), 1);
    assert!(
        out(&o).contains("Design: design/a.owd: missing"),
        "{}",
        out(&o)
    );
}
