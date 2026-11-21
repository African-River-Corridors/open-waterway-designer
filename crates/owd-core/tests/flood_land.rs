//! Flooded land per pool, bank-full, and the flood terms of the weir search (ticket OPT-2), on a
//! synthetic straight valley with a known cross-section. No real river is read.
//!
//! The valley runs along x. Across it (y is the distance from the line):
//!
//! ```text
//! |y| <  25 m            river trench: the river polygon; the DEM reads the water surface
//! 25 m <= |y| <= 205 m   flat floodplain, 2 m above the water (the banks stand 2 m)
//! |y| > 205 m            valley sides, rising 1 m in 10 m
//! ```
//!
//! With 10 m cells whose centres are on multiples of 10 m, the edges at 25 m and 205 m fall
//! between cell centres, so the closed form is exact on the grid: a pool at `L` (above the
//! floodplain) floods `2 x (180 + 10 x (L - 2 - water))` metres of width over its length.
//!
//! Labels are on each test.

use owd_core::geom::{Polygon, Pt};
use owd_core::optimise::cascade::{
    Cascade, CascadeConfig, LevelTable, cascade_levels, earthworks_reach_cost, search_cascade,
};
use owd_core::optimise::flood::{
    BANK_PERCENTILE, BankProfile, FloodField, FloodTable, FloodTerms, explain_blocked, percentile,
    with_flood_cost,
};

const LENGTH: f64 = 4000.0;
const CELL: f64 = 10.0;
const DIST: f64 = 500.0;
const BANK: f64 = 2.0;

fn line() -> Vec<Pt> {
    vec![[0.0, 0.0], [LENGTH, 0.0]]
}

/// The river polygon: 25 m each side of the line, and past both ends of it.
fn river() -> Vec<Polygon> {
    vec![Polygon {
        exterior: vec![
            [-100.0, -25.0],
            [LENGTH + 100.0, -25.0],
            [LENGTH + 100.0, 25.0],
            [-100.0, 25.0],
            [-100.0, -25.0],
        ],
        holes: vec![],
    }]
}

/// The valley's ground at one point, on a water surface that climbs `slope` per metre of x.
fn valley_at(x: f64, y: f64, slope: f64) -> f64 {
    let water = slope * x;
    let a = y.abs();
    if a < 25.0 {
        water
    } else if a <= 205.0 {
        water + BANK
    } else {
        water + BANK + (a - 205.0) / 10.0
    }
}

fn flat(xs: &[f64], ys: &[f64]) -> Vec<f64> {
    xs.iter()
        .zip(ys)
        .map(|(&x, &y)| valley_at(x, y, 0.0))
        .collect()
}

/// The flat valley with a hollow in the valley side: 0.5 m ground, 100 m by 40 m, with ground of
/// 13.5 m and more all round it.
fn with_hollow(xs: &[f64], ys: &[f64]) -> Vec<f64> {
    xs.iter()
        .zip(ys)
        .map(|(&x, &y)| {
            if (1400.0..=1500.0).contains(&x) && (330.0..=370.0).contains(&y) {
                0.5
            } else {
                valley_at(x, y, 0.0)
            }
        })
        .collect()
}

const SLOPE: f64 = 0.002;

fn sloped(xs: &[f64], ys: &[f64]) -> Vec<f64> {
    xs.iter()
        .zip(ys)
        .map(|(&x, &y)| valley_at(x, y, SLOPE))
        .collect()
}

fn field(terrain: owd_core::types::TerrainSampler) -> FloodField {
    FloodField::build(&line(), terrain, &river(), DIST, CELL).unwrap()
}

/// The closed form on the flat valley: the flooded land of a pool at `level` over `length`.
fn closed_form_m2(level: f64, length: f64) -> f64 {
    if level <= BANK {
        0.0
    } else {
        2.0 * (180.0 + 10.0 * (level - BANK)) * length
    }
}

