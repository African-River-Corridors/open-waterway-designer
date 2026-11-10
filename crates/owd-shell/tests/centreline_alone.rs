//! A river centreline alone is a full line run.
//!
//! A design with only a river centreline (no bank lines, no river polygon) gives a design line on
//! the command line and in the app shell, and the two documents are equal. It works at the scale
//! of a whole river. A stage that needs the banks refuses, and says that the design has no banks.
//!
//! Label: property

use owd_cli::commands::{self, AlignFlags, SizeFlags, WaterFlags};
use owd_format::design::Role;
use owd_shell::state::AppState;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const DATE: &str = "2026-01-02";
/// The minimum bend radius of the sizing below: three lengths of an 85 m vessel.
const R_MIN_M: f64 = 255.0;

/// A fresh folder under the system's temporary folder, removed at the end of the test.
struct Dir(PathBuf);
impl Dir {
    fn new(tag: &str) -> Dir {
        let p = std::env::temp_dir().join(format!("owd-centreline-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Dir(p)
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// One meander, about 5.5 km long, that ends on the heading it began on: straights, two gentle
/// bends (radius 2,000 m) and two tight bends (radius 150 m, tighter than the minimum radius).
/// Each piece is `(length_m, radius_m)`: no radius is a straight; a positive radius turns left.
const MEANDER: [(f64, f64); 9] = [
    (600.0, 0.0),
    (1400.0, 2000.0),
    (400.0, 0.0),
    (150.0, -150.0),
    (300.0, 0.0),
    (150.0, 150.0),
    (400.0, 0.0),
    (1400.0, -2000.0),
    (600.0, 0.0),
];

/// A centreline of at least `length_m`, in UTM 30N, with a position every 15 m: the meander, then
/// its mirror image, and so on. It runs east.
fn centreline(length_m: f64) -> Vec<[f64; 2]> {
    let step = 15.0;
    let (mut x, mut y, mut heading) = (300_000.0_f64, 600_000.0_f64, 0.0_f64);
    let mut pts = vec![[x, y]];
    let mut side = 1.0;
    while (pts.len() - 1) as f64 * step < length_m {
        for (length, radius) in MEANDER {
            for _ in 0..(length / step).round() as usize {
                let turn = if radius == 0.0 {
                    0.0
                } else {
                    side * step / radius
                };
                // One 15 m chord of the arc.
                let mid = heading + turn / 2.0;
                x += step * mid.cos();
                y += step * mid.sin();
                heading += turn;
                pts.push([x, y]);
            }
        }
        side = -side;
    }
    pts
}

fn length_m(pts: &[[f64; 2]]) -> f64 {
    pts.windows(2)
        .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
        .sum()
}

fn geojson(pts: &[[f64; 2]]) -> String {
    json!({
        "type": "LineString",
        "crs": {"type": "name", "properties": {"name": "urn:ogc:def:crs:EPSG::32630"}},
        "coordinates": pts,
    })
    .to_string()
}

fn points(line: &Value) -> Vec<[f64; 2]> {
    line["coordinates"]
        .as_array()
        .expect("the option has a line")
        .iter()
        .map(|p| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()])
        .collect()
}

fn ok(step: &str, done: commands::Result) {
    if let Err(f) = done {
        panic!("`owd {step}` failed: {}", f.message);
    }
}

/// The command line: a new design in `dir`, the centreline imported, the channel sized. Returns
/// the document's path.
fn cli_design(dir: &Dir, line: &[[f64; 2]]) -> String {
    let doc = dir.0.join("cli.owd").to_string_lossy().into_owned();
    let file = dir.0.join("centreline.geojson");
    std::fs::write(&file, geojson(line)).unwrap();
    ok("new", commands::new(&doc, "Centreline", Some(DATE), true));
    ok(
        "import",
        commands::import(
            &doc,
            [
                ("left_bank", None),
                ("right_bank", None),
                ("centreline", Some(file.to_string_lossy().into_owned())),
                ("river_polygon", None),
            ],
            None,
            WaterFlags::default(),
            &None,
            true,
        ),
    );
    ok(
        "size",
        commands::size(
            &doc,
            SizeFlags {
                rule_sets: vec!["pianc-wg141".into()],
                set_inputs: vec!["se_quality=B".into()],
                vessel: Some("CEMT_IV".into()),
                draught: Some(2.5),
                custom: None,
            },
            &None,
            true,
        ),
    );
    doc
}

fn cli_align(doc: &str) -> commands::Result {
    commands::align(
        doc,
        "rolling_circle",
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
        true,
    )
}

/// The app shell: the same design.
fn shell_design(line: &[[f64; 2]]) -> AppState {
    let mut s = AppState::new_design("Centreline", DATE);
    s.load_inputs(&[(Role::Centreline, geojson(line))], DATE)
        .unwrap();
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
        .unwrap();
    s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
    s
}

fn shell_run(s: &mut AppState, stage: &str, method: Option<&str>) -> Result<&'static str, String> {
    let (job, _) = s.begin_run(stage, method)?;
    let outcome = job.execute(&|_| {});
    s.finish_run(outcome)
}

/// The document, without the provenance records: ticket A2 (one import) makes those equal.
fn document(text: &str) -> Value {
    let mut d: Value = serde_json::from_str(text).unwrap();
    d.as_object_mut().unwrap().remove("provenance");
    d
}

fn read(doc: &str) -> Value {
    document(&std::fs::read_to_string(doc).unwrap())
}

#[test]
fn a_centreline_alone_gives_the_same_design_line_on_both_surfaces() {
    let line = centreline(5_000.0);
    let dir = Dir::new("both");

    let doc = cli_design(&dir, &line);
    ok("align", cli_align(&doc));
    let cli = read(&doc);

    let mut shell = shell_design(&line);
    assert_eq!(
        shell_run(&mut shell, "alignment", Some("rolling_circle")),
        Ok("done")
    );
    let shell = document(&shell.save_text_for(Path::new(&doc)));

    assert_eq!(cli, shell, "the two surfaces give different documents");
    for bank in ["left_bank", "right_bank", "river_polygon"] {
        assert!(cli["inputs"][bank].is_null(), "{bank}");
    }
    let option = &cli["options"][0];
    assert_eq!(option["method"], "rolling_circle");
    // Easing the tight bends changes the length a little.
    let (given, made) = (length_m(&line), length_m(&points(&option["line"])));
    assert!(
        (made - given).abs() <= 0.01 * given,
        "the centreline is {given:.0} m long, the design line {made:.0} m"
    );
    // The centreline has bends of 150 m radius. The design line passes the option's own
    // minimum radius check.
    assert_eq!(option["checks"]["r_min_m"], R_MIN_M);
    assert_eq!(option["checks"]["radius_ok"], true, "{}", option["checks"]);
}

#[test]
fn a_centreline_of_a_whole_river_gives_a_design_line_in_good_time() {
    let line = centreline(200_000.0);
    let given = length_m(&line);
    assert!((200_000.0..206_000.0).contains(&given), "{given}");
    let mut shell = shell_design(&line);

    let start = Instant::now();
    assert_eq!(
        shell_run(&mut shell, "alignment", Some("rolling_circle")),
        Ok("done")
    );
    let took = start.elapsed();
    assert!(
        took < Duration::from_secs(SCALE_LIMIT_S),
        "the rolling circle took {took:?} on {given:.0} m of centreline"
    );

    let d = document(&shell.save_text_for(Path::new("whole-river.owd")));
    let option = &d["options"][0];
    let made = length_m(&points(&option["line"]));
    assert!(
        (made - given).abs() <= 0.01 * given,
        "the centreline is {given:.0} m long, the design line {made:.0} m"
    );
    assert_eq!(option["checks"]["radius_ok"], true, "{}", option["checks"]);
}

/// The time limit of the whole-river run, in a debug build.
const SCALE_LIMIT_S: u64 = 120;

#[test]
fn what_fits_says_that_the_design_has_no_banks() {
    let line = centreline(5_000.0);
    let dir = Dir::new("fits");

    let doc = cli_design(&dir, &line);
    let before = read(&doc);
    let refusal = commands_fits(&doc).expect_err("`owd fits` must refuse: there are no banks");
    assert_eq!(refusal.code, 2);
    assert!(
        refusal.message.contains("no banks"),
        "`owd fits` said: {}",
        refusal.message
    );
    assert_eq!(read(&doc), before, "a refusal changes nothing");

    let mut shell = shell_design(&line);
    let said = shell.run_fits().expect_err("the shell must refuse too");
    assert_eq!(said, refusal.message, "the two surfaces say the same");
}

fn commands_fits(doc: &str) -> commands::Result {
    owd_cli::fits::fits(doc, &[], false, None, false, None, &None, true)
}
