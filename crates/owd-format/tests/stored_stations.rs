//! Drawings and reports read the volumes stage's stored stations (spec 0017, ticket 10).
//!
//! The web example (`examples/web/river-example.owd`) was saved before the volumes stage stored its
//! stations, so drawing it walks the line again: that is the drawing as it was before this ticket.
//! Running the volumes stage again on the same DEM stores the stations and moves no number; drawing
//! that document reads the stored stations. Every sheet of every kind must be the same, byte for
//! byte: the drawing document, the normalised content, the SVG, the PDF and the DXF.
//!
//! Label: property

use owd_format::design::sections::{
    COMPUTED_FOR_PROFILE, StationSource, Stations, WALKED_NO_STATIONS, WALKED_STATIONS_INCOMPLETE,
};
use owd_format::design::terrain::dem_sampler;
use owd_format::design::{DesignError, Session, Stage};
use owd_format::drawing::{self, DrawOptions, Sheet, SheetKind, Style};
use owd_format::geotiff;
use owd_format::types::TerrainSampler;
use serde_json::{Value, json};
use std::cell::Cell;
use std::path::PathBuf;

fn web() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/web")
}

/// The example as saved (no stored stations), and its DEM.
fn example() -> (String, Vec<u8>) {
    (
        std::fs::read_to_string(web().join("river-example.owd")).unwrap(),
        std::fs::read(web().join("river-dem.tif")).unwrap(),
    )
}

const ID: &str = "rolling-circle";

/// The example with the volumes stage run again on its DEM: the same numbers, and the stations
/// stored.
fn stored() -> (String, Vec<u8>) {
    let (text, dem) = example();
    let mut s = Session::open(&text).unwrap();
    let before = s.document()["stages"]["volumes"]["results"].clone();
    assert!(
        before["sections"].is_null(),
        "the example holds no stations"
    );
    let grid = geotiff::read_bytes(&dem).unwrap();
    let t = dem_sampler(&grid, Some(32630)).unwrap();
    s.run_stage_with(Stage::Volumes, Some(&t as TerrainSampler))
        .unwrap();
    let after = &s.document()["stages"]["volumes"]["results"];
    // No design number moves: every number the record held is the same.
    for (k, v) in before.as_object().unwrap() {
        assert_eq!(&after[k], v, "results.{k} moved");
    }
    assert!(after["sections"].is_object(), "the stations are stored");
    (s.to_text(), dem)
}

fn all_sheets(text: &str, dem: &[u8], options: &DrawOptions) -> Vec<Sheet> {
    drawing::sheets_from_text(text, Some(dem), options).unwrap()
}

/// Every output of every sheet, byte for byte.
fn assert_same(before: &[Sheet], after: &[Sheet], what: &str) {
    assert_eq!(
        before.iter().map(|s| &s.number).collect::<Vec<_>>(),
        after.iter().map(|s| &s.number).collect::<Vec<_>>(),
        "{what}: the same sheets"
    );
    for (b, a) in before.iter().zip(after) {
        assert!(
            b.document_text() == a.document_text(),
            "{what}: {}: the drawing document differs",
            b.number
        );
        let (rb, ra) = (drawing::render(b).unwrap(), drawing::render(a).unwrap());
        assert!(
            rb.normalised == ra.normalised,
            "{what}: {} normalised",
            b.number
        );
        assert!(rb.svg == ra.svg, "{what}: {} SVG", b.number);
        assert!(rb.pdf == ra.pdf, "{what}: {} PDF", b.number);
        assert!(rb.dxf == ra.dxf, "{what}: {} DXF", b.number);
        assert!(rb.extra == ra.extra, "{what}: {} extra files", b.number);
    }
}

fn warned(sheets: &[Sheet], kind: SheetKind, text: &str) -> bool {
    sheets
        .iter()
        .filter(|s| s.kind == kind)
        .all(|s| s.warnings.iter().any(|w| w == text))
}