/// Flooded area per level matches the closed-form width times length. Stated tolerance: one cell
/// of width on each side of the river (2 x 10 m x the length). At whole metres of level the grid
/// is laid so the closed form is exact, and the test asks for that.
///
/// Label: property (closed form)
#[test]
fn flooded_area_per_level_matches_width_times_length() {
    let f = field(&flat);
    for level in [1.0, 2.0, 2.5, 3.0, 3.7, 4.0, 5.0, 12.0] {
        let got = f.flooded(1000.0, 2000.0, level);
        let want = closed_form_m2(level, 1000.0);
        let tolerance = if level.fract() == 0.0 {
            0.0
        } else {
            2.0 * CELL * 1000.0
        };
        assert!(
            (got.area_m2 - want).abs() <= tolerance,
            "level {level}: {} m2, closed form {want} m2",
            got.area_m2
        );
        assert!(!got.reaches_limit, "level {level} is far from the edge");
    }
    // Ground at the level is not below it: a pool exactly at the floodplain floods none of it.
    assert_eq!(f.flooded(0.0, LENGTH, BANK).area_m2, 0.0);
}

/// A hollow not joined to the river stays dry until the water tops its rim; ground inside the
/// river polygon is never counted.
///
/// Label: property
#[test]
fn a_hollow_off_the_river_stays_dry_and_the_river_is_never_counted() {
    let f = field(&with_hollow);
    // At 5 m the hollow (0.5 m ground, rim 13.5 m) is below the pool but not joined: dry.
    let got = f.flooded(1000.0, 2000.0, 5.0);
    assert_eq!(got.area_m2, closed_form_m2(5.0, 1000.0));
    let wet_hollow = |cells: &[Pt]| {
        cells
            .iter()
            .filter(|p| (1400.0..=1500.0).contains(&p[0]) && (330.0..=370.0).contains(&p[1]))
            .count()
    };
    assert_eq!(wet_hollow(&f.flooded_cells(1000.0, 2000.0, 5.0, false)), 0);
    // Its fill level is its rim, not its own ground.
    let hollow = f
        .cells
        .iter()
        .find(|c| {
            let p = f.centre(c);
            (p[0] - 1450.0).abs() < 1e-6 && (p[1] - 350.0).abs() < 1e-6
        })
        .unwrap();
    assert_eq!(hollow.ground_m, 0.5);
    assert_eq!(hollow.fill_m, 13.5);
    assert_eq!(wet_hollow(&f.flooded_cells(1000.0, 2000.0, 13.5, false)), 0);
    // Once the water tops the rim the hollow floods: 11 by 5 cells.
    assert_eq!(
        wet_hollow(&f.flooded_cells(1000.0, 2000.0, 20.0, false)),
        55
    );

    // No flooded cell is in the river polygon, at any level, and the river cells are a layer of
    // their own.
    for level in [3.0, 20.0, 1000.0] {
        let land = f.flooded_cells(0.0, LENGTH, level, false);
        assert!(land.iter().all(|p| p[1].abs() >= 25.0), "level {level}");
        let all = f.flooded_cells(0.0, LENGTH, level, true);
        let river_cells = all.iter().filter(|p| p[1].abs() < 25.0).count();
        assert_eq!(river_cells, 400 * 5, "level {level}");
        assert_eq!(all.len(), land.len() + river_cells);
    }
}

/// The ends of a reach and the edge of the grid. Ground is in the reach that holds its nearest
/// chainage, so two reaches side by side flood what the two flood as one. Ground beyond the ends
/// of the line is in no reach. A pool that floods out to the distance limit is flagged.
///
/// Label: property
#[test]
fn reach_ends_add_up_and_a_flood_at_the_grid_edge_is_flagged() {
    let f = field(&flat);
    let whole = f.flooded(0.0, LENGTH, 3.0);
    let (a, b) = (f.flooded(0.0, 1500.0, 3.0), f.flooded(1500.0, LENGTH, 3.0));
    assert_eq!(a.area_m2 + b.area_m2, whole.area_m2);
    assert_eq!(whole.area_m2, closed_form_m2(3.0, LENGTH));
    // The field holds ground beyond both ends (it is within the distance of an end point), but no
    // reach counts it, however wide the reach is asked.
    assert!(f.cells.iter().any(|c| c.beyond_end));
    let wide = f.flooded(-1.0e9, LENGTH, 3.0);
    assert_eq!(wide.area_m2, whole.area_m2);
    // 31.5 m of ground at the 500 m limit: a pool above it floods to the edge and says so.
    assert!(!f.flooded(0.0, LENGTH, 30.0).reaches_limit);
    let over = f.flooded(0.0, LENGTH, 40.0);
    assert!(over.reaches_limit);
    // Every land cell within the distance, and no more: the area stops at the limit.
    assert_eq!(over.area_m2, 2.0 * (500.0 - 20.0) * LENGTH);

    // A DEM with a hole (no value) is an edge too: the flood stops there and is flagged.
    let holed = |xs: &[f64], ys: &[f64]| -> Vec<f64> {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| {
                if y > 100.0 {
                    f64::NAN
                } else {
                    valley_at(x, y, 0.0)
                }
            })
            .collect()
    };
    let h = field(&holed);
    let got = h.flooded(0.0, LENGTH, 3.0);
    assert!(got.reaches_limit);
    // South side whole (190 m), north side to the hole (30 m to 100 m: 8 cells).
    assert_eq!(got.area_m2, (190.0 + 80.0) * LENGTH);
}

