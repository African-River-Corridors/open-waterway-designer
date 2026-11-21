//! Label: property
//! The channel as geometry and the cut as a surface (MAP-5), on synthetic ground: a straight
//! river 3 km long in UTM 30N, three pools, and a DEM made here. What is drawn must be what the
//! volumes stage counted: the daylight offsets are the cut rule's exact width, the area under the
//! drawn face is the stored area, and the After terrain is the bed inside the bed edges and the
//! DEM outside the daylight lines.
use owd_core::cut_model::{ABOVE_SLOPE_H_PER_V, BERM_M};
use owd_format::design::channel::{Channel, CutSurface, depth_class};
use owd_format::design::crs::Utm;
use owd_format::design::terrain::dem_sampler;
use owd_format::design::terrain_tile::{TILE, TerrainState, TileMaker, decode_png, pixel_lon_lat};
use owd_format::design::{AlignMethod, Role, Session, Stage};
use owd_format::geotiff::Dem;
use serde_json::{Value, json};
use std::rc::Rc;
use std::sync::Arc;

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
const X0: f64 = 500_000.0;
const Y0: f64 = 600_000.0;
const ZONE: Utm = Utm {
    zone: 30,
    south: false,
};
const CELL: f64 = 5.0;

/// A DEM of `ground(x, y)`: 5 m cells from 200 m before the river to 200 m after it, 400 m each
/// side of it.
fn dem_of(ground: impl Fn(f64, f64) -> f64) -> Dem {
    let (w, h) = (680usize, 160usize);
    let (left, top) = (X0 - 200.0, Y0 + 400.0);
    let data = (0..w * h)
        .map(|i| {
            let (r, c) = (i / w, i % w);
            ground(
                left + (c as f64 + 0.5) * CELL,
                top - (r as f64 + 0.5) * CELL,
            )
        })
        .collect();
    Dem {
        width: w,
        height: h,
        data,
        geotransform: [CELL, 0.0, left, 0.0, -CELL, top],
        nodata: Some(-9999.0),
        epsg: Some(32630),
    }
}

/// The design up to the alignment: the river along x, three pools at 12 m, 10 m and 8 m.
fn design(dem: &Dem) -> Session {
    let centre: Vec<[f64; 2]> = (0..=30).map(|i| [X0 + 100.0 * i as f64, Y0]).collect();
    let ring = [
        [X0 - 100.0, Y0 - 300.0],
        [X0 + 3100.0, Y0 - 300.0],
        [X0 + 3100.0, Y0 + 300.0],
        [X0 - 100.0, Y0 + 300.0],
        [X0 - 100.0, Y0 - 300.0],
    ];
    let mut s = Session::new("Channel", "2026-11-20");
    s.import(&[
        (
            Role::Centreline,
            &format!(
                r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
                json!(centre)
            ),
        ),
        (
            Role::RiverPolygon,
            &format!(
                r#"{{"type":"Polygon"{UTM30},"coordinates":{}}}"#,
                json!([ring])
            ),
        ),
    ])
    .unwrap();
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
        .unwrap();
    s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    s.align(
        AlignMethod::RollingCircle,
        &json!({"use_water": false}),
        None,
    )
    .unwrap();
    s.set_dem("dem.tif", b"a stand-in terrain file", dem);
    s.set_water_pools(&[[0.0, 12.0], [1000.0, 10.0], [2000.0, 8.0]])
        .unwrap();
    s
}

/// The design with the volumes run on `dem`, and its channel with the ground read.
fn with_volumes(dem: &Dem) -> (Session, Channel) {
    let mut s = design(dem);
    let t = dem_sampler(dem, Some(32630)).unwrap();
    s.run_volumes(&Value::Null, Some(&t)).unwrap();
    let channel = s.channel(Some(&t)).unwrap();
    (s, channel)
}

fn surface(s: &Session, channel: &Channel, dem: &Dem) -> CutSurface {
    s.cut_surface(channel, dem).unwrap().expect("a cut surface")
}

