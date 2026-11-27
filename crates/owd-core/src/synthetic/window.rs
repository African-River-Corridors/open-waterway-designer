//! Synthetic rivers for the window optimiser (OPT-5, section 5): made in code, in a local metric
//! frame, from no real river. Water level 10 m; the river bed at 5.5 m (under a grade of 6 m, so a
//! channel in the river needs no dredging); the ground beside the river at 11 m unless a fixture
//! says otherwise.

use crate::geom::offset::{Cap, buffer_line};
use crate::geom::{Geometry, Polygon, Pt};

/// The water level of every fixture (m).
pub const WATER_LEVEL_M: f64 = 10.0;
/// The river bed of every fixture (m): below the grade of a 4 m channel.
pub const BED_M: f64 = 5.5;
/// The ground beside the river (m).
pub const BANK_M: f64 = 11.0;

/// The river runs on this far (m) past each end of a starting line, so a section at an end of the
/// line is in the river as it is everywhere else.
pub const RUN_ON_M: f64 = 400.0;

/// A river, its starting line and its ground.
pub struct WindowFixture {
    pub centreline: Vec<Pt>,
    pub river: Vec<Polygon>,
    pub start_line: Vec<Pt>,
    /// The ground at a point (the river bed inside the river).
    pub ground: Box<dyn Fn(f64, f64) -> f64 + Sync + Send>,
}

impl WindowFixture {
    /// The ground as a terrain sampler.
    pub fn sampler(&self) -> impl Fn(&[f64], &[f64]) -> Vec<f64> + Sync + '_ {
        move |xs: &[f64], ys: &[f64]| {
            xs.iter()
                .zip(ys)
                .map(|(&x, &y)| (self.ground)(x, y))
                .collect()
        }
    }

    /// The same fixture mirrored across the x axis.
    pub fn mirrored(self) -> WindowFixture {
        let flip = |l: &[Pt]| l.iter().map(|p| [p[0], -p[1]]).collect::<Vec<Pt>>();
        let ground = self.ground;
        WindowFixture {
            centreline: flip(&self.centreline),
            river: self
                .river
                .iter()
                .map(|p| Polygon {
                    exterior: flip(&p.exterior).into_iter().rev().collect(),
                    holes: p
                        .holes
                        .iter()
                        .map(|h| flip(h).into_iter().rev().collect())
                        .collect(),
                })
                .collect(),
            start_line: flip(&self.start_line),
            ground: Box::new(move |x, y| ground(x, -y)),
        }
    }
}

/// The polygon of `line` buffered `half_m` either side (flat ends).
pub fn river_of(line: &[Pt], half_m: f64) -> Vec<Polygon> {
    match buffer_line(line, half_m, 16, Cap::Flat) {
        Ok(Geometry::Polygon(p)) => vec![p],
        Ok(Geometry::MultiPolygon(ps)) => ps,
        _ => Vec::new(),
    }
}

/// A path of straights and arcs from a pose: each `(radius, turn)` is an arc (turn in radians,
/// positive left) or, with radius infinite, a straight of `turn` metres. Points about every
/// `step_m`.
pub fn path(start: (f64, f64, f64), parts: &[(f64, f64)], step_m: f64) -> Vec<Pt> {
    let (mut x, mut y, mut th) = start;
    let mut out = vec![[x, y]];
    for &(r, t) in parts {
        if r.is_infinite() {
            let n = std::cmp::max(1, (t / step_m).ceil() as usize);
            for k in 1..=n {
                let s = t * k as f64 / n as f64;
                out.push([x + s * libm::cos(th), y + s * libm::sin(th)]);
            }
            x += t * libm::cos(th);
            y += t * libm::sin(th);
        } else {
            let len = r * t.abs();
            let n = std::cmp::max(2, (len / step_m).ceil() as usize);
            let k = t.signum() / r;
            for i in 1..=n {
                let s = len * i as f64 / n as f64;
                let (lx, ly) = (libm::sin(k * s) / k, (1.0 - libm::cos(k * s)) / k);
                out.push([
                    x + libm::cos(th) * lx - libm::sin(th) * ly,
                    y + libm::sin(th) * lx + libm::cos(th) * ly,
                ]);
            }
            let (lx, ly) = (libm::sin(k * len) / k, (1.0 - libm::cos(k * len)) / k);
            x += libm::cos(th) * lx - libm::sin(th) * ly;
            y += libm::sin(th) * lx + libm::cos(th) * ly;
            th += t;
        }
    }
    out
}