/// The table the search reads gives the field's own area for every (break, break, level), and
/// nothing for an end or a level it does not hold.
///
/// Label: property
#[test]
fn the_flood_table_equals_the_field() {
    let f = field(&sloped);
    let breaks: Vec<f64> = (0..=8).map(|k| 500.0 * f64::from(k)).collect();
    let levels: Vec<f64> = (0..=12).map(f64::from).collect();
    let t = FloodTable::new(&f, &breaks, &levels);
    let mut some = 0.0;
    for (i, &a) in breaks.iter().enumerate() {
        for &b in &breaks[i..] {
            for &l in &levels {
                let want = f.flooded(a, b, l).area_m2;
                assert_eq!(t.area_m2(a, b, l), Some(want), "{a} {b} {l}");
                some += want;
            }
        }
    }
    assert!(some > 0.0);
    assert_eq!(t.area_m2(250.0, 1000.0, 3.0), None);
    assert_eq!(t.area_m2(0.0, 1000.0, 3.5), None);
}

/// Bank-full is the 25th percentile of the ground in the cells that touch the river polygon, on
/// each side, in each 500 m of line, and the lower of the two sides. The DEM river level is the
/// median of the river cells.
///
/// Label: property (the stated definition)
#[test]
fn bank_full_is_the_stated_percentile_of_the_ground_beside_the_polygon() {
    // Flat: every bank cell is 2 m, the river 0 m.
    let f = field(&flat);
    let b = BankProfile::new(&f);
    assert!(b.known());
    assert_eq!(b.bank_m.len(), 8);
    assert!(b.measured.iter().all(|m| *m));
    for k in 0..8 {
        assert_eq!(b.bank_m[k], BANK);
        assert_eq!(b.river_m[k], 0.0);
    }
    let e = b.at_ends(1000.0, 2000.0, 5.0);
    assert_eq!(e.above_bank_lower_m, 3.0);
    assert_eq!(e.above_bank_upper_m, 3.0);
    assert_eq!(e.max_above_bank_m, 3.0);
    assert_eq!(e.depth_over_river_lower_m, 5.0);
    assert_eq!(e.depth_over_river_upper_m, 5.0);

    // Sloped: the bank cells of a window are the cells at |y| = 30 m, x = 0, 10, ... in the
    // window, at 2 m above the water there. The two sides are the same, so the lower is either.
    let f = field(&sloped);
    let b = BankProfile::new(&f);
    for k in 0..8usize {
        // The last window also holds the column at the end of the line (x = 4000 m).
        let n = if k == 7 { 51 } else { 50 };
        let water: Vec<f64> = (0..n)
            .map(|i| SLOPE * (500.0 * k as f64 + 10.0 * f64::from(i)))
            .collect();
        let mut want: Vec<f64> = water.iter().map(|w| w + BANK).collect();
        let want = percentile(&mut want, BANK_PERCENTILE);
        assert!((b.bank_m[k] - want).abs() < 1e-9, "window {k}");
        // The river cells: five rows of the same x, so the median of the water levels.
        let mid = percentile(&mut water.clone(), 50.0);
        assert!((b.river_m[k] - mid).abs() < 1e-9, "window {k}");
    }
    // A low bank on one side only lowers bank-full: it is the lower side that counts.
    let low_south = |xs: &[f64], ys: &[f64]| -> Vec<f64> {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| {
                if (-35.0..-25.0).contains(&y) {
                    1.0
                } else {
                    valley_at(x, y, 0.0)
                }
            })
            .collect()
    };
    let b = BankProfile::new(&field(&low_south));
    assert_eq!(b.bank_m[3], 1.0);
    // The pool's worst place is its lowest bank: a reach is read over every window it touches.
    assert_eq!(b.max_above_bank(0.0, LENGTH, 4.0), 3.0);
    let b = BankProfile::new(&f);
    assert!(b.min_bank(1000.0, 3000.0) < b.bank_at(2999.0));
    assert_eq!(b.min_bank(1000.0, 3000.0), b.bank_at(1000.0));
    // A reach that ends on a window's edge does not read the next window.
    assert_eq!(b.min_bank(1000.0, 1500.0), b.bank_m[2]);
    assert_eq!(
        b.at_ends(1000.0, 1500.0, 9.0).above_bank_upper_m,
        9.0 - b.bank_m[2]
    );
}

