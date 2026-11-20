//! The reports read the volumes stage's stored stations (spec 0017, ticket 10). The web example was
//! saved before the stations were stored, so its reports walk the line again: the reports as they
//! were. With the volumes stage run again on the same DEM (no number moves), the reports read the
//! stored stations, and each report is the same: its ORD document and every asset (the figures),
//! byte for byte, and every rendered format. The run report is left out: it lists the hash of the
//! design file, which holds the stored stations now.
//!
//! Label: property

mod common;

use common::*;
use owd_format::design::terrain::dem_sampler;
use owd_format::design::{Session, Stage};
use owd_format::geotiff;
use owd_format::types::TerrainSampler;
use owd_report::{Control, Format, Kind, Request, build};

fn with_stations(design: &str, dem: &[u8]) -> String {
    let mut s = Session::open(design).unwrap();
    let grid = geotiff::read_bytes(dem).unwrap();
    let t = dem_sampler(&grid, Some(32630)).unwrap();
    s.run_stage_with(Stage::Volumes, Some(&t as TerrainSampler))
        .unwrap();
    assert!(s.document()["stages"]["volumes"]["results"]["sections"].is_object());
    s.to_text()
}

#[test]
fn every_report_read_from_the_stored_stations_is_the_report_from_the_walk() {
    let (old, dem) = web();
    let new = with_stations(&old, &dem);
    for kind in [Kind::Screening, Kind::Quantities, Kind::Sections] {
        let req = Request {
            kind,
            option: None,
            control: Control::draft(DATE),
            requested_by: None,
        };
        let before = build(&old, Some(&dem), &req).unwrap();
        let after = build(&new, Some(&dem), &req).unwrap();
        assert_eq!(before.number, after.number);
        assert!(
            before.document == after.document,
            "{kind:?}: the ORD document differs"
        );
        assert!(before.assets == after.assets, "{kind:?}: an asset differs");
        for fmt in Format::ALL {
            assert!(
                before.render(fmt).unwrap().bytes == after.render(fmt).unwrap().bytes,
                "{kind:?}: {fmt:?} differs"
            );
        }
    }
}

#[test]
fn a_stale_record_is_not_read_as_fresh_in_the_quantity_schedule() {
    // A stale record is not read as fresh: the faces say why they are not computed.
    let (old, dem) = web();
    let new = with_stations(&old, &dem);
    let mut s = Session::open(&new).unwrap();
    s.set_water_profile(&[[0.0, 37.0]]).unwrap();
    let req = Request {
        kind: Kind::Quantities,
        option: None,
        control: Control::draft(DATE),
        requested_by: None,
    };
    let r = build(&s.to_text(), Some(&dem), &req).unwrap();
    let text = r.document.to_string();
    assert!(text.contains("stale"), "the schedule says the volumes are stale");
    assert!(text.contains("not computed"));
}
