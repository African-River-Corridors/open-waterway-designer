//! The centreline of a river from its two banks (spec 0013): their mid-line. One function, used by the
//! bank generator, the edge fit and the bank editor, so the three never disagree.
//!
//! It starts from a seed line. Then, three times: at stations along the line, cast the normal both
//! ways to the first crossing with each bank on its own side, and move the station to the midpoint of the two
//! crossings. A light smoothing goes between the passes, not after the last one, so the line stored is
//! a mid-line. A station whose normal meets one bank or neither keeps its place.
//!
//! The geometry is planar, in project metres. Square roots go through `libm`, so native and wasm give
//! the same bits.
//!
//! Label: property

type Pt = [f64; 2];

// The station helpers moved to the core (ticket B3a); they keep their names here.
pub use owd_core::banks::stations::{
    GAP_FACTOR, GAP_MIN_M, MAX_RAY_M, MIDLINE_PASSES, MIN_MOVE_M, SegIndex, TANGENT_HALF_M, arc,
    arc_normals, bank_index, bank_offsets, gap_length, median_width, midline_offsets, normals,
    point_at, resample,
};
use owd_core::banks::stations::{cell_for, side_hits, tangent_half};

/// The mid-line of two banks, at stations about `station_m` apart, from a seed line (for example the
/// last centreline, or the two banks paired by their share of length).
pub fn midline(left: &[Pt], right: &[Pt], seed: &[Pt], station_m: f64) -> Vec<Pt> {
    if left.len() < 2 || right.len() < 2 || seed.len() < 2 {
        return seed.to_vec();
    }
    let cell = cell_for(left, right);
    let (li, ri) = (bank_index(left, cell), bank_index(right, cell));
    let mut line = resample(seed, station_m);
    let mut buf = Vec::new();
    // Normals over half the river's width at least, so a wide river does not swing them.
    let half = tangent_half(&line, &li, &ri);
    // A station moves at most about one river width in a pass: a normal that meets a bank farther
    // off has met a fault of the banks (a loop, a crossing), and the station keeps its place.
    let max_move = (2.0 * half).max(MIN_MOVE_M);
    for pass in 0..MIDLINE_PASSES {
        let nrm = arc_normals(&line, half);
        line = line
            .iter()
            .zip(&nrm)
            .map(|(p, n)| match side_hits(&li, &ri, *p, *n, &mut buf) {
                (Some(a), Some(b)) if ((a + b) / 2.0).abs() <= max_move => {
                    let t = (a + b) / 2.0;
                    [p[0] + n[0] * t, p[1] + n[1] * t]
                }
                _ => *p,
            })
            .collect();
        if pass + 1 < MIDLINE_PASSES {
            line = resample(&smooth(&line), station_m);
        }
    }
    line
}