/// On a line with a bend the nearest point of the line gives each cell its chainage and distance:
/// equal to a search of every segment.
///
/// Label: property (brute force)
#[test]
fn every_cell_has_the_distance_and_chainage_of_its_nearest_point() {
    let line: Vec<Pt> = vec![[0.0, 0.0], [600.0, 0.0], [600.0, 500.0], [1200.0, 500.0]];
    let flat0 = |xs: &[f64], _: &[f64]| vec![0.0; xs.len()];
    let f = FloodField::build(&line, &flat0, &[], 200.0, 10.0).unwrap();
    assert_eq!(f.length_m, 1700.0);
    let mut worst: f64 = 0.0;
    for c in &f.cells {
        let p = f.centre(c);
        let mut best = (f64::INFINITY, 0.0);
        let mut run = 0.0;
        for w in line.windows(2) {
            let (a, b) = (w[0], w[1]);
            let len = libm::hypot(b[0] - a[0], b[1] - a[1]);
            let t = (((p[0] - a[0]) * (b[0] - a[0]) + (p[1] - a[1]) * (b[1] - a[1])) / (len * len))
                .clamp(0.0, 1.0);
            let q = [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])];
            let d = libm::hypot(p[0] - q[0], p[1] - q[1]);
            if d < best.0 - 1e-9 {
                best = (d, run + t * len);
            }
            run += len;
        }
        assert!(c.distance_m <= 200.0 + 1e-9);
        worst = worst.max((c.distance_m - best.0).abs());
        // Where two segments are as near as each other the chainage may be either's.
        if (c.distance_m - best.0).abs() < 1e-9 && !c.beyond_end {
            let tie = (c.chainage_m - best.1).abs() > 1e-6;
            if tie {
                // Only the bisectors of the two corners may differ.
                let on_bisector = ((p[0] - 600.0).abs() - p[1].abs()).abs() < 1e-6
                    || ((p[0] - 600.0).abs() - (p[1] - 500.0).abs()).abs() < 1e-6;
                assert!(on_bisector, "{p:?}: {} against {}", c.chainage_m, best.1);
            }
        }
    }
    assert!(worst < 1e-9, "worst distance error {worst} m");
    // Every cell within the distance is there: the count is the area of the 200 m band.
    let inside = f.cells.len() as f64 * 100.0;
    let band = 1700.0 * 400.0 + std::f64::consts::PI * 200.0 * 200.0;
    assert!(
        (inside - band).abs() < 0.02 * band,
        "{inside} against {band}"
    );
}

/// A field too large to hold is refused with the cell size that fits.
///
/// Label: property
#[test]
fn a_field_too_large_is_refused_with_the_cell_that_fits() {
    let long: Vec<Pt> = vec![[0.0, 0.0], [240_000.0, 0.0]];
    let e = FloodField::build(&long, &flat, &[], 1500.0, 2.0).unwrap_err();
    assert!(
        e.contains("flood_cell_m") && e.contains("at least 6 m"),
        "{e}"
    );
}