/// A smooth, repeatable relief of about `amp_m` (value noise on a 20 m lattice, from `seed`).
pub fn relief(seed: u64, amp_m: f64) -> impl Fn(f64, f64) -> f64 + Sync + Send {
    let h = move |i: i64, j: i64| -> f64 {
        let mut v = (i as u64)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add((j as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F))
            .wrapping_add(seed);
        v ^= v >> 31;
        v = v.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        v ^= v >> 29;
        (v % 10_000) as f64 / 10_000.0 - 0.5
    };
    move |x: f64, y: f64| {
        let (u, v) = (x / 20.0, y / 20.0);
        let (i, j) = (u.floor() as i64, v.floor() as i64);
        let (fu, fv) = (u - i as f64, v - j as f64);
        let a = h(i, j) * (1.0 - fu) + h(i + 1, j) * fu;
        let b = h(i, j + 1) * (1.0 - fu) + h(i + 1, j + 1) * fu;
        2.0 * amp_m * (a * (1.0 - fv) + b * fv)
    }
}

/// S3: a straight river `width_m` wide and `length_m` long along +x, its bed below grade, flat ground
/// at `bank_m` beside it. The starting line is the centreline.
pub fn straight_reach(length_m: f64, width_m: f64, bank_m: f64) -> WindowFixture {
    let start_line = path((0.0, 0.0, 0.0), &[(f64::INFINITY, length_m)], 10.0);
    let centreline = path(
        (-RUN_ON_M, 0.0, 0.0),
        &[(f64::INFINITY, length_m + 2.0 * RUN_ON_M)],
        10.0,
    );
    let river = river_of(&centreline, 0.5 * width_m);
    let r = crate::geom::overlay::PolygonIndex::new(&river);
    WindowFixture {
        start_line,
        centreline,
        river,
        ground: Box::new(move |x, y| if r.contains_xy([x, y]) { BED_M } else { bank_m }),
    }
}

/// S5 (b): the straight reach, with a starting line that humps `hump_m` sideways (to the left) over
/// `hump_len_m` in its middle, into a bank `bank_m` high on that side.
pub fn hump_reach(
    length_m: f64,
    width_m: f64,
    hump_m: f64,
    hump_len_m: f64,
    bank_m: f64,
) -> WindowFixture {
    let mut f = straight_reach(length_m, width_m, BANK_M);
    let x0 = 0.5 * (length_m - hump_len_m);
    f.start_line = f
        .start_line
        .iter()
        .map(|p| {
            let t = (p[0] - x0) / hump_len_m;
            let d = if (0.0..=1.0).contains(&t) {
                0.5 * hump_m * (1.0 - libm::cos(2.0 * core::f64::consts::PI * t))
            } else {
                0.0
            };
            [p[0], p[1] + d]
        })
        .collect();
    let r = crate::geom::overlay::PolygonIndex::new(&f.river);
    f.ground = Box::new(move |x, y| {
        if r.contains_xy([x, y]) {
            BED_M
        } else if y > 0.0 {
            bank_m
        } else {
            BANK_M
        }
    });
    f
}

/// S2: a river bend of radius `bend_r_m` turning 90 degrees left round a spur on its inside, with
/// `arm_m` of straight river before and after. The spur rises at 1:1 to 30 m above the water from
/// 10 m inside the river's inner edge; the outside of the bend is flat floodplain at 11 m. The
/// starting line is the arc of `r_min_m` tangent to the two arms.
pub fn bend_against_spur(bend_r_m: f64, width_m: f64, arm_m: f64, r_min_m: f64) -> WindowFixture {
    use core::f64::consts::FRAC_PI_2;
    let centreline = path(
        (-arm_m - RUN_ON_M, 0.0, 0.0),
        &[
            (f64::INFINITY, arm_m + RUN_ON_M),
            (bend_r_m, FRAC_PI_2),
            (f64::INFINITY, arm_m + RUN_ON_M),
        ],
        5.0,
    );
    let river = river_of(&centreline, 0.5 * width_m);
    // The arms meet at the corner (bend_r_m, 0); the R_min fillet starts r_min_m before it.
    let start_line = path(
        (-arm_m, 0.0, 0.0),
        &[
            (f64::INFINITY, arm_m + bend_r_m - r_min_m),
            (r_min_m, FRAC_PI_2),
            (f64::INFINITY, arm_m + bend_r_m - r_min_m),
        ],
        5.0,
    );
    let r = crate::geom::overlay::PolygonIndex::new(&river);
    let (cx, cy) = (0.0, bend_r_m);
    let inner_edge = bend_r_m - 0.5 * width_m;
    WindowFixture {
        centreline,
        river,
        start_line,
        ground: Box::new(move |x, y| {
            if r.contains_xy([x, y]) {
                return BED_M;
            }
            let d = libm::hypot(x - cx, y - cy);
            // The spur: inside the bend only (the quarter between the arms).
            if x >= cx && y <= cy {
                let rise = (inner_edge - 10.0 - d).clamp(0.0, 29.0);
                return BANK_M + rise;
            }
            BANK_M
        }),
    }
}

