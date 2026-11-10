//! The user can say which way the river flows (ticket B2b): `owd import --downstream LON,LAT` and
//! the shell's downstream point both go through the one import, which orients the banks towards
//! the point and names them looking that way. Banks that fail the side check wholesale get one
//! plain message, not a line per station.
//!
//! The river is built in the test: straight, 3.2 km long, 200 m wide, in UTM 30N, drawn west to
//! east. The file named `left` holds the south bank, the file named `right` the north bank: right
//! for a river that flows west, wrong for one that flows east.
//!
//! Label: property

use owd_format::design::Role;
use owd_format::design::crs::Utm;
use owd_shell::state::AppState;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const UTM30: &str = r#""crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
const ZONE: Utm = Utm {
    zone: 30,
    south: false,
};

fn bank(y: f64) -> Vec<[f64; 2]> {
    (0..=32)
        .map(|i| [500_000.0 + 100.0 * f64::from(i), 600_000.0 + y])
        .collect()
}

fn text(pts: &[[f64; 2]]) -> String {
    format!(
        r#"{{"type":"LineString",{UTM30},"coordinates":{}}}"#,
        json!(pts)
    )
}

/// The point at `x` on the river's middle, as `lon,lat`.
fn lon_lat_at(x: f64) -> [f64; 2] {
    ZONE.inverse(x, 600_000.0).unwrap()
}