// ----- the weir search on the sloped valley -----

/// A cut cost that falls with every metre of pool level, as a real one does: per metre of line,
/// `4000 x exp(-(level - water) / 2)`, summed at 10 m steps.
fn cut(a: f64, b: f64, level: f64) -> Result<f64, String> {
    let mut sum = 0.0;
    let mut x = a;
    while x < b - 1e-9 {
        sum += 4000.0 * libm::exp(-(level - SLOPE * x) / 2.0) * 10.0;
        x += 10.0;
    }
    Ok(sum)
}

fn structure(_lift: f64) -> f64 {
    1.0e6
}

/// Sites every 500 m, lifts of 1, 2 and 4 m, from 0 m to a top pool of at least 8 m (the water
/// climbs 8 m over the 4 km).
fn config(top: f64) -> CascadeConfig {
    let sites: Vec<f64> = (1..8).map(|k| 500.0 * f64::from(k)).collect();
    CascadeConfig::new(sites, vec![1.0, 2.0, 4.0], LENGTH, top)
        .validated()
        .unwrap()
}

struct Ground {
    field: FloodField,
    table: FloodTable,
    banks: BankProfile,
}

fn ground(cfg: &CascadeConfig) -> Ground {
    let field = field(&sloped);
    let mut breaks = vec![cfg.start_m, cfg.terminal_m];
    breaks.extend(cfg.candidate_sites_m.iter().copied());
    let table = FloodTable::new(&field, &breaks, &cascade_levels(cfg));
    let banks = BankProfile::new(&field);
    Ground {
        field,
        table,
        banks,
    }
}

fn run(cfg: &CascadeConfig, g: &Ground, cost_per_ha: f64, limit: Option<f64>) -> Option<Cascade> {
    let terms = FloodTerms {
        field: &g.field,
        table: &g.table,
        banks: &g.banks,
        cost_per_ha,
        max_above_bank_m: limit,
    };
    let reach = with_flood_cost(&cut, terms);
    search_cascade(cfg, &reach, &structure)
        .unwrap()
        .best()
        .cloned()
}

fn flooded_ha(g: &Ground, c: &Cascade) -> f64 {
    c.pools()
        .iter()
        .map(|&(a, b, l)| g.field.flooded(a, b, l).area_ha())
        .sum()
}

fn max_lift(c: &Cascade) -> f64 {
    c.weirs.iter().map(|w| w.lift_m).fold(0.0, f64::max)
}

/// At flood cost 0 the search takes the largest lift at every weir (today's behaviour, pinned):
/// the terms change nothing, and the result is the search with the cut cost alone.
///
/// Label: property
#[test]
fn at_flood_cost_zero_the_search_takes_the_largest_lift() {
    let cfg = config(8.0);
    let g = ground(&cfg);
    let plain = search_cascade(&cfg, &cut, &structure)
        .unwrap()
        .best()
        .cloned()
        .unwrap();
    let zero = run(&cfg, &g, 0.0, None).unwrap();
    assert_eq!(zero, plain);
    assert!(!plain.weirs.is_empty());
    for w in &plain.weirs {
        assert_eq!(w.lift_m, 4.0, "{:?}", plain.weirs);
    }
    // And it floods: its pools stand well above the 2 m banks.
    assert!(flooded_ha(&g, &plain) > 50.0);
}