/// The station nearest `chainage`.
fn station(channel: &Channel, chainage: f64) -> &owd_format::design::channel::ChannelStation {
    channel
        .stations
        .iter()
        .min_by(|a, b| {
            (a.chainage_m - chainage)
                .abs()
                .total_cmp(&(b.chainage_m - chainage).abs())
        })
        .unwrap()
}

/// The area between level ground and the drawn face, between the daylight offsets.
fn drawn_area(
    st: &owd_format::design::channel::ChannelStation,
    ground: f64,
    wet_slope: f64,
) -> f64 {
    let (a, b) = (st.daylight_left_m.unwrap(), st.daylight_right_m.unwrap());
    let n = 200_000;
    let step = (b - a) / n as f64;
    (0..n)
        .map(|i| {
            let o = a + (i as f64 + 0.5) * step;
            (ground - st.face_m(o, wet_slope)).max(0.0) * step
        })
        .sum()
}

#[test]
fn on_a_flat_plain_the_daylight_is_the_cut_rules_exact_width_and_the_area_is_the_stored_one() {
    let dem = dem_of(|_, _| 20.0);
    let (s, channel) = with_volumes(&dem);
    assert!(channel.from_volumes && channel.has_ground);
    assert_eq!(channel.option_id, "rolling-circle");
    let stored = &s.document()["stages"]["volumes"]["results"]["sections"]["stations"];
    assert_eq!(channel.stations.len(), stored.as_array().unwrap().len());
    for (st, kept) in channel.stations.iter().zip(stored.as_array().unwrap()) {
        // The plain is above the water: the cut goes up the wet slope, across the berm and up
        // the dry slope to the ground.
        let exact = st.water_edge_m(channel.wet_slope)
            + BERM_M
            + ABOVE_SLOPE_H_PER_V * (20.0 - st.water_level_m);
        let (l, r) = (st.daylight_left_m.unwrap(), st.daylight_right_m.unwrap());
        assert!((r - exact).abs() < 1e-6, "right {r} against {exact}");
        assert!((l + exact).abs() < 1e-6, "left {l} against {exact}");
        // The volumes' own top of cut is the last sample inside it.
        assert!(st.top_of_cut_right_m.unwrap() <= r && r < st.top_of_cut_right_m.unwrap() + 1.0);
        // The area under the drawn face is the area the volumes stage stored, within 1 %.
        let kept_area =
            kept["dredge_area_m2"].as_f64().unwrap() + kept["excav_area_m2"].as_f64().unwrap();
        assert_eq!(st.dredge_area_m2 + st.excav_area_m2, kept_area);
        let drawn = drawn_area(st, 20.0, channel.wet_slope);
        assert!(
            ((drawn - kept_area) / kept_area).abs() < 0.01,
            "at {} m: drawn {drawn} m2, stored {kept_area} m2",
            st.chainage_m
        );
    }
}

#[test]
fn on_a_hillside_the_daylight_is_further_out_on_the_high_side() {
    // The ground rises to the north: the positive offsets of a line that runs east.
    let dem = dem_of(|_, y| 20.0 + 0.05 * (y - Y0));
    let (_, channel) = with_volumes(&dem);
    for st in &channel.stations {
        let (l, r) = (st.daylight_left_m.unwrap(), st.daylight_right_m.unwrap());
        assert!(r > -l + 5.0, "at {} m: left {l}, right {r}", st.chainage_m);
        // Each is where the face meets the hillside: ground and face agree there.
        for o in [l, r] {
            let ground = 20.0 + 0.05 * o;
            assert!((ground - st.face_m(o, channel.wet_slope)).abs() < 1e-6);
        }
    }
}

