//! Buildings a pool floods (BLD-1), counted by the weir search's own flood rule, and the building
//! cost in the search. On a synthetic straight valley (the valley of `flood_land.rs`): no real
//! river or building is read.
//!
//! The valley runs along x. Across it (y is the distance from the line): the river polygon for
//! `|y| < 25 m`, a flat floodplain 2 m above the water out to 205 m, then valley sides rising 1 m
//! in 10 m. A row of buildings stands on the valley side at known ground levels.

use owd_core::geom::{Polygon, Pt};
use owd_core::optimise::buildings::{
    BuildingField, BuildingTable, BuildingTerms, with_building_cost,
};
use owd_core::optimise::cascade::{CascadeConfig, cascade_levels, search_cascade};
use owd_core::optimise::flood::{BankProfile, FloodField, FloodTable, FloodTerms, with_flood_cost};

const LENGTH: f64 = 4000.0;
const CELL: f64 = 10.0;
const DIST: f64 = 600.0;

fn line() -> Vec<Pt> {
    vec![[0.0, 0.0], [LENGTH, 0.0]]
}

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

fn valley_at(x: f64, y: f64, slope: f64) -> f64 {
    let water = slope * x;
    let a = y.abs();
    if a < 25.0 {
        water
    } else if a <= 205.0 {
        water + 2.0
    } else {
        water + 2.0 + (a - 205.0) / 10.0
    }
}

/// The flat valley with a hollow at x 1400..1500, y 330..370: ground 0.5 m, rimmed by 14 m and more.
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

/// A row of buildings on the valley side at x = 1000, y = 220, 230, ..., 310 (cell centres): ground
/// 3.5, 4.5, ..., 12.5 m; one in the hollow; and one at ground 5.5 m with a confidence below the
/// threshold.
fn row() -> (Vec<Pt>, Vec<bool>) {
    let mut pts: Vec<Pt> = (0..10).map(|k| [1000.0, 220.0 + 10.0 * k as f64]).collect();
    pts.push([1450.0, 350.0]);
    pts.push([2000.0, -240.0]);
    let mut counted = vec![true; pts.len()];
    counted[11] = false;
    (pts, counted)
}

#[test]
fn the_count_rises_with_the_level_as_the_ground_says_and_a_hollow_stays_dry() {
    // Label: property
    let field = FloodField::build(&line(), &with_hollow, &river(), DIST, CELL).unwrap();
    let (pts, counted) = row();
    let (xs, ys): (Vec<f64>, Vec<f64>) = pts.iter().map(|p| (p[0], p[1])).unzip();
    let ground = with_hollow(&xs, &ys);
    let bf = BuildingField::new(&field, &pts, &ground, 0.3).unwrap();
    let levels: Vec<f64> = (1..=30).map(|k| 0.5 * f64::from(k)).collect();
    let table = BuildingTable::new(&bf, &|i| counted[i], &[0.0, LENGTH], &levels);
    for &l in &levels {
        // The row: ground 3.5..12.5 m, flooded where the water is at least 0.3 m deep.
        // The hollow's building (ground 0.5 m) is dry until the water tops its rim (13.5 m).
        let want = (0..10)
            .filter(|k| l >= 3.5 + *k as f64 + 0.3 - 1e-9)
            .count()
            + usize::from(l > 13.5);
        assert_eq!(table.count(0.0, LENGTH, l), Some(want as u32), "level {l}");
        // The field's own count agrees with the table.
        let direct = bf.flooded(0.0, LENGTH, l, &|i| counted[i]);
        assert_eq!(direct.len(), want, "level {l}");
        assert_eq!(direct.contains(&10), l > 13.5, "level {l}");
        // The building below the confidence threshold floods from 5.8 m, but is never counted.
        let all = bf.flooded(0.0, LENGTH, l, &|_| true);
        assert_eq!(all.contains(&11), l >= 5.8 - 1e-9, "level {l}");
    }
    // A puddle does not count: at 3.7 m the first building (ground 3.5 m) is wet but 0.2 m deep.
    assert_eq!(table.count(0.0, LENGTH, 3.5), Some(0));
    assert!(!bf.buildings[0].flooded_at(3.7) && bf.buildings[0].flooded_at(3.8));
    // Reach ends: the row is at chainage 1000, so a reach that ends before it counts none.
    let t2 = BuildingTable::new(&bf, &|i| counted[i], &[0.0, 999.0, 1001.0, LENGTH], &levels);
    assert_eq!(t2.count(0.0, 999.0, 15.0), Some(0));
    assert_eq!(t2.count(999.0, 1001.0, 15.0), Some(10));
    assert!(BuildingField::new(&field, &pts, &ground[..3], 0.3).is_err());
}