#[test]
fn every_sheet_drawn_from_the_stored_stations_is_the_sheet_drawn_from_the_walk() {
    let (old, dem) = example();
    let (new, _) = stored();
    let o = DrawOptions::default();
    let before = all_sheets(&old, &dem, &o);
    let after = all_sheets(&new, &dem, &o);
    for kind in SheetKind::ALL {
        assert!(before.iter().any(|s| s.kind == *kind), "{kind:?} is drawn");
    }
    assert_same(&before, &after, "default style");
    // The old file says it was walked again; the stored one says nothing of the kind.
    assert!(warned(
        &before,
        SheetKind::CrossSections,
        WALKED_NO_STATIONS
    ));
    assert!(warned(&before, SheetKind::LongSection, WALKED_NO_STATIONS));
    for s in &after {
        assert!(
            !s.warnings
                .iter()
                .any(|w| w == WALKED_NO_STATIONS || w == COMPUTED_FOR_PROFILE),
            "{}: {:?}",
            s.number,
            s.warnings
        );
    }
}

/// A terrain that counts the heights it is asked for.
fn counting<'a>(
    inner: TerrainSampler<'a>,
    asked: &'a Cell<usize>,
) -> impl Fn(&[f64], &[f64]) -> Vec<f64> + 'a {
    move |xs, ys| {
        asked.set(asked.get() + xs.len());
        inner(xs, ys)
    }
}

#[test]
fn a_fresh_record_is_drawn_without_reading_the_terrain() {
    let (new, dem) = stored();
    let s = Session::open(&new).unwrap();
    let grid = geotiff::read_bytes(&dem).unwrap();
    let t = dem_sampler(&grid, Some(32630)).unwrap();
    let asked = Cell::new(0);
    let c = counting(&t, &asked);
    // The plan sheet's relief reads the terrain for its picture, so it is left out here.
    let o = DrawOptions {
        sheets: vec![
            SheetKind::CrossSections,
            SheetKind::LongSection,
            SheetKind::TypicalSection,
            SheetKind::SettingOut,
        ],
        ..DrawOptions::default()
    };
    let sheets = drawing::sheets(&s, &c as TerrainSampler, &o).unwrap();
    assert!(!sheets.is_empty());
    assert_eq!(asked.get(), 0, "no terrain read: nothing walked again");
    let st = s.section_stations(ID, &[], None).unwrap();
    assert_eq!(st.source, StationSource::Stored);
}

#[test]
fn a_sheet_at_another_section_spacing_cuts_its_sections_again_and_says_so() {
    let (old, dem) = example();
    let (new, _) = stored();
    let o = DrawOptions {
        sheets: vec![SheetKind::CrossSections, SheetKind::Plan],
        style: Style::builtin()
            .with_overrides(&[("sections.spacing_m".into(), json!(300.0))])
            .unwrap(),
        ..DrawOptions::default()
    };
    let before = all_sheets(&old, &dem, &o);
    let after = all_sheets(&new, &dem, &o);
    assert_same(&before, &after, "sections every 300 m");
    assert!(warned(
        &after,
        SheetKind::CrossSections,
        COMPUTED_FOR_PROFILE
    ));
    assert!(warned(&after, SheetKind::Plan, COMPUTED_FOR_PROFILE));
}

#[test]
fn a_station_with_no_stored_profile_is_refused_without_terrain_never_a_neighbour() {
    let (new, dem) = stored();
    let s = Session::open(&new).unwrap();
    let st = s.section_stations(ID, &[], None).unwrap();
    let n = st.walk.stations.len();
    let kept: Vec<usize> = (0..n).filter(|&i| st.has_profile(i)).collect();
    assert!(
        kept.len() > 1 && kept.len() < n,
        "{} of {n} kept",
        kept.len()
    );
    let bare = (0..n).find(|i| !st.has_profile(*i)).unwrap();
    let at = st.walk.stations[bare].chainage_m;

    // The cross-section of a bare station is refused, by name; a kept one is the walk's.
    let e = s.cross_section(&st, at).unwrap_err().to_string();
    assert!(e.contains("no stored section profile"), "{e}");
    let e = s.section_stations(ID, &[at], None).unwrap_err().to_string();
    assert!(e.contains("needs the DEM"), "{e}");

    let grid = geotiff::read_bytes(&dem).unwrap();
    let t = dem_sampler(&grid, Some(32630)).unwrap();
    let walked = s
        .section_stations(ID, &[at], Some(&t as TerrainSampler))
        .unwrap();
    assert_eq!(
        walked.source,
        StationSource::Computed {
            reason: COMPUTED_FOR_PROFILE
        }
    );
    assert!(walked.has_profile(bare));
    for &i in &kept {
        let c = st.walk.stations[i].chainage_m;
        assert_eq!(
            s.cross_section(&st, c).unwrap(),
            s.cross_section(&walked, c).unwrap(),
            "the stored profile at {c} m is the walk's"
        );
    }
    // The long section is the same from either.
    assert_eq!(
        s.long_section(ID, &st).unwrap(),
        s.long_section(ID, &walked).unwrap()
    );
}