#[test]
fn ground_below_the_bed_has_no_cut_and_no_daylight() {
    let dem = dem_of(|_, _| -50.0);
    let (s, channel) = with_volumes(&dem);
    assert!(channel.stations.iter().all(|st| {
        st.daylight_left_m.is_none()
            && st.daylight_right_m.is_none()
            && st.dredge_area_m2 + st.excav_area_m2 == 0.0
    }));
    let g = s.channel_geometry(&channel).unwrap();
    assert!(!g.bed.left.is_empty() && !g.water_surface.left.is_empty());
    assert_eq!(g.daylight.unwrap().polygons.len(), 0, "no cut to outline");
    // The After ground is the ground: no tile differs.
    let cut = surface(&s, &channel, &dem);
    assert!(cut.at([X0 + 500.0, Y0], -50.0).is_none());
    let mut maker = TileMaker::new(Arc::new(dem)).unwrap();
    maker.set_overlay(TerrainState::After, Some(Rc::new(cut)));
    let (z, x, y) = tile_at(&maker, X0 + 500.0, Y0);
    assert_eq!(
        maker.heights_in(TerrainState::After, z, x, y).unwrap(),
        maker.heights(z, x, y).unwrap()
    );
}

/// The deepest tile worth asking for that holds the point (`x`, `y`) of the project CRS.
fn tile_at(maker: &TileMaker, x: f64, y: f64) -> (u8, u32, u32) {
    let [lon, lat] = ZONE.inverse(x, y).unwrap();
    let z = maker.info().max_zoom;
    let n = f64::from(1u32 << z);
    let tx = ((lon + 180.0) / 360.0 * n).floor() as u32;
    let rad = lat.to_radians();
    let ty = ((1.0 - libm::log(libm::tan(rad) + 1.0 / libm::cos(rad)) / std::f64::consts::PI) / 2.0
        * n)
        .floor() as u32;
    (z, tx, ty)
}

#[test]
fn the_after_tile_is_the_bed_inside_the_bed_edges_and_the_dem_outside_the_daylight_lines() {
    let dem = dem_of(|x, y| 20.0 + 0.002 * (x - X0) + 0.01 * (y - Y0));
    let ground = |x: f64, y: f64| 20.0 + 0.002 * (x - X0) + 0.01 * (y - Y0);
    let (s, channel) = with_volumes(&dem);
    let cut = surface(&s, &channel, &dem);
    assert!(
        cut.memory_bytes() < 400 * channel.stations.len(),
        "{}",
        cut.memory_bytes()
    );
    let mut maker = TileMaker::new(Arc::new(dem)).unwrap();
    maker.set_overlay(TerrainState::After, Some(Rc::new(cut)));
    // A tile in the first pool, away from its ends.
    let (z, tx, ty) = tile_at(&maker, X0 + 500.0, Y0);
    let before = maker.heights(z, tx, ty).unwrap();
    let after = maker.heights_in(TerrainState::After, z, tx, ty).unwrap();
    let (mut in_bed, mut on_slope, mut outside, mut tall) = (0, 0, 0, 0);
    for row in 0..TILE {
        for col in 0..TILE {
            let [lon, lat] = pixel_lon_lat(z, tx, ty, col, row);
            let [px, py] = ZONE.forward(lon, lat).unwrap();
            if !(X0 + 100.0..=X0 + 900.0).contains(&px) {
                continue;
            }
            let st = station(&channel, px - X0);
            let o = py - Y0;
            let (h, was) = (
                f64::from(after[row * TILE + col]),
                f64::from(before[row * TILE + col]),
            );
            let (l, r) = (st.daylight_left_m.unwrap(), st.daylight_right_m.unwrap());
            if o.abs() < st.half_width_m - 0.5 {
                assert!((h - st.invert_m).abs() < 0.01, "bed at offset {o}: {h}");
                in_bed += 1;
            } else if o < l - 1.5 || o > r + 1.5 {
                assert_eq!(h, was, "outside the daylight lines at offset {o}");
                // On the DEM (it ends 400 m from the line): the ground itself.
                if o.abs() < 390.0 {
                    assert!((h - ground(px, py)).abs() < 0.05);
                    outside += 1;
                }
            } else if o.abs() > st.half_width_m + 0.5 && o > l + 1.5 && o < r - 1.5 {
                // On the cut: the face, a real slope up to the ground.
                let face = st.face_m(o, channel.wet_slope);
                assert!(
                    (h - face).abs() < 0.05,
                    "face at offset {o}: {h} against {face}"
                );
                assert!(h < was);
                on_slope += 1;
                if h > st.water_level_m + 3.0 {
                    tall += 1;
                }
            }
        }
    }
    assert!(
        in_bed > 100 && on_slope > 100 && outside > 100 && tall > 10,
        "{in_bed} bed, {on_slope} slope, {outside} outside, {tall} high on the face"
    );
    // The tile of each state is kept apart, and a tile the cut does not reach is the ground's.
    let a = maker.tile_in(TerrainState::After, z, tx, ty).unwrap();
    let b = maker.tile(z, tx, ty).unwrap();
    assert_ne!(a, b);
    assert!(Arc::ptr_eq(
        &a,
        &maker.tile_in(TerrainState::After, z, tx, ty).unwrap()
    ));
    let decoded = decode_png(&a).unwrap();
    assert!((decoded[0] - f64::from(after[0])).abs() < 0.01);
    assert!(Arc::ptr_eq(
        &maker.tile_in(TerrainState::After, 2, 0, 0).unwrap(),
        &maker.tile(2, 0, 0).unwrap()
    ));
    // With the overlay off, After is the ground again.
    maker.set_overlay(TerrainState::After, None);
    assert_eq!(
        maker.heights_in(TerrainState::After, z, tx, ty).unwrap(),
        before
    );
    assert_eq!(TerrainState::from_name("after"), Some(TerrainState::After));
    assert_eq!(
        TerrainState::from_name("before"),
        Some(TerrainState::Before)
    );
    assert_eq!(TerrainState::from_name("pool"), None);
}

