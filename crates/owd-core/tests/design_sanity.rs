//! The design sanity check on synthetic shapes: a self-crossing line, a bend under R_min and a
//! station whose cut is far above its neighbours' are each found; a clean line and an even cut
//! raise nothing. A long cut that is huge against the channel's own section is found by the extreme
//! check, which the spike check misses by design.
//!
//! Label: property

use owd_core::geom::Pt;
use owd_core::sanity::{
    CutStation, EXTREME_AREA_RATIO, EXTREME_FACE_RATIO, Finding, cut_spikes, extreme_cuts,
    line_findings,
};

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

/// A channel section of 100 m2 and a design depth of 3 m, `n` stations 25 m apart, each with the
/// given cut area and face height.
fn stations(n: usize, cut_m2: f64, face_h_m: f64) -> Vec<CutStation> {
    (0..n)
        .map(|i| CutStation {
            chainage_m: i as f64 * 25.0,
            cut_m2,
            face_h_m,
            channel_area_m2: 100.0,
            depth_m: 3.0,
        })
        .collect()
}

#[test]
fn a_cut_of_the_size_of_the_channel_raises_nothing() {
    // Twice the channel's section and a face of 2 m: a bank shaping.
    assert!(extreme_cuts(&stations(100, 200.0, 2.0)).is_empty());
    // Exactly at the bounds is not beyond them.
    let at = stations(20, EXTREME_AREA_RATIO * 100.0, EXTREME_FACE_RATIO * 3.0);
    assert!(extreme_cuts(&at).is_empty());
}

#[test]
fn a_long_extreme_cut_through_a_hill_is_found_though_it_is_no_spike() {
    // 40 stations of 5000 m2 (50 times the section): the neighbours are high too.
    let mut st = stations(100, 200.0, 2.0);
    for s in &mut st[30..70] {
        s.cut_m2 = 5000.0;
    }
    let cuts: Vec<(f64, f64)> = st.iter().map(|s| (s.chainage_m, s.cut_m2)).collect();
    assert!(
        cut_spikes(&cuts).is_empty(),
        "the spike check misses it by design"
    );
    let f = extreme_cuts(&st);
    assert_eq!(f.len(), 1, "{f:?}");
    match &f[0] {
        Finding::CutExtreme {
            from_m,
            to_m,
            max_cut_m2,
            max_area_ratio,
            ..
        } => {
            assert_eq!((*from_m, *to_m), (750.0, 1725.0));
            assert_eq!(*max_cut_m2, 5000.0);
            assert_eq!(*max_area_ratio, 50.0);
        }
        other => panic!("{other:?}"),
    }
    assert!(f[0].message().contains("very large for the channel"));
}

#[test]
fn a_tall_face_alone_is_found() {
    // A small area but a face of 40 m (more than 10 times the 3 m depth), at one station.
    let mut st = stations(50, 200.0, 2.0);
    st[10].face_h_m = 40.0;
    let f = extreme_cuts(&st);
    assert_eq!(f.len(), 1, "{f:?}");
    match &f[0] {
        Finding::CutExtreme {
            from_m,
            to_m,
            max_face_h_m,
            max_face_ratio,
            ..
        } => {
            assert_eq!((*from_m, *to_m), (250.0, 250.0));
            assert_eq!(*max_face_h_m, 40.0);
            assert!((*max_face_ratio - 40.0 / 3.0).abs() < 1e-12);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn two_extreme_stretches_are_two_findings() {
    let mut st = stations(100, 200.0, 2.0);
    for i in (10..13).chain(60..62) {
        st[i].cut_m2 = 3000.0;
    }
    assert_eq!(extreme_cuts(&st).len(), 2);
}

#[test]
fn the_bound_goes_with_the_channels_own_section() {
    // The same 3000 m2 cut is extreme beside a 100 m2 channel and not beside a 400 m2 one.
    let mut small = stations(10, 3000.0, 2.0);
    assert_eq!(extreme_cuts(&small).len(), 1);
    for s in &mut small {
        s.channel_area_m2 = 400.0;
    }
    assert!(extreme_cuts(&small).is_empty());
}

#[test]
fn a_station_with_no_reading_is_skipped_not_flagged() {
    let mut st = stations(10, f64::NAN, f64::NAN);
    assert!(extreme_cuts(&st).is_empty());
    st[3].channel_area_m2 = 0.0; // no channel section: the area ratio is not read
    st[3].cut_m2 = 1e6;
    assert!(extreme_cuts(&st).is_empty());
}
