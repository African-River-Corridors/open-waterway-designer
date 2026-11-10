//! A point of a reach is range-checked in one place, below both surfaces.
//!
//! A start, end or via point that is not a longitude from -180 to 180 and a latitude from -90 to
//! 90 is refused through the app shell and through the command line with the same words: when the
//! route is planned from points, and when a reach is cut out of a chosen river.
//!
//! Label: property

use owd_cli::commands::{ReachFlags, reach_with};
use owd_reach::{DroppedFiles, ReplayFetch};
use owd_shell::reach::{ReachEndpoints, plan_reach, read_river};
use owd_shell::state::AppState;
use std::path::{Path, PathBuf};

const OVERPASS: &str = "https://overpass.example/api/interpreter";
const RIVER: &str = "relation/3328713";
/// Two points on the recorded river.
const ON_RIVER: [[f64; 2]; 2] = [[-2.2688538, 4.8983929], [-2.1349229, 5.7294889]];
/// A longitude and a latitude out of range.
const BAD: [[f64; 2]; 2] = [[200.0, 5.0], [-2.25, 95.0]];

fn endpoints() -> ReachEndpoints {
    ReachEndpoints {
        overpass: OVERPASS.into(),
        ..ReachEndpoints::default()
    }
}

/// The recorded Overpass answer for the river.
fn recorded() -> ReplayFetch {
    let body = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/reach/overpass_ankobra.json"),
    )
    .unwrap();
    ReplayFetch::new().with_response_containing("rel%283328713%29", body)
}

fn text(p: [f64; 2]) -> String {
    format!("{},{}", p[0], p[1])
}

fn work_file(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("owd-point-check-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("reach.owd")
}

/// `owd reach FILE --start .. --end .. [--via ..] [--osm-id ..]`. `Err` holds the refusal.
fn cli_reach(
    file: &Path,
    osm_id: Option<&str>,
    start: [f64; 2],
    end: [f64; 2],
    via: &[[f64; 2]],
    fetch: &ReplayFetch,
) -> Result<(), String> {
    let flags = ReachFlags {
        banks: false,
        seed: None,
        jrc_tile: Vec::new(),
        mask_threshold: None,
        edge_fit: false,
        max_cloud: None,
        stac: None,
        file: file.to_string_lossy().into_owned(),
        river: None,
        osm_id: osm_id.map(str::to_string),
        start: Some(text(start)),
        end: Some(text(end)),
        via: via.iter().map(|p| text(*p)).collect(),
        trace: None,
        title: Some("Point check".into()),
        overpass: Some(OVERPASS.into()),
        date: Some("2026-10-22".into()),
        json: true,
    };
    reach_with(flags, fetch, &DroppedFiles::new()).map_err(|f| f.message)
}

fn same_words(what: &str, bad: [f64; 2], shell: &str, cli: &str) {
    let number = if bad[0].abs() > 180.0 { bad[0] } else { bad[1] };
    assert!(
        shell.contains(&number.to_string()),
        "{what}: the shell's refusal does not name the bad number {number}: {shell}"
    );
    assert_eq!(
        shell, cli,
        "{what}: the shell and the command line refuse {bad:?} with different words"
    );
}

#[test]
fn a_route_from_points_refuses_a_point_out_of_range_with_the_same_words() {
    let file = work_file("points");
    for bad in BAD {
        // Start, end and via in turn. Nothing is recorded: the refusal comes before any request.
        for (start, end, via) in [
            (bad, ON_RIVER[1], vec![]),
            (ON_RIVER[0], bad, vec![]),
            (ON_RIVER[0], ON_RIVER[1], vec![bad]),
        ] {
            let fetch = ReplayFetch::new();
            let shell = match plan_reach(&fetch, &endpoints(), None, start, end, &via) {
                Ok(_) => panic!("the shell planned a route through {bad:?}"),
                Err(why) => why,
            };
            let cli = cli_reach(&file, None, start, end, &via, &fetch)
                .expect_err("the command line refuses the point");
            same_words("a route from points", bad, &shell, &cli);
            assert!(
                fetch.requests().is_empty(),
                "a request went out for {bad:?}"
            );
        }
    }
    assert!(!file.exists(), "a refused reach wrote a document");
}

#[test]
fn a_cut_of_a_chosen_river_refuses_a_point_out_of_range_with_the_same_words() {
    let file = work_file("cut");
    let fetch = recorded();
    let river = read_river(&fetch, &endpoints(), RIVER, &[]).unwrap();
    let mut state = AppState::new_design("Point check", "2026-10-22");
    state.start_river(river, None, "2026-10-22").unwrap();
    let before = state.snapshot()["document"].clone();
    for bad in BAD {
        for (start, end, via) in [
            (bad, ON_RIVER[1], vec![]),
            (ON_RIVER[0], bad, vec![]),
            (ON_RIVER[0], ON_RIVER[1], vec![bad]),
        ] {
            let shell = state
                .reach_cut(start, end, &via, "2026-10-22")
                .expect_err("the shell refuses the point");
            let cli = cli_reach(&file, Some(RIVER), start, end, &via, &fetch)
                .expect_err("the command line refuses the point");
            same_words("a cut of a chosen river", bad, &shell, &cli);
        }
    }
    assert_eq!(before, state.snapshot()["document"]);
    assert!(!file.exists(), "a refused reach wrote a document");
}