/// As the flood cost rises the chosen lifts fall and the flooded land does not rise. Three values
/// are swept above 0. The total cost is the cut, the structures and the flood cost.
///
/// Label: property
#[test]
fn a_rising_flood_cost_lowers_the_lifts_and_never_raises_the_flooded_land() {
    let cfg = config(8.0);
    let g = ground(&cfg);
    let mut last_ha = f64::INFINITY;
    let mut last_lift = f64::INFINITY;
    let mut rows = Vec::new();
    for cost in [0.0, 50_000.0, 100_000.0, 200_000.0, 300_000.0] {
        let c = run(&cfg, &g, cost, None).unwrap();
        let ha = flooded_ha(&g, &c);
        let lift = max_lift(&c);
        rows.push((cost, c.weirs.len(), lift, ha, c.total_cost));
        assert!(ha <= last_ha + 1e-9, "{rows:?}");
        assert!(lift <= last_lift, "{rows:?}");
        // The total is the cut, the structures and the flood cost of the pools.
        let cut_sum: f64 = c
            .pools()
            .iter()
            .map(|&(a, b, l)| cut(a, b, l).unwrap())
            .sum();
        let want = cut_sum + 1.0e6 * c.weirs.len() as f64 + cost * ha;
        assert!((c.total_cost - want).abs() < 1e-6 * want, "{rows:?}");
        last_ha = ha;
        last_lift = lift;
    }
    println!("flood cost per ha, weirs, largest lift, flooded ha, total cost: {rows:?}");
    // The sweep changes the answer: the top value floods less with smaller lifts than 0 does.
    assert!(rows[4].3 < rows[0].3, "{rows:?}");
    assert!(rows[4].2 < rows[0].2, "{rows:?}");
}

/// With `max_above_bank_m` 1.0 no pool stands more than 1.0 m above bank-full anywhere in its
/// reach, and the search still reaches the top level.
///
/// Label: property
#[test]
fn with_a_bank_limit_no_pool_stands_above_it() {
    let cfg = config(8.0);
    let g = ground(&cfg);
    let free = run(&cfg, &g, 0.0, None).unwrap();
    let worst = |c: &Cascade| {
        c.pools()
            .iter()
            .map(|&(a, b, l)| g.banks.max_above_bank(a, b, l))
            .fold(f64::NEG_INFINITY, f64::max)
    };
    assert!(worst(&free) > 1.0, "the free search breaks the limit");
    let held = run(&cfg, &g, 0.0, Some(1.0)).unwrap();
    assert!(worst(&held) <= 1.0 + 1e-9, "{}", worst(&held));
    assert!(held.pools().last().unwrap().2 >= 8.0);
    assert!(held.total_cost.is_finite());
    assert!(held.total_cost > free.total_cost);
    assert!(flooded_ha(&g, &held) < flooded_ha(&g, &free));
}

/// An impossible limit fails with a plain message: which reach blocks it, and what would unblock
/// it. The search gives no cascade, never a fallback.
///
/// Label: property
#[test]
fn an_impossible_limit_names_the_reach_that_blocks_and_what_unblocks_it() {
    // A top pool of 14 m where the highest bank is about 9.9 m: no pool may hold it within 1 m.
    let cfg = config(14.0);
    let g = ground(&cfg);
    assert!(run(&cfg, &g, 0.0, Some(1.0)).is_none());
    assert!(run(&cfg, &g, 0.0, None).is_some());
    let explain = |cfg: &CascadeConfig, limit: f64| {
        let mut sites = cfg.candidate_sites_m.clone();
        sites.sort_by(|a, b| a.partial_cmp(b).unwrap());
        explain_blocked(
            &g.banks,
            limit,
            cfg.start_m,
            cfg.base_level_m,
            &sites,
            &cfg.lifts_m,
            &cascade_levels(cfg),
            cfg.terminal_m,
            cfg.terminal_min_level_m,
            cfg.min_spacing_m,
        )
    };
    let m = explain(&cfg, 1.0);
    println!("{m}");
    assert!(m.starts_with("No cascade reaches the top level of 14.00 m with every pool at most 1.00 m above bank-full"), "{m}");
    assert!(
        m.contains("The reach from ") && m.contains(" blocks it"),
        "{m}"
    );
    assert!(m.contains("above bank-full there"), "{m}");
    assert!(
        m.contains("To unblock it: give more candidate sites"),
        "{m}"
    );
    assert!(m.contains("allow smaller lifts (`--lifts`)"), "{m}");
    assert!(m.contains("raise `max_above_bank_m` to at least "), "{m}");

    // Sites too few for the lift: one site at 500 m and a 6 m lift only. The weir there must hold
    // 6 m, about 2.8 m over the bank just above it.
    let mut few = CascadeConfig::new(vec![500.0], vec![6.0], LENGTH, 6.0)
        .validated()
        .unwrap();
    few.k_best = 1;
    let gf = ground(&few);
    assert!(run(&few, &gf, 0.0, Some(1.0)).is_none());
    let m = explain(&few, 1.0);
    println!("{m}");
    assert!(
        m.contains("The reach from 500 m to 4000 m blocks it"),
        "{m}"
    );
    assert!(m.contains("the top pool, at 6.00 m"), "{m}");
    // The limit the message names does unblock it.
    let need: f64 = m
        .rsplit("at least ")
        .next()
        .unwrap()
        .trim_end_matches(" m.")
        .parse()
        .unwrap();
    assert!(run(&few, &gf, 0.0, Some(need + 0.005)).is_some(), "{need}");
}