#[test]
fn a_stale_record_is_not_drawn_and_a_record_without_stations_says_so() {
    let (new, dem) = stored();
    let grid = geotiff::read_bytes(&dem).unwrap();
    let t = dem_sampler(&grid, Some(32630)).unwrap();

    // Stale: a new water level changes what the volumes read.
    let mut s = Session::open(&new).unwrap();
    s.set_water_profile(&[[0.0, 37.0]]).unwrap();
    assert!(matches!(
        s.section_stations(ID, &[], Some(&t as TerrainSampler)),
        Err(DesignError::StaleUpstream { .. })
    ));
    let e = drawing::sheets_from_text(&s.to_text(), Some(&dem), &DrawOptions::default())
        .unwrap_err()
        .to_string();
    assert!(e.contains("stale"), "{e}");

    // A fresh record with no stations (saved before they were stored): refused with no terrain,
    // walked again with it, and said so.
    let mut doc: Value = serde_json::from_str(&new).unwrap();
    doc["stages"]["volumes"]["results"]
        .as_object_mut()
        .unwrap()
        .remove("sections");
    let s = Session::open(&doc.to_string()).unwrap();
    let e = s.section_stations(ID, &[], None).unwrap_err().to_string();
    assert!(e.contains("holds no stations"), "{e}");
    let w = s
        .section_stations(ID, &[], Some(&t as TerrainSampler))
        .unwrap();
    assert_eq!(
        w.source,
        StationSource::Walked {
            reason: WALKED_NO_STATIONS
        }
    );
}

#[test]
fn stations_saved_without_the_top_of_cut_are_walked_again_and_say_so() {
    // QA on PR 568, finding 1: a record saved in PR 564's format has no top of cut at its stations.
    // Read as "no top of cut", the plan would draw no top-of-cut lines; it is walked again instead.
    let (new, dem) = stored();
    let grid = geotiff::read_bytes(&dem).unwrap();
    let t = dem_sampler(&grid, Some(32630)).unwrap();
    let mut doc: Value = serde_json::from_str(&new).unwrap();
    for st in doc["stages"]["volumes"]["results"]["sections"]["stations"]
        .as_array_mut()
        .unwrap()
    {
        let o = st.as_object_mut().unwrap();
        o.shift_remove("top_of_cut_left_m");
        o.shift_remove("top_of_cut_right_m");
    }
    let text = doc.to_string();
    let s = Session::open(&text).unwrap();
    assert!(s.stored_stations(ID).unwrap().is_none());
    let e = s.section_stations(ID, &[], None).unwrap_err().to_string();
    assert!(e.contains("top of cut"), "{e}");
    let w = s
        .section_stations(ID, &[], Some(&t as TerrainSampler))
        .unwrap();
    assert_eq!(
        w.source,
        StationSource::Walked {
            reason: WALKED_STATIONS_INCOMPLETE
        }
    );
    assert!(w.top_of_cut.iter().any(|(l, r)| l.is_some() || r.is_some()));
    // The plan draws the top of cut from the walk, and says so; it equals the fresh stored plan.
    let o = DrawOptions::default();
    let walked = all_sheets(&text, &dem, &o);
    assert!(warned(&walked, SheetKind::Plan, WALKED_STATIONS_INCOMPLETE));
    let fresh = all_sheets(&new, &dem, &o);
    assert_same(&fresh, &walked, "top of cut walked again");
}