const SLOPE: f64 = 0.002;
fn sloped(xs: &[f64], ys: &[f64]) -> Vec<f64> {
    xs.iter()
        .zip(ys)
        .map(|(&x, &y)| valley_at(x, y, SLOPE))
        .collect()
}

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

/// Buildings along the whole valley side, every 50 m of x, at 215 m and 245 m each side: ground
/// 3 m and 6 m above the water there.
fn valley_buildings() -> Vec<Pt> {
    let mut pts = Vec::new();
    for k in 0..80 {
        let x = 25.0 + 50.0 * f64::from(k);
        for y in [215.0, 245.0, -215.0, -245.0] {
            pts.push([x, y]);
        }
    }
    pts
}

#[test]
fn a_rising_building_cost_never_floods_more_buildings_and_at_zero_the_search_is_unchanged() {
    // Label: property
    let sites: Vec<f64> = (1..8).map(|k| 500.0 * f64::from(k)).collect();
    let cfg = CascadeConfig::new(sites, vec![1.0, 2.0, 4.0], LENGTH, 8.0)
        .validated()
        .unwrap();
    let field = FloodField::build(&line(), &sloped, &river(), DIST, CELL).unwrap();
    let mut breaks = vec![cfg.start_m, cfg.terminal_m];
    breaks.extend(cfg.candidate_sites_m.iter().copied());
    let levels = cascade_levels(&cfg);
    let flood_table = FloodTable::new(&field, &breaks, &levels);
    let banks = BankProfile::new(&field);
    let pts = valley_buildings();
    let (xs, ys): (Vec<f64>, Vec<f64>) = pts.iter().map(|p| (p[0], p[1])).unzip();
    let bf = BuildingField::new(&field, &pts, &sloped(&xs, &ys), 0.3).unwrap();
    let all = |_: usize| true;
    let table = BuildingTable::new(&bf, &all, &breaks, &levels);
    let flood = FloodTerms {
        field: &field,
        table: &flood_table,
        banks: &banks,
        cost_per_ha: 0.0,
        max_above_bank_m: None,
    };
    let flooded = with_flood_cost(&cut, flood);
    let plain = search_cascade(&cfg, &flooded, &structure).unwrap();
    let count = |c: &owd_core::optimise::cascade::Cascade| -> usize {
        c.pools()
            .iter()
            .map(|&(a, b, l)| bf.flooded(a, b, l, &all).len())
            .sum()
    };
    let mut last = usize::MAX;
    let mut seen = Vec::new();
    for cost in [0.0, 1.0e5, 1.0e6, 1.0e7] {
        let terms = BuildingTerms {
            field: &bf,
            table: &table,
            counted: &all,
            cost_per_building: cost,
        };
        let reach = with_building_cost(&flooded, terms);
        let s = search_cascade(&cfg, &reach, &structure).unwrap();
        let best = s.best().unwrap();
        if cost == 0.0 {
            // At 0 the term adds nothing: the same search, bit for bit.
            assert_eq!(&s, &plain);
        }
        let n = count(best);
        assert!(
            n <= last,
            "cost {cost}: {n} buildings flooded, more than {last} at the lower cost"
        );
        // The table and the field agree on the chosen pools.
        let by_table: usize = best
            .pools()
            .iter()
            .map(|&(a, b, l)| terms.count(a, b, l))
            .sum();
        assert_eq!(by_table, n);
        last = n;
        seen.push(n);
    }
    assert!(
        seen[0] > *seen.last().unwrap(),
        "the cost moves the search: {seen:?}"
    );
}