/// The reach cost built once on the table's cumulative volumes is the cost read from
/// `reach_volumes` each time: the same numbers, bit for bit.
///
/// Label: property
#[test]
fn the_cached_reach_cost_is_reach_volumes_bit_for_bit() {
    let n = 40;
    let ch: Vec<f64> = (0..n).rev().map(|i| 25.0 * f64::from(i)).collect();
    let levels = vec![0.0, 0.5, 1.0, 1.5];
    let area = |i: usize, k: usize, s: f64| {
        s * (1.0 + libm::sin(i as f64 * 0.37).abs()) / (1.0 + k as f64)
    };
    let dredge: Vec<Vec<f64>> = (0..n as usize)
        .map(|i| (0..4).map(|k| area(i, k, 31.7)).collect())
        .collect();
    let excav: Vec<Vec<f64>> = (0..n as usize)
        .map(|i| (0..4).map(|k| area(i, k, 113.3)).collect())
        .collect();
    let t = LevelTable::new(ch, levels.clone(), dredge, excav, 25.0);
    let cost = earthworks_reach_cost(&t, 1.3, 2.9);
    for a in [0.0, 100.0, 512.5] {
        for b in [100.0, 700.0, 1000.0, 5000.0] {
            for &l in &levels {
                let (d, e) = t.reach_volumes(a, b, l).unwrap();
                assert_eq!(
                    cost(a, b, l).unwrap().to_bits(),
                    (1.3 * d + 2.9 * e).to_bits()
                );
            }
        }
    }
    assert!(
        cost(0.0, 100.0, 0.25).is_err(),
        "a level off the table is refused"
    );
}

