//! The percentile of the water level is checked in one place, below both surfaces.
//!
//! A percentile outside 0 to 100 (or not a number) is refused through the app shell and through
//! the command line, with the same words, before anything is downloaded. No surface can reach the
//! water level code with a percentile it would panic on.
//!
//! Label: property

use owd_cli::level::{Endpoints, LevelFlags, water_level_with};
use owd_format::design::crs::Utm;
use owd_format::design::{Role, Session};
use owd_reach::ReplayFetch;
use owd_shell::reach::{ReachEndpoints, ReachStep};
use owd_shell::state::AppState;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

const RECORD: &str = "https://zenodo.example/api/records/sword";
const ZIP: &str = "https://zenodo.example/sword.zip";
const HYDROCRON: &str = "https://hydrocron.example/timeseries";
const REACHES: [&str; 5] = [
    "14560100021",
    "14560100033",
    "14560100041",
    "14560100051",
    "14560100061",
];
const BAD: [f64; 3] = [150.0, -1.0, f64::NAN];

fn ex(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/reach")
            .join(name),
    )
    .unwrap()
}

/// The recorded SWORD subset and the recorded SWOT series of its five reaches.
fn recorded() -> ReplayFetch {
    let zip = ex("sword_v17c_shp_subset.zip");
    let record = format!(
        r#"{{"files":[{{"key":"SWORD_v17c_shp.zip","size":{},"links":{{"self":"{ZIP}"}}}}]}}"#,
        zip.len()
    );
    let mut f = ReplayFetch::new()
        .with_response(RECORD, record)
        .with_response(ZIP, zip);
    for id in REACHES {
        f = f.with_response_containing(
            &format!("feature_id={id}&"),
            ex(&format!("hydrocron_{id}.json")),
        );
    }
    f
}

fn endpoints() -> ReachEndpoints {
    ReachEndpoints {
        sword_record: RECORD.into(),
        hydrocron: HYDROCRON.into(),
        ..ReachEndpoints::default()
    }
}

/// A saved study whose centreline is the line of the recorded reaches. It has no terrain.
fn study(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("owd-percentile-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let reaches =
        owd_reach::sword::reaches_in(&recorded(), RECORD, [-6.4, 4.9, -6.0, 5.2]).unwrap();
    let mut line: Vec<[f64; 2]> = Vec::new();
    for r in &reaches {
        let mut l = r.line.clone();
        match line.last() {
            Some(last) => {
                let d = |p: [f64; 2]| (p[0] - last[0]).powi(2) + (p[1] - last[1]).powi(2);
                if d(l[l.len() - 1]) < d(l[0]) {
                    l.reverse();
                }
            }
            None if l[0][0] < l[l.len() - 1][0] => l.reverse(),
            None => {}
        }
        line.extend(l);
    }
    let utm = Utm {
        zone: 29,
        south: false,
    }
    .forward_all(&line)
    .unwrap();
    let gj = format!(
        r#"{{"type":"LineString","coordinates":{},"crs":{{"type":"name","properties":{{"name":"urn:ogc:def:crs:EPSG::32629"}}}}}}"#,
        serde_json::to_string(&utm).unwrap()
    );
    let mut s = Session::new("Percentile check", "2026-10-21");
    s.import(&[(Role::Centreline, gj.as_str())]).unwrap();
    let file = dir.join("study.owd");
    std::fs::write(&file, s.to_text()).unwrap();
    file
}

/// The water level step through the app shell. `Err` holds the refusal.
fn shell_level(file: &Path, percentile: f64, fetch: &ReplayFetch) -> Result<(), String> {
    let text = std::fs::read_to_string(file).unwrap();
    let state = AppState::open(file, &text, None).unwrap();
    let step = ReachStep::of("level", Some(percentile))?;
    let job = state.begin_reach(step)?;
    job.run(fetch, &endpoints(), "2026-10-01").map(|_| ())
}

/// `owd water-level --percentile P`. `Err` holds the refusal.
fn cli_level(file: &Path, percentile: f64, fetch: &ReplayFetch) -> Result<(), String> {
    let flags = LevelFlags {
        doc: file.to_string_lossy().into_owned(),
        percentile: Some(percentile),
        min_swot_width: None,
        from: None,
        to: Some("2026-10-01".into()),
        dem_datum: None,
        output: None,
        json: true,
    };
    let at = Endpoints {
        sword_record: RECORD,
        hydrocron: HYDROCRON,
    };
    water_level_with(flags, fetch, &at).map_err(|f| f.message)
}

#[test]
fn the_shell_refuses_a_bad_percentile_and_does_not_panic() {
    let file = study("panic");
    for p in BAD {
        // Every source answers, so the step reaches the percentile of real observations.
        let fetch = recorded();
        let done = catch_unwind(AssertUnwindSafe(|| shell_level(&file, p, &fetch)));
        let Ok(done) = done else {
            panic!("the water level step panicked on a percentile of {p}");
        };
        let why = done.expect_err("a bad percentile is refused");
        assert!(
            why.contains("percentile") && why.contains("0 to 100"),
            "the refusal of a percentile of {p} does not give the range: {why}"
        );
    }
}

#[test]
fn both_surfaces_refuse_a_bad_percentile_with_the_same_words_before_any_download() {
    let file = study("words");
    let before = std::fs::read_to_string(&file).unwrap();
    for p in BAD {
        // Nothing is recorded: any request fails, so a refusal that names the percentile came first.
        let (for_shell, for_cli) = (ReplayFetch::new(), ReplayFetch::new());
        let shell = shell_level(&file, p, &for_shell).expect_err("the shell refuses it");
        let cli = cli_level(&file, p, &for_cli).expect_err("the command line refuses it");
        assert!(
            shell.contains("percentile") && shell.contains("0 to 100"),
            "the shell's refusal of a percentile of {p} does not give the range: {shell}"
        );
        assert!(
            cli.contains("--percentile") && cli.ends_with(&shell),
            "the command line does not refuse a percentile of {p} with the shell's words.\n \
             shell: {shell}\n command line: {cli}"
        );
        assert!(
            for_shell.requests().is_empty() && for_cli.requests().is_empty(),
            "a download began before the percentile of {p} was refused"
        );
    }
    assert_eq!(before, std::fs::read_to_string(&file).unwrap());
}