/// S1: an omega loop of R_min arcs (left `r_min_m`, right `r_min_m` round the loop, left `r_min_m`,
/// each turning so that the loop is three times its chord) between two straight arms of `arm_m`,
/// the river `width_m` wide along it, the starting line its centreline. A round hill of radius 150 m
/// with sides at 1:4 and its crest `hill_m` above the water stands in the neck, on the chord between
/// where the loop leaves the arms.
pub fn omega_loop(r_min_m: f64, width_m: f64, arm_m: f64, hill_m: f64) -> WindowFixture {
    // beta / sin(beta) = 3.
    let beta = 2.278_862_660_06;
    let parts = |run: f64| {
        [
            (f64::INFINITY, arm_m + run),
            (r_min_m, beta),
            (r_min_m, -2.0 * beta),
            (r_min_m, beta),
            (f64::INFINITY, arm_m + run),
        ]
    };
    let start_line = path((0.0, 0.0, 0.0), &parts(0.0), 5.0);
    let centreline = path((-RUN_ON_M, 0.0, 0.0), &parts(RUN_ON_M), 5.0);
    let chord = 4.0 * r_min_m * libm::sin(beta);
    let river = river_of(&centreline, 0.5 * width_m);
    let r = crate::geom::overlay::PolygonIndex::new(&river);
    let (hx, hy) = (arm_m + 0.5 * chord, 0.0);
    WindowFixture {
        start_line,
        centreline,
        river,
        ground: Box::new(move |x, y| {
            if r.contains_xy([x, y]) {
                return BED_M;
            }
            let d = libm::hypot(x - hx, y - hy);
            let top = WATER_LEVEL_M + hill_m;
            let z = if d <= 150.0 {
                top
            } else {
                top - (d - 150.0) / 4.0
            };
            z.max(BANK_M)
        }),
    }
}

/// The chord of [`omega_loop`]'s loop (m): where it leaves the first arm to where it meets the second.
pub fn omega_chord(r_min_m: f64) -> f64 {
    4.0 * r_min_m * libm::sin(2.278_862_660_06)
}

/// A long S of two opposite bends of `r_m` turning `turn` radians each, with straights of
/// `arm_m` before and after, and a ridge `ridge_m` above the water across the inside of the first
/// bend that a line using both bends avoids (S4).
pub fn long_s(r_m: f64, turn: f64, width_m: f64, arm_m: f64, ridge_m: f64) -> WindowFixture {
    let parts = |run: f64| {
        [
            (f64::INFINITY, arm_m + run),
            (r_m, turn),
            (r_m, -turn),
            (f64::INFINITY, arm_m + run),
        ]
    };
    let start_line = path((0.0, 0.0, 0.0), &parts(0.0), 5.0);
    let centreline = path((-RUN_ON_M, 0.0, 0.0), &parts(RUN_ON_M), 5.0);
    let river = river_of(&centreline, 0.5 * width_m);
    let r = crate::geom::overlay::PolygonIndex::new(&river);
    // The ridge: a band 200 m wide across the inside of the first bend, about its centre.
    let (cx, cy) = (arm_m, r_m);
    WindowFixture {
        start_line,
        centreline,
        river,
        ground: Box::new(move |x, y| {
            if r.contains_xy([x, y]) {
                return BED_M;
            }
            let d = libm::hypot(x - cx, y - cy);
            if d < r_m - 0.5 * width_m && d > r_m - 0.5 * width_m - 200.0 {
                WATER_LEVEL_M + ridge_m
            } else {
                BANK_M
            }
        }),
    }
}

/// A long sine river (`length_m`, wavelength and amplitude in metres) for the whole-river run: its
/// bends gentle enough that the centreline is the starting line, the ground beside it at 11 m with
/// a relief of 1.5 m.
pub fn sine_river(
    length_m: f64,
    wavelength_m: f64,
    amplitude_m: f64,
    width_m: f64,
) -> WindowFixture {
    let k = 2.0 * core::f64::consts::PI / wavelength_m;
    let sine = |x0: f64, x1: f64| -> Vec<Pt> {
        let n = ((x1 - x0) / 10.0).ceil() as usize;
        (0..=n)
            .map(|i| {
                let x = x0 + (x1 - x0) * i as f64 / n as f64;
                [x, amplitude_m * libm::sin(k * x)]
            })
            .collect()
    };
    let start_line = sine(0.0, length_m);
    let centreline = sine(-RUN_ON_M, length_m + RUN_ON_M);
    let river = river_of(&centreline, 0.5 * width_m);
    let r = crate::geom::overlay::PolygonIndex::new(&river);
    let rough = relief(7, 1.5);
    WindowFixture {
        start_line,
        centreline,
        river,
        ground: Box::new(move |x, y| {
            if r.contains_xy([x, y]) {
                BED_M
            } else {
                BANK_M + 1.0 + rough(x, y)
            }
        }),
    }
}