/// The scale the owner works at: 240 km of line, 40,000 buildings, 479 sites, 73 levels. Prints
/// the time the building field and the count table take, and the search with and without the
/// term. Run with `cargo test --release -p owd-core --test flood_buildings -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement: about a minute in release"]
fn measure_the_count_table_at_240_km() {
    // Label: example
    use std::time::Instant;
    let length = 240_000.0;
    // A gently meandering line, so chainage is not x.
    let line: Vec<Pt> = (0..=2400)
        .map(|k| {
            let x = 100.0 * f64::from(k);
            [x, 800.0 * libm::sin(x / 5000.0)]
        })
        .collect();
    let terrain = |xs: &[f64], ys: &[f64]| -> Vec<f64> {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| {
                let d = (y - 800.0 * libm::sin(x / 5000.0)).abs();
                let water = 0.0002 * x;
                if d < 40.0 {
                    water
                } else {
                    water + 2.0 + (d - 40.0) / 150.0
                }
            })
            .collect()
    };
    let river = vec![Polygon {
        exterior: line
            .iter()
            .map(|p| [p[0], p[1] - 40.0])
            .chain(line.iter().rev().map(|p| [p[0], p[1] + 40.0]))
            .collect(),
        holes: vec![],
    }];
    let t = Instant::now();
    let field = FloodField::build(&line, &terrain, &river, 1500.0, 25.0).unwrap();
    let field_ms = t.elapsed().as_millis();
    let sites: Vec<f64> = (1..480)
        .map(|k| 500.0 * f64::from(k))
        .filter(|c| *c < length)
        .collect();
    let lifts: Vec<f64> = vec![0.5, 1.0];
    let cfg = CascadeConfig::new(sites.clone(), lifts, length, 35.0)
        .validated()
        .unwrap();
    let levels = cascade_levels(&cfg);
    let mut breaks = vec![cfg.start_m, cfg.terminal_m];
    breaks.extend(sites.iter().copied());
    // 40,000 buildings within 1.5 km of the line, spread by a fixed rule.
    let pts: Vec<Pt> = (0..40_000)
        .map(|i| {
            let x = (f64::from(i) * 5.999).rem_euclid(length);
            let off = 60.0 + (f64::from(i) * 37.0).rem_euclid(1400.0);
            let side = if i % 2 == 0 { 1.0 } else { -1.0 };
            [x, 800.0 * libm::sin(x / 5000.0) + side * off]
        })
        .collect();
    let (xs, ys): (Vec<f64>, Vec<f64>) = pts.iter().map(|p| (p[0], p[1])).unzip();
    let ground = terrain(&xs, &ys);
    let t = Instant::now();
    let bf = BuildingField::new(&field, &pts, &ground, 0.3).unwrap();
    let place_ms = t.elapsed().as_secs_f64() * 1000.0;
    let all = |_: usize| true;
    let t = Instant::now();
    let table = BuildingTable::new(&bf, &all, &breaks, &levels);
    let table_ms = t.elapsed().as_secs_f64() * 1000.0;
    let flood_table = FloodTable::new(&field, &breaks, &levels);
    let banks = BankProfile::new(&field);
    let flood = FloodTerms {
        field: &field,
        table: &flood_table,
        banks: &banks,
        cost_per_ha: 0.0,
        max_above_bank_m: None,
    };
    let cutc =
        |a: f64, b: f64, l: f64| -> Result<f64, String> { Ok((b - a) * libm::exp(-l / 10.0)) };
    let flooded = with_flood_cost(&cutc, flood);
    let t = Instant::now();
    let _ = search_cascade(&cfg, &flooded, &|_| 1.0e5).unwrap();
    let plain_ms = t.elapsed().as_secs_f64() * 1000.0;
    let terms = BuildingTerms {
        field: &bf,
        table: &table,
        counted: &all,
        cost_per_building: 1.0e4,
    };
    let reach = with_building_cost(&flooded, terms);
    let t = Instant::now();
    let _ = search_cascade(&cfg, &reach, &|_| 1.0e5).unwrap();
    let with_ms = t.elapsed().as_secs_f64() * 1000.0;
    println!(
        "240 km, {} cells, 40,000 buildings, {} sites, {} levels: flood field {field_ms} ms; buildings placed {place_ms:.1} ms; count table {table_ms:.1} ms; search without the building term {plain_ms:.0} ms, with it {with_ms:.0} ms",
        field.cells.len(),
        sites.len(),
        levels.len()
    );
    assert_eq!(levels.len(), 73);
    assert_eq!(sites.len(), 479);
}