#[test]
fn a_point_of_the_cut_reads_out_its_levels_kind_and_station() {
    let dem = dem_of(|_, _| 20.0);
    let (s, channel) = with_volumes(&dem);
    let cut = surface(&s, &channel, &dem);
    let st = station(&channel, 500.0);
    // On the line, in the river polygon: a dredge down to the bed.
    let c = cut.at([X0 + 500.0, Y0], 20.0).unwrap();
    assert_eq!(c.kind, "dredge");
    assert!((c.chainage_m - 500.0).abs() < 1e-6 && c.offset_m.abs() < 1e-9);
    assert_eq!((c.ground_m, c.after_m), (20.0, st.invert_m));
    assert!((c.depth_m - (20.0 - st.invert_m)).abs() < 1e-9);
    assert_eq!(c.station_chainage_m, 500.0);
    assert_eq!(c.dredge_area_m2, st.dredge_area_m2);
    // Offsets are signed as the sections': the positive side is north of a line that runs east.
    assert!(cut.at([X0 + 500.0, Y0 + 20.0], 20.0).unwrap().offset_m > 0.0);
    assert!(cut.at([X0 + 500.0, Y0 - 20.0], 20.0).unwrap().offset_m < 0.0);
    // Beyond the daylight line, and beyond the ends of the line: nothing.
    let day = st.daylight_right_m.unwrap();
    assert!(cut.at([X0 + 500.0, Y0 + day - 0.5], 20.0).is_some());
    assert!(cut.at([X0 + 500.0, Y0 + day + 0.5], 20.0).is_none());
    assert!(cut.at([X0 - 20.0, Y0], 20.0).is_none());
    // The same through longitude and latitude on the DEM.
    let maker = TileMaker::new(Arc::new(dem)).unwrap();
    let [lon, lat] = ZONE.inverse(X0 + 500.0, Y0).unwrap();
    let c = cut.at_lon_lat(&maker, lon, lat).unwrap();
    assert!((c.depth_m - (20.0 - st.invert_m)).abs() < 1e-6);
    // The picture of the depth: made where the cut is, none where it is not.
    let (z, tx, ty) = tile_at(&maker, X0 + 500.0, Y0);
    let png = cut
        .depth_tile(&maker, z, tx, ty)
        .unwrap()
        .expect("a picture");
    assert_eq!(&png[1..4], b"PNG");
    assert!(cut.depth_tile(&maker, z, 0, 0).unwrap().is_none());
    assert_eq!(depth_class(0.4), 0);
    assert_eq!(depth_class(7.0), 3);
    assert_eq!(depth_class(150.0), 5);
}

