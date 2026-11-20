//! The design sanity check on synthetic shapes: a self-crossing line, a bend under R_min and a
//! station whose cut is far above its neighbours' are each found; a clean line and an even cut
//! raise nothing.
//!
//! Label: property

use owd_core::geom::Pt;
use owd_core::sanity::{Finding, cut_spikes, line_findings};

const CHORD: f64 = 85.0;
const R_MIN: f64 = 255.0;

fn straight(len: f64) -> Vec<Pt> {
    (0..=(len / 10.0) as usize)
        .map(|i| [i as f64 * 10.0, 0.0])
        .collect()
}

/// An arc of radius `r` for `sweep` radians, one point per 10 m.
fn arc(r: f64, sweep: f64) -> Vec<Pt> {
    let n = (r * sweep / 10.0).ceil() as usize;
    (0..=n)
        .map(|i| {
            let a = sweep * i as f64 / n as f64;
            [r * libm::sin(a), r * (1.0 - libm::cos(a))]
        })
        .collect()
}

#[test]
fn a_gentle_line_raises_nothing() {
    let mut line = straight(500.0);
    let a = arc(400.0, 0.8);
    let last = *line.last().unwrap();
    line.extend(a.iter().skip(1).map(|p| [last[0] + p[0], last[1] + p[1]]));
    assert!(line_findings(&line, CHORD, R_MIN).is_empty());
}

#[test]
fn a_line_that_crosses_itself_is_found_where_it_crosses() {
    // A figure: out along x, up, back across the first leg.
    let mut line: Vec<Pt> = (0..=100).map(|i| [i as f64 * 10.0, 0.0]).collect();
    line.extend((1..=50).map(|i| [1000.0, i as f64 * 10.0]));
    line.extend((1..=60).map(|i| [1000.0 - i as f64 * 10.0, 500.0]));
    line.extend((1..=60).map(|i| [400.0, 500.0 - i as f64 * 10.0]));
    let f = line_findings(&line, CHORD, R_MIN);
    let x = f
        .iter()
        .find_map(|f| match f {
            Finding::SelfCrossing { at, chainage_m } => Some((*at, *chainage_m)),
            _ => None,
        })
        .expect("a self-crossing");
    assert!(
        (x.0[0] - 400.0).abs() < 1e-6 && x.0[1].abs() < 1e-6,
        "{x:?}"
    );
    // The earlier of the two passes: 400 m along the first leg.
    assert!((x.1 - 400.0).abs() < 1e-6, "{x:?}");
}

#[test]
fn a_bend_under_r_min_is_found_with_its_stretch_and_radius() {
    let mut line = straight(300.0);
    let a = arc(150.0, 1.2);
    let last = *line.last().unwrap();
    line.extend(a.iter().skip(1).map(|p| [last[0] + p[0], last[1] + p[1]]));
    let last = *line.last().unwrap();
    let end_dir = [libm::cos(1.2), libm::sin(1.2)];
    line.extend((1..=30).map(|i| {
        [
            last[0] + end_dir[0] * i as f64 * 10.0,
            last[1] + end_dir[1] * i as f64 * 10.0,
        ]
    }));
    let f = line_findings(&line, CHORD, R_MIN);
    let bends: Vec<_> = f
        .iter()
        .filter_map(|f| match f {
            Finding::BendUnderRMin {
                from_m,
                to_m,
                min_radius_m,
                ..
            } => Some((*from_m, *to_m, *min_radius_m)),
            _ => None,
        })
        .collect();
    assert_eq!(bends.len(), 1, "{f:?}");
    let (from, to, r) = bends[0];
    // The arc runs from 300 m to 300 + 150 x 1.2 = 480 m.
    assert!(from >= 250.0 && to <= 530.0 && from < to, "{bends:?}");
    assert!((r - 150.0).abs() < 8.0, "radius {r}");
    assert!(f.iter().all(|f| !f.message().is_empty()));
}

fn even_cut(n: usize, v: f64) -> Vec<(f64, f64)> {
    (0..n).map(|i| (i as f64 * 25.0, v)).collect()
}

#[test]
fn an_even_cut_raises_nothing() {
    assert!(cut_spikes(&even_cut(100, 400.0)).is_empty());
    assert!(cut_spikes(&even_cut(100, 0.0)).is_empty());
}

#[test]
fn one_station_far_above_its_neighbours_is_found() {
    let mut cuts = even_cut(100, 120.0);
    cuts[40].1 = 2000.0;
    let f = cut_spikes(&cuts);
    assert_eq!(f.len(), 1, "{f:?}");
    match &f[0] {
        Finding::CutSpike {
            from_m,
            to_m,
            max_cut_m2,
            neighbours_median_m2,
        } => {
            assert_eq!((*from_m, *to_m), (1000.0, 1000.0));
            assert_eq!(*max_cut_m2, 2000.0);
            assert_eq!(*neighbours_median_m2, 120.0);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_run_of_high_stations_is_one_finding() {
    let mut cuts = even_cut(100, 100.0);
    for c in &mut cuts[30..33] {
        c.1 = 900.0;
    }
    let f = cut_spikes(&cuts);
    assert_eq!(f.len(), 1, "{f:?}");
}

#[test]
fn a_long_high_cut_is_not_a_spike() {
    // A cutting through a hill: high for 40 stations, so its neighbours are high too.
    let mut cuts = even_cut(100, 100.0);
    for c in &mut cuts[30..70] {
        c.1 = 900.0;
    }
    assert!(cut_spikes(&cuts).is_empty());
}

#[test]
fn a_small_cut_beside_none_is_not_a_spike() {
    let mut cuts = even_cut(100, 0.0);
    cuts[40].1 = 60.0;
    assert!(cut_spikes(&cuts).is_empty());
}