/// The stored example with two pools, so the water level steps along the line.
fn stored_with_pools() -> (String, Vec<u8>) {
    let (text, dem) = example();
    let mut s = Session::open(&text).unwrap();
    s.set_water_pools(&[[0.0, 0.0], [1610.0, -0.5]]).unwrap();
    let grid = geotiff::read_bytes(&dem).unwrap();
    let t = dem_sampler(&grid, Some(32630)).unwrap();
    s.run_stage_with(Stage::Volumes, Some(&t as TerrainSampler))
        .unwrap();
    (s.to_text(), dem)
}

#[test]
fn one_section_off_the_stored_profiles_costs_one_section_and_is_the_walks_section() {
    // The fault: a cross section at a station with no stored profile walked the whole line again,
    // which on a long design takes minutes. One station's section reads only its own samples, and
    // it is the walk's section at that station, exactly.
    let (new, dem) = stored_with_pools();
    let s = Session::open(&new).unwrap();
    let grid = geotiff::read_bytes(&dem).unwrap();
    let t = dem_sampler(&grid, Some(32630)).unwrap();
    let walk = s.option_section_walk(ID, &t as TerrainSampler).unwrap();
    let st = s.section_stations(ID, &[], None).unwrap();
    let n = st.walk.stations.len();
    assert_eq!(n, walk.stations.len());
    let bare: Vec<usize> = (0..n).filter(|&i| !st.has_profile(i)).collect();
    let wl = |i: usize| st.walk.stations[i].wl;
    // A station on the pool step: the water level differs from a neighbour's.
    let step = *bare
        .iter()
        .find(|&&i| i > 0 && i + 1 < n && (wl(i) != wl(i - 1) || wl(i) != wl(i + 1)))
        .expect("a bare station on the pool step");
    // The station in the tightest bend: the largest turn of heading to its neighbours.
    let turn = |i: usize| {
        let h = |k: usize| st.walk.stations[k].heading_rad;
        (h(i + 1) - h(i - 1)).sin().abs()
    };
    let bend = *bare
        .iter()
        .filter(|&&i| i > 0 && i + 1 < n)
        .max_by(|&&a, &&b| turn(a).total_cmp(&turn(b)))
        .unwrap();
    assert!(turn(bend) > 0.0, "the example bends");
    let picks = [
        bare[0],
        bare[bare.len() / 2],
        *bare.last().unwrap(),
        step,
        bend,
    ];
    for &i in &picks {
        let c = st.walk.stations[i].chainage_m;
        let asked = Cell::new(0);
        let counted = counting(&t, &asked);
        let one = s
            .section_stations(ID, &[c], Some(&counted as TerrainSampler))
            .unwrap();
        let samples = walk.stations[i].offsets.len();
        assert_eq!(
            asked.get(),
            samples,
            "the section at {c} m reads its own {samples} samples, not the line's"
        );
        assert!(one.has_profile(i));
        assert_eq!(
            format!("{:?}", one.walk.stations[i]),
            format!("{:?}", walk.stations[i]),
            "the section at {c} m is the walk's"
        );
        assert_eq!(
            s.cross_section(&one, c).unwrap(),
            s.cross_section(&Stations::from(walk.clone()), c).unwrap()
        );
        // The long section is the stored one: one section moves nothing else.
        assert_eq!(
            s.long_section(ID, &one).unwrap(),
            s.long_section(ID, &st).unwrap()
        );
    }
    // A station with a stored profile still reads no terrain.
    let kept = (0..n).find(|&i| st.has_profile(i)).unwrap();
    let asked = Cell::new(0);
    let counted = counting(&t, &asked);
    s.section_stations(
        ID,
        &[st.walk.stations[kept].chainage_m],
        Some(&counted as TerrainSampler),
    )
    .unwrap();
    assert_eq!(asked.get(), 0);
}