#[test]
fn the_cut_filled_with_water_is_flat_at_the_water_level_and_the_face_above_it() {
    let dem = dem_of(|_, _| 20.0);
    let (s, channel) = with_volumes(&dem);
    let cut = surface(&s, &channel, &dem);
    let st = station(&channel, 500.0);
    let wl = st.water_level_m;
    // In the bed and on the slope below water: the water level. On the berm: the water level too.
    for o in [
        0.0,
        st.half_width_m + 2.0,
        st.water_edge_m(channel.wet_slope) + 1.0,
    ] {
        assert_eq!(cut.filled_level(X0 + 500.0, Y0 + o, 20.0), wl, "offset {o}");
        assert_eq!(
            cut.at([X0 + 500.0, Y0 + o], 20.0).unwrap().water_level_m,
            wl
        );
    }
    // On the dry slope: the face, above the water and below the ground.
    let o = st.water_edge_m(channel.wet_slope) + 8.0;
    let high = cut.filled_level(X0 + 500.0, Y0 + o, 20.0);
    assert!((high - st.face_m(o, channel.wet_slope)).abs() < 1e-9 && high > wl && high < 20.0);
    // Outside the cut: the ground.
    assert_eq!(cut.filled_level(X0 + 500.0, Y0 + 150.0, 20.0), 20.0);
}

#[test]
fn with_no_dem_there_are_bed_and_water_surface_edges_only() {
    let dem = dem_of(|_, _| 20.0);
    // Before the volumes have run: the walk's frame, with no terrain read.
    let s = design(&dem);
    let text = s.to_text();
    let frame = s.channel(None).unwrap();
    assert!(!frame.from_volumes && !frame.has_ground);
    let g = s.channel_geometry(&frame).unwrap();
    assert_eq!(g.bed.left.len(), 1);
    assert_eq!(g.bed.left[0].len(), frame.stations.len());
    assert_eq!(g.water_surface.polygons.len(), 1);
    assert!(g.daylight.is_none());
    assert!(s.cut_surface(&frame, &dem).unwrap().is_none());
    // With the volumes run and no DEM at hand: the same edges, from the stored stations.
    let (s2, with_ground) = with_volumes(&dem);
    let text2 = s2.to_text();
    let no_ground = s2.channel(None).unwrap();
    assert!(no_ground.from_volumes && !no_ground.has_ground);
    let g2 = s2.channel_geometry(&no_ground).unwrap();
    assert!(g2.daylight.is_none());
    assert_eq!(g2.bed, g.bed, "the frame is the walk's frame");
    assert_eq!(g2.water_surface, g.water_surface);
    assert!(
        s2.channel_geometry(&with_ground)
            .unwrap()
            .daylight
            .is_some()
    );
    // Nothing of this is written to the design.
    assert_eq!(s.to_text(), text);
    assert_eq!(s2.to_text(), text2);
    assert_ne!(s.channel_key(), s2.channel_key());
}