/// A wide lift menu on a long river: 240 km, a site every 500 m (479 sites), lifts of 1 to 12 m in
/// 1 m steps, 25 m flood cells out to 1,500 m. Prints the time of each part. Run with
/// `cargo test --release -p owd-core --test flood_land -- --ignored --nocapture`.
///
/// Label: property (prints times; checks a cascade is found and the limit is held)
#[test]
#[ignore = "benchmark: run on request"]
fn a_240_km_river_with_lifts_of_1_to_12_m() {
    let len = 240_000.0;
    let slope = 60.0 / len;
    // A valley like the one above, 60 m of fall, with banks that vary along it.
    let terrain = |xs: &[f64], ys: &[f64]| -> Vec<f64> {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| {
                let water = slope * x;
                let a = y.abs();
                let bank = 2.5 + 0.5 * libm::sin(x / 3000.0);
                if a < 40.0 {
                    water
                } else if a <= 600.0 + 300.0 * libm::sin(x / 7000.0) {
                    water + bank + 0.3 * libm::sin(a / 90.0)
                } else {
                    water + bank + (a - 600.0 - 300.0 * libm::sin(x / 7000.0)) / 8.0
                }
            })
            .collect()
    };
    let line: Vec<Pt> = (0..=2400).map(|i| [100.0 * f64::from(i), 0.0]).collect();
    let mut ring: Vec<Pt> = (0..=2400).map(|i| [100.0 * f64::from(i), -40.0]).collect();
    ring.extend((0..=2400).rev().map(|i| [100.0 * f64::from(i), 40.0]));
    ring.push(ring[0]);
    let river = vec![Polygon {
        exterior: ring,
        holes: vec![],
    }];
    let sites: Vec<f64> = (1..480).map(|k| 500.0 * f64::from(k)).collect();
    let lifts: Vec<f64> = (1..=12).map(f64::from).collect();
    let cfg = CascadeConfig::new(sites, lifts, len, 60.0)
        .validated()
        .unwrap();
    let levels = cascade_levels(&cfg);

    let t = std::time::Instant::now();
    let field = FloodField::build(&line, &terrain, &river, 1500.0, 25.0).unwrap();
    let t_field = t.elapsed().as_secs_f64();
    let t = std::time::Instant::now();
    let banks = BankProfile::new(&field);
    let mut breaks = vec![cfg.start_m, cfg.terminal_m];
    breaks.extend(cfg.candidate_sites_m.iter().copied());
    let table = FloodTable::new(&field, &breaks, &levels);
    let t_table = t.elapsed().as_secs_f64();
    println!(
        "flood field: {} cells in {t_field:.2} s; bank profile and flood table ({} breaks x {} levels): {t_table:.2} s",
        field.cells.len(),
        breaks.len(),
        levels.len()
    );

    // The cut table the stage builds: `section_volumes` once a level, on 25 m stations.
    let t = std::time::Instant::now();
    let few: Vec<f64> = levels.iter().copied().take(4).collect();
    let lt = owd_core::optimise::cascade::build_level_table(
        &line,
        &terrain,
        &river,
        &few,
        3.0,
        &owd_core::volumes::sections::HalfWidth::Fixed(30.0),
        None,
        Some(25.0),
        Some(200.0),
        None,
        owd_core::optimise::cascade::SectionOptions::default(),
    )
    .unwrap();
    let per_level = t.elapsed().as_secs_f64() / 4.0;
    println!(
        "cut table: {per_level:.2} s a level on {} stations, so about {:.0} s for the {} levels",
        lt.chainage_m.len(),
        per_level * levels.len() as f64,
        levels.len()
    );

    // An analytic cut cost through a cumulative table, as cheap a read as `earthworks_reach_cost`.
    let n_st = (len / 25.0) as usize;
    let mut cum = vec![vec![0.0f64; levels.len()]; n_st + 1];
    for i in 0..n_st {
        let water = slope * 25.0 * i as f64;
        for (k, &l) in levels.iter().enumerate() {
            let depth = (water + 3.0 - l).max(0.0);
            cum[i + 1][k] = cum[i][k]
                + 25.0 * (60.0 * depth + 3.0 * depth * depth)
                + 25.0 * 5.0 * libm::exp(-(l - water) / 4.0);
        }
    }
    let li: std::collections::HashMap<u64, usize> = levels
        .iter()
        .enumerate()
        .map(|(i, v)| (v.to_bits(), i))
        .collect();
    let cut = |a: f64, b: f64, l: f64| -> Result<f64, String> {
        let k = li[&l.to_bits()];
        Ok(cum[(b / 25.0) as usize][k] - cum[(a / 25.0) as usize][k])
    };
    let structure = |lift: f64| 2.5e6 + 6.5e5 * lift;
    for (cost, limit) in [
        (0.0, None),
        (100.0, None),
        (300.0, None),
        (1000.0, None),
        (3000.0, None),
        (10_000.0, None),
        (30_000.0, None),
        (3000.0, Some(1.0)),
    ] {
        let terms = FloodTerms {
            field: &field,
            table: &table,
            banks: &banks,
            cost_per_ha: cost,
            max_above_bank_m: limit,
        };
        let reach = with_flood_cost(&cut, terms);
        let t = std::time::Instant::now();
        let res = search_cascade(&cfg, &reach, &structure).unwrap();
        let dt = t.elapsed().as_secs_f64();
        let best = res.best().expect("a cascade");
        let ha: f64 = best
            .pools()
            .iter()
            .map(|&(a, b, l)| field.flooded(a, b, l).area_ha())
            .sum();
        let worst = best
            .pools()
            .iter()
            .map(|&(a, b, l)| banks.max_above_bank(a, b, l))
            .fold(f64::NEG_INFINITY, f64::max);
        println!(
            "search, flood cost {cost}/ha, limit {limit:?}: {dt:.2} s, {} states, {} weirs, largest lift {} m, {ha:.0} ha flooded, worst {worst:.2} m above bank-full, total {:.0}",
            res.states_explored,
            best.weirs.len(),
            max_lift(best),
            best.total_cost
        );
        if let Some(l) = limit {
            assert!(worst <= l + 1e-9);
        }
    }
}