struct Dir(PathBuf);
impl Dir {
    fn new(tag: &str) -> Dir {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("owd-b2b-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("left.geojson"), text(&bank(-100.0))).unwrap();
        std::fs::write(p.join("right.geojson"), text(&bank(100.0))).unwrap();
        Dir(p)
    }
    fn path(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn owd(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_owd"))
        .args(args)
        .output()
        .unwrap()
}

fn said(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// `owd new` then `owd import --left-bank --right-bank [--downstream]`, in `d`.
fn cli_import(d: &Dir, downstream: Option<&str>) -> Output {
    let doc = d.path("d.owd");
    let o = owd(&["new", &doc, "--title", "Flows west", "--date", "2026-01-02"]);
    assert!(o.status.success(), "{}", said(&o));
    let (l, r) = (d.path("left.geojson"), d.path("right.geojson"));
    let mut args = vec!["import", &doc, "--left-bank", &l, "--right-bank", &r];
    if let Some(p) = downstream {
        args.extend(["--downstream", p]);
    }
    owd(&args)
}

fn read(path: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn line(doc: &Value, role: &str) -> Vec<[f64; 2]> {
    doc["inputs"][role]["coordinates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()])
        .collect()
}

fn without_provenance(mut d: Value) -> Value {
    d.as_object_mut().unwrap().remove("provenance");
    d
}

#[test]
fn a_downstream_point_names_and_orients_the_banks_the_same_on_both_surfaces() {
    // Downstream is the east end: looking east, the south bank is on the right.
    let d = Dir::new("both");
    let [lon, lat] = lon_lat_at(503_200.0);
    let point = format!("{lon},{lat}");
    let o = cli_import(&d, Some(&point));
    assert!(o.status.success(), "{}", said(&o));
    let cli = read(&d.path("d.owd"));

    // Stored under the right names, both running downstream (east).
    let (left, right) = (line(&cli, "left_bank"), line(&cli, "right_bank"));
    assert!(
        left.iter().all(|p| (p[1] - 600_100.0).abs() < 1e-6),
        "left is the north bank"
    );
    assert!(
        right.iter().all(|p| (p[1] - 599_900.0).abs() < 1e-6),
        "right is the south bank"
    );
    assert!(left[0][0] < left[left.len() - 1][0] && right[0][0] < right[right.len() - 1][0]);

    // The bank checks pass on them.
    let o = owd(&["check-banks", &d.path("d.owd")]);
    assert!(o.status.success(), "{}", said(&o));

    // The shell, with the same point, gives the same document.
    let mut shell = AppState::open(
        Path::new(&d.path("s.owd")),
        &AppState::new_design("Flows west", "2026-01-02")
            .save_text_for(Path::new(&d.path("s.owd"))),
        None,
    )
    .unwrap();
    shell.set_downstream_point(Some([lon, lat])).unwrap();
    shell
        .load_inputs(
            &[
                (Role::LeftBank, text(&bank(-100.0))),
                (Role::RightBank, text(&bank(100.0))),
            ],
            "2026-01-02",
        )
        .unwrap();
    let from_shell: Value =
        serde_json::from_str(&shell.save_text_for(Path::new(&d.path("d.owd")))).unwrap();
    assert_eq!(
        without_provenance(from_shell.clone()),
        without_provenance(cli.clone())
    );

    // Both record the point the user gave, with the banks it oriented.
    for doc in [&cli, &from_shell] {
        for role in ["left_bank", "right_bank"] {
            let got = &doc["provenance"][role]["settings"]["downstream_lon_lat"];
            assert_eq!(got, &json!([lon, lat]), "{role}: {}", doc["provenance"]);
        }
    }
}

#[test]
fn banks_named_for_the_other_way_get_one_plain_message_not_a_line_per_station() {
    let d = Dir::new("wholesale");
    let o = cli_import(&d, None);
    assert!(
        o.status.success(),
        "the import still succeeds: {}",
        said(&o)
    );
    let msg = said(&o);
    assert!(msg.contains("wrong way round"), "{msg}");
    assert!(msg.contains("--downstream"), "{msg}");
    assert!(
        !msg.contains("the banks fail the `side` check at"),
        "the per-check side line gives way to the plain message: {msg}"
    );

    // `owd check-banks` says the same thing first, and does not print a line per station.
    let o = owd(&["check-banks", &d.path("d.owd")]);
    let out = said(&o);
    assert!(
        out.contains("wrong way round") && out.contains("--downstream"),
        "{out}"
    );
    assert!(
        out.lines().count() < 10,
        "check-banks printed a line per station:\n{out}"
    );
    // The list is still there in the JSON.
    let o = owd(&["check-banks", &d.path("d.owd"), "--json"]);
    let v: Value = serde_json::from_str(&String::from_utf8_lossy(&o.stdout)).unwrap();
    assert!(v["failed"].as_array().unwrap().len() > 100, "{v}");
}

#[test]
fn a_bad_downstream_point_is_refused_by_name() {
    let d = Dir::new("bad");
    let o = cli_import(&d, Some("abc"));
    assert_eq!(o.status.code(), Some(2), "{}", said(&o));
    assert!(said(&o).contains("--downstream"), "{}", said(&o));
    let o = cli_import(&d, Some("200,5"));
    assert_eq!(o.status.code(), Some(2), "{}", said(&o));
    assert!(
        said(&o).contains("is not a longitude and latitude"),
        "{}",
        said(&o)
    );
}

#[test]
fn a_point_at_the_upstream_end_of_the_centreline_is_refused() {
    // The centreline runs east: a downstream point at its west end disagrees with it.
    let d = Dir::new("upstream");
    std::fs::write(d.0.join("centre.geojson"), text(&bank(0.0))).unwrap();
    let doc = d.path("d.owd");
    let o = owd(&["new", &doc, "--title", "T", "--date", "2026-01-02"]);
    assert!(o.status.success(), "{}", said(&o));
    let [lon, lat] = lon_lat_at(500_000.0);
    let point = format!("{lon},{lat}");
    let (l, r, c) = (
        d.path("left.geojson"),
        d.path("right.geojson"),
        d.path("centre.geojson"),
    );
    let o = owd(&[
        "import",
        &doc,
        "--left-bank",
        &l,
        "--right-bank",
        &r,
        "--centreline",
        &c,
        "--downstream",
        &point,
    ]);
    assert_eq!(o.status.code(), Some(2), "{}", said(&o));
}