#[test]
fn the_edges_are_the_trapezoid_and_split_where_the_line_is_off_the_dem() {
    let dem = dem_of(|_, _| 20.0);
    let (s, mut channel) = with_volumes(&dem);
    let g = s.channel_geometry(&channel).unwrap();
    let day = g.daylight.as_ref().unwrap();
    assert_eq!((g.bed.polygons.len(), day.polygons.len()), (1, 1));
    // Back in metres: the bed is the bottom width wide, the water surface wider by the slopes,
    // the daylight lines wider again, each centred on the line.
    let i = 20;
    let st = &channel.stations[i];
    let width = |e: &owd_format::design::channel::Edges| {
        let l = ZONE.forward(e.left[0][i][0], e.left[0][i][1]).unwrap();
        let r = ZONE.forward(e.right[0][i][0], e.right[0][i][1]).unwrap();
        assert!((l[0] - st.x).abs() < 1e-3 && (r[0] - st.x).abs() < 1e-3);
        assert!(l[1] < Y0 && r[1] > Y0, "left is the negative offset");
        r[1] - l[1]
    };
    assert!((width(&g.bed) - 2.0 * st.half_width_m).abs() < 1e-3);
    assert!((width(&g.water_surface) - 2.0 * st.water_edge_m(channel.wet_slope)).abs() < 1e-3);
    assert!(
        (width(day) - (st.daylight_right_m.unwrap() - st.daylight_left_m.unwrap())).abs() < 1e-3
    );
    let ring = &g.bed.polygons[0];
    assert_eq!(ring.first(), ring.last());
    assert_eq!(ring.len(), 2 * channel.stations.len() + 1);
    // Ten stations off the DEM: every kind is in two parts, and nothing is drawn across the gap.
    for st in &mut channel.stations[40..50] {
        st.on_dem = false;
    }
    let g = s.channel_geometry(&channel).unwrap();
    assert_eq!(g.bed.left.len(), 2);
    assert_eq!(g.water_surface.polygons.len(), 2);
    assert_eq!(g.daylight.unwrap().right.len(), 2);
    assert_eq!(g.bed.left[0].len(), 40);
    assert_eq!(g.bed.left[1].len(), channel.stations.len() - 50);
    let cut = surface(&s, &channel, &dem);
    assert!(
        cut.at([X0 + 45.0 * 25.0, Y0], 20.0).is_none(),
        "no cut off the DEM"
    );
    assert!(cut.at([X0 + 20.0 * 25.0, Y0], 20.0).is_some());
}

#[test]
fn a_dem_in_degrees_gives_the_same_cut() {
    // The same plain as a WGS84 DEM over the river.
    let [w, s_] = ZONE.inverse(X0 - 300.0, Y0 - 500.0).unwrap();
    let [e, n] = ZONE.inverse(X0 + 3300.0, Y0 + 500.0).unwrap();
    let (cols, rows) = (720usize, 200usize);
    let geo = Dem {
        width: cols,
        height: rows,
        data: vec![20.0; cols * rows],
        geotransform: [
            (e - w) / cols as f64,
            0.0,
            w,
            0.0,
            -(n - s_) / rows as f64,
            n,
        ],
        nodata: Some(-9999.0),
        epsg: Some(4326),
    };
    let (s, channel) = with_volumes(&geo);
    let cut = surface(&s, &channel, &geo);
    let st = station(&channel, 500.0);
    let mut maker = TileMaker::new(Arc::new(geo)).unwrap();
    let [lon, lat] = ZONE.inverse(X0 + 500.0, Y0 + 3.0).unwrap();
    let c = cut.at_lon_lat(&maker, lon, lat).unwrap();
    assert!((c.after_m - st.invert_m).abs() < 1e-6 && (c.offset_m - 3.0).abs() < 1e-3);
    maker.set_overlay(TerrainState::After, Some(Rc::new(cut)));
    let (z, tx, ty) = tile_at(&maker, X0 + 500.0, Y0);
    let after = maker.heights_in(TerrainState::After, z, tx, ty).unwrap();
    assert!(
        after
            .iter()
            .any(|h| (f64::from(*h) - st.invert_m).abs() < 0.01)
    );
    assert!(after.contains(&20.0));
}