/// One pass of a 1-2-1 smoothing; the ends stay.
fn smooth(line: &[Pt]) -> Vec<Pt> {
    let n = line.len();
    (0..n)
        .map(|i| {
            if i == 0 || i + 1 == n {
                line[i]
            } else {
                [
                    (line[i - 1][0] + 2.0 * line[i][0] + line[i + 1][0]) / 4.0,
                    (line[i - 1][1] + 2.0 * line[i][1] + line[i + 1][1]) / 4.0,
                ]
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use owd_core::banks::stations::dist;

    fn wavy(off: f64, n: usize) -> Vec<Pt> {
        (0..=n)
            .map(|i| {
                let x = i as f64 * 10.0;
                [x, off + 30.0 * libm::sin(x / 400.0)]
            })
            .collect()
    }

    #[test]
    fn the_mid_line_of_smooth_banks_is_their_middle_within_a_metre() {
        // A meandering channel 100 m wide (in y), and a seed 40 m off to one side.
        let (left, right) = (wavy(100.0, 300), wavy(0.0, 300));
        let seed: Vec<Pt> = (0..=300).map(|i| [i as f64 * 10.0, 90.0]).collect();
        let m = midline(&left, &right, &seed, 20.0);
        let off = midline_offsets(&m, &left, &right);
        // Away from the ends, where the normal can miss a bank, every station is on the mid-line.
        for (i, o) in off.iter().enumerate().skip(3).take(off.len() - 6) {
            let o = o.unwrap_or_else(|| panic!("station {i} met no bank"));
            assert!(o < 1.0, "station {i} is {o} m off the mid-line");
        }
        // And between the banks.
        for (p, (l, r)) in m.iter().zip(bank_offsets(&m, &left, &right)) {
            if let (Some(l), Some(r)) = (l, r) {
                assert!(l > 0.0 && r < 0.0, "{p:?}: {l} {r}");
            }
        }
    }

    #[test]
    fn a_mid_line_lies_between_banks_of_any_shape_that_do_not_cross() {
        // Property over random smooth banks: the mid-line stays between them.
        let mut x: u64 = 11;
        let mut next = || {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((x >> 33) as f64) / (1u64 << 31) as f64
        };
        for _ in 0..40 {
            let (a, ph, w) = (next() * 60.0, next() * 6.0, 60.0 + next() * 300.0);
            let bank = |off: f64, k: f64| -> Vec<Pt> {
                (0..=200)
                    .map(|i| {
                        let x = i as f64 * 15.0;
                        [
                            x,
                            off + a * libm::sin(x / 500.0 + ph) + k * libm::sin(x / 90.0),
                        ]
                    })
                    .collect()
            };
            let (left, right) = (bank(w, 3.0), bank(0.0, -2.0));
            let seed: Vec<Pt> = (0..=200).map(|i| [i as f64 * 15.0, w * 0.2]).collect();
            let m = midline(&left, &right, &seed, 20.0);
            for (l, r) in bank_offsets(&m, &left, &right).into_iter().flatten_pairs() {
                assert!(l > 0.0 && r < 0.0, "width {w}: {l} {r}");
            }
        }
    }

    trait FlattenPairs {
        fn flatten_pairs(self) -> Vec<(f64, f64)>;
    }
    impl<I: Iterator<Item = (Option<f64>, Option<f64>)>> FlattenPairs for I {
        fn flatten_pairs(self) -> Vec<(f64, f64)> {
            self.filter_map(|(a, b)| Some((a?, b?))).collect()
        }
    }

    /// Issue #373: a bank with a gap (two vertices far apart, a straight chord across land) is not
    /// a bank. On the whole Sassandra run the right bank had an 11 km gap across a bend, the
    /// mid-line moved its stations halfway to the chord, and 6.7 km of centreline tangled in a
    /// 600 m box (93 self-crossings). A station whose normal meets only a gap keeps its place.
    #[test]
    fn a_gap_in_a_bank_does_not_tangle_the_mid_line() {
        // A meander 80 m wide (amplitude 600 m, wavelength 2.5 km) over 6 km, a vertex every 10 m.
        let bank = |off: f64| -> Vec<Pt> {
            (0..=600)
                .map(|i| {
                    let x = i as f64 * 10.0;
                    [x, off + 600.0 * libm::sin(x / 400.0)]
                })
                .collect()
        };
        let (left, full_right, centre) = (bank(40.0), bank(-40.0), bank(0.0));
        // The right bank loses x = 2 to 4 km: one straight segment across the bend.
        let right: Vec<Pt> = full_right
            .iter()
            .enumerate()
            .filter(|(i, _)| *i <= 200 || *i >= 400)
            .map(|(_, p)| *p)
            .collect();
        let m = midline(&left, &right, &centre, 10.0);
        let ratio = arc(&m).last().unwrap() / arc(&centre).last().unwrap();
        assert!(ratio < 1.05, "the mid-line is {ratio} times the river");
        // Every station stays within half the river's width of its middle.
        let worst = m
            .iter()
            .map(|p| {
                centre
                    .iter()
                    .map(|q| dist(*p, *q))
                    .fold(f64::INFINITY, f64::min)
            })
            .fold(0.0, f64::max);
        assert!(
            worst < 40.0,
            "a station is {worst} m off the river's middle"
        );
    }

    #[test]
    fn a_bank_with_even_long_segments_has_no_gap() {
        // A user's bank drawn with a vertex every 300 m: its segments are all alike, so none is a gap.
        let left: Vec<Pt> = (0..=10).map(|i| [i as f64 * 300.0, 50.0]).collect();
        let right: Vec<Pt> = (0..=10).map(|i| [i as f64 * 300.0, 0.0]).collect();
        let seed: Vec<Pt> = vec![[0.0, 10.0], [3000.0, 10.0]];
        let m = midline(&left, &right, &seed, 20.0);
        assert!(
            (m[m.len() / 2][1] - 25.0).abs() < 1e-6,
            "{:?}",
            m[m.len() / 2]
        );
    }

    #[test]
    fn a_normal_that_meets_one_bank_keeps_its_station() {
        let left: Vec<Pt> = (0..=10).map(|i| [i as f64 * 10.0, 50.0]).collect();
        let right: Vec<Pt> = (0..=10).map(|i| [i as f64 * 10.0, 0.0]).collect();
        // A seed that runs on past the banks' end.
        let seed: Vec<Pt> = vec![[0.0, 25.0], [300.0, 25.0]];
        let m = midline(&left, &right, &seed, 20.0);
        assert_eq!(*m.last().unwrap(), [300.0, 25.0]);
        assert!((m[2][1] - 25.0).abs() < 1e-9);
    }
}
