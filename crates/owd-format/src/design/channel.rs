//! The channel as geometry (MAP-5): what the map draws of the design channel and of its cut.
//!
//! Everything here is derived on demand and nothing is stored in the design. The stations are the
//! volumes stage's stored stations when its record is fresh (their place, half-width, water level,
//! invert, bench flags and top of cut), so what is drawn is what the volumes counted. With no fresh
//! record the stations are the walk's frame with no terrain read: the bed and the water-surface
//! edges can still be drawn; the cut cannot.
//!
//! The cut face is the core's [`cut_model_a_face`], the one rule the volumes use, under both cut
//! models: a model B station's top of cut already stops at its daylight limit. Three things are
//! made from the stations:
//!
//! - [`ChannelGeometry`]: the bed edges, the edges at the water surface and the daylight lines, as
//!   lines and polygons in longitude and latitude.
//! - [`CutSurface`]: the cut as a surface over the ground, read point by point. It is the stations
//!   and an index of the segments between them, not a raster: about 150 bytes a station and at
//!   most 150 bytes of index a station (under 3 MB for 8,000 stations), whatever the size of the
//!   DEM. It is the [`Overlay`] of the After terrain state, and it colours the cut-depth tiles.
//! - [`CutAt`]: what a click on the cut reads out.
//!
//! Between two stations the half-width, the levels and the reach of the cut run straight from one
//! to the other, and the bench flag is the nearer station's. A point is cut where the ground is
//! above the face, as in the volumes' own sum; where the ground is below the face it stays as it
//! is (the model has no fill).

use super::crs::{Crs, Utm};
use super::session::{DesignError, Session, Stage, StageStatus};
use super::terrain_tile::{Overlay, TILE, TileMaker};
use crate::geotiff::Dem;
use crate::types::TerrainSampler;
use owd_core::cut_model::cut_model_a_face;
use owd_core::geom::Pt;
use owd_core::geom::overlay::PolygonIndex;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

/// One station of the channel.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelStation {
    pub chainage_m: f64,
    /// The station's place on the design line, in the project CRS.
    pub x: f64,
    pub y: f64,
    pub heading_rad: f64,
    /// The bottom half-width.
    pub half_width_m: f64,
    pub water_level_m: f64,
    /// The channel bed level.
    pub invert_m: f64,
    /// The cut continues above the water on the left (negative offsets) or the right: the core's
    /// bench gate, as the volumes stage stored it. False with no stored station.
    pub bench_left: bool,
    pub bench_right: bool,
    /// The volumes stage's top of cut each side: the outermost sample of the section where the
    /// face is at or below the ground, as an offset (left negative). `None`: no cut on that side,
    /// or no stored station.
    pub top_of_cut_left_m: Option<f64>,
    pub top_of_cut_right_m: Option<f64>,
    /// Where the cut face meets the ground each side, as an offset (left negative): the top of
    /// cut, moved out to the crossing between it and the next sample. `None` with no ground read.
    pub daylight_left_m: Option<f64>,
    pub daylight_right_m: Option<f64>,
    /// The station's stored cut areas (zero with no stored station).
    pub dredge_area_m2: f64,
    pub excav_area_m2: f64,
    pub max_face_h_m: f64,
    /// False where the design line is off the DEM at the station (the volumes stage stored no
    /// ground on the line there).
    pub on_dem: bool,
}

impl ChannelStation {
    /// The design depth of water.
    pub fn depth_m(&self) -> f64 {
        self.water_level_m - self.invert_m
    }

    /// The offset of the water's edge on the cut slope, from the design line.
    pub fn water_edge_m(&self, wet_slope: f64) -> f64 {
        self.half_width_m + wet_slope * self.depth_m()
    }

    /// The cut face level at `offset_m` (left negative): the volumes' own rule. Infinity where
    /// the rule has no face (above the water with no bench).
    pub fn face_m(&self, offset_m: f64, wet_slope: f64) -> f64 {
        let bench = if offset_m < 0.0 {
            self.bench_left
        } else {
            self.bench_right
        };
        cut_model_a_face(
            offset_m.abs(),
            bench,
            self.half_width_m,
            self.invert_m,
            self.water_level_m,
            self.depth_m(),
            wet_slope,
        )
    }

    /// The point `offset_m` across the line at this station, in the project CRS: the place of the
    /// section's sample there.
    pub fn at_offset(&self, offset_m: f64) -> Pt {
        [
            self.x - offset_m * libm::sin(self.heading_rad),
            self.y + offset_m * libm::cos(self.heading_rad),
        ]
    }
}

/// The channel along the design line.
#[derive(Debug, Clone, PartialEq)]
pub struct Channel {
    /// The design line's id.
    pub option_id: String,
    pub stations: Vec<ChannelStation>,
    /// The slope of the cut below water, horizontal per vertical.
    pub wet_slope: f64,
    /// The spacing of a section's samples.
    pub dx_m: f64,
    /// How far a section reaches each side.
    pub maxw_m: f64,
    /// The stations are the volumes stage's stored ones.
    pub from_volumes: bool,
    /// The ground was read: the stations hold their daylight offsets.
    pub has_ground: bool,
}

/// The two edges of one kind, as lines in longitude and latitude, and the band between them as
/// polygons: one of each for every run of stations on the DEM.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct Edges {
    pub left: Vec<Vec<[f64; 2]>>,
    pub right: Vec<Vec<[f64; 2]>>,
    /// Each a closed ring: up the left edge and back down the right.
    pub polygons: Vec<Vec<[f64; 2]>>,
}

/// The channel as the map draws it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChannelGeometry {
    pub option_id: String,
    pub stations: usize,
    /// The stations are the volumes stage's stored ones.
    pub from_volumes: bool,
    /// The edges of the channel bed: half the bottom width each side of the design line.
    pub bed: Edges,
    /// Where the side slopes meet the pool's water level.
    pub water_surface: Edges,
    /// Where the cut slopes meet the existing ground. `None` with no ground read or no stored
    /// stations. A side with no cut at a station is on the design line there.
    pub daylight: Option<Edges>,
}

/// What a point of the cut reads out.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CutAt {
    /// The chainage of the point along the design line.
    pub chainage_m: f64,
    /// Its offset from the design line (left negative).
    pub offset_m: f64,
    /// The ground before the cut.
    pub ground_m: f64,
    /// The level after the cut: the cut face.
    pub after_m: f64,
    pub depth_m: f64,
    /// The design water level there: the pool's level.
    pub water_level_m: f64,
    /// `dredge` inside the river polygon, else `dry`.
    pub kind: &'static str,
    /// The nearer station and its stored cut areas.
    pub station_chainage_m: f64,
    pub dredge_area_m2: f64,
    pub excav_area_m2: f64,
    pub max_face_h_m: f64,
}

impl Session {
    /// A text that changes when the channel of the design line may have changed: what the volumes
    /// stage depends on, what it depended on when it ran, whether it is fresh, and whether its
    /// record holds the stations (a record saved before OWD stored them holds none, and a new run
    /// on the same design adds them).
    pub fn channel_key(&self) -> String {
        let res = &self.doc["stages"]["volumes"]["results"];
        format!(
            "{}|{}|{:?}|{}|{}",
            self.dependency_hash(Stage::Volumes),
            self.doc["stages"]["volumes"]["depends_on"],
            self.stage_status(Stage::Volumes),
            res["sections"]["stations"]
                .as_array()
                .map_or(0, |stations| stations.len()),
            res["total_m3"],
        )
    }

    /// The channel along the design line. With `terrain` (the DEM in the project CRS) and fresh
    /// stored stations, each station holds where its cut meets the ground. Refused, with the
    /// reason, when there is no design line, no sizing or no water level yet.
    pub fn channel(&self, terrain: Option<TerrainSampler>) -> Result<Channel, DesignError> {
        let Some(o) = self.design_lines().into_iter().next() else {
            return Err(DesignError::Input(
                "there is no design line: run the alignment stage first".into(),
            ));
        };
        let id = o["id"].as_str().unwrap_or("?").to_string();
        let plan = self.section_plan(&Value::Null)?;
        let (wet_slope, maxw_m, dx_m) = Session::plan_cut_settings(&plan);
        let stored = match self.stage_status(Stage::Volumes) {
            StageStatus::Fresh => self.stored_stations(&id).ok().flatten(),
            _ => None,
        };
        let Some(stored) = stored else {
            let (frame, depth) = self.option_frame(&plan, &o)?;
            let stations = frame
                .iter()
                .map(|s| ChannelStation {
                    chainage_m: s.chainage_m,
                    x: s.xy[0],
                    y: s.xy[1],
                    heading_rad: s.heading_rad,
                    half_width_m: s.hb,
                    water_level_m: s.wl,
                    invert_m: s.wl - depth,
                    bench_left: false,
                    bench_right: false,
                    top_of_cut_left_m: None,
                    top_of_cut_right_m: None,
                    daylight_left_m: None,
                    daylight_right_m: None,
                    dredge_area_m2: 0.0,
                    excav_area_m2: 0.0,
                    max_face_h_m: 0.0,
                    on_dem: true,
                })
                .collect();
            return Ok(Channel {
                option_id: id,
                stations,
                wet_slope,
                dx_m,
                maxw_m,
                from_volumes: false,
                has_ground: false,
            });
        };
        let mut stations: Vec<ChannelStation> = stored
            .walk
            .stations
            .iter()
            .zip(&stored.top_of_cut)
            .zip(&stored.bed_m)
            .map(|((s, top), bed)| ChannelStation {
                chainage_m: s.chainage_m,
                x: s.x,
                y: s.y,
                heading_rad: s.heading_rad,
                half_width_m: s.hb,
                water_level_m: s.wl,
                invert_m: s.grade,
                bench_left: s.bench_left,
                bench_right: s.bench_right,
                top_of_cut_left_m: top.0,
                top_of_cut_right_m: top.1,
                daylight_left_m: None,
                daylight_right_m: None,
                dredge_area_m2: s.dredge_area_m2,
                excav_area_m2: s.excav_area_m2,
                max_face_h_m: s.max_face_h_m,
                on_dem: bed.is_finite(),
            })
            .collect();
        let has_ground = terrain.is_some();
        if let Some(terrain) = terrain {
            // The ground as the walk read it, at the top of cut and at the next sample out, each
            // side of each station: four points a station, in one call.
            let ground = Session::plan_terrain(&plan, terrain);
            let mut at: Vec<(usize, bool, f64)> = Vec::new();
            let (mut xs, mut ys) = (Vec::new(), Vec::new());
            for (i, s) in stations.iter().enumerate() {
                for (left, top) in [(true, s.top_of_cut_left_m), (false, s.top_of_cut_right_m)] {
                    let Some(top) = top else { continue };
                    let next = top + if left { -dx_m } else { dx_m };
                    for o in [top, next] {
                        let p = s.at_offset(o);
                        xs.push(p[0]);
                        ys.push(p[1]);
                    }
                    at.push((i, left, top));
                }
            }
            let z = ground(&xs, &ys);
            for (k, &(i, left, top)) in at.iter().enumerate() {
                let s = &stations[i];
                let next = top + if left { -dx_m } else { dx_m };
                let (g0, g1) = (
                    z.get(2 * k).copied().unwrap_or(f64::NAN) - s.face_m(top, wet_slope),
                    z.get(2 * k + 1).copied().unwrap_or(f64::NAN) - s.face_m(next, wet_slope),
                );
                // The crossing between the two samples, where both are known and the ground goes
                // from above the face to below it; else the top of cut itself.
                let t = if g0.is_finite() && g1.is_finite() && g0 >= 0.0 && g1 < 0.0 {
                    (g0 / (g0 - g1)).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let day = top + (next - top) * t;
                let day = day.clamp(-maxw_m, maxw_m);
                if left {
                    stations[i].daylight_left_m = Some(day);
                } else {
                    stations[i].daylight_right_m = Some(day);
                }
            }
        }
        Ok(Channel {
            option_id: id,
            stations,
            wet_slope,
            dx_m,
            maxw_m,
            from_volumes: true,
            has_ground,
        })
    }

    /// The channel as lines and polygons in longitude and latitude.
    pub fn channel_geometry(&self, channel: &Channel) -> Result<ChannelGeometry, DesignError> {
        let utm = self.project_utm()?;
        let st = &channel.stations;
        // The runs of stations on the DEM: an edge is split where the design line is off it.
        let runs = |keep: &dyn Fn(&ChannelStation) -> bool| -> Vec<(usize, usize)> {
            let mut out = Vec::new();
            let mut from = None;
            for (i, s) in st.iter().enumerate() {
                match (keep(s), from) {
                    (true, None) => from = Some(i),
                    (false, Some(a)) => {
                        out.push((a, i));
                        from = None;
                    }
                    _ => {}
                }
            }
            if let Some(a) = from {
                out.push((a, st.len()));
            }
            out.retain(|(a, b)| b - a >= 2);
            out
        };
        let edges = |keep: &dyn Fn(&ChannelStation) -> bool,
                     offsets: &dyn Fn(&ChannelStation) -> (f64, f64)|
         -> Result<Edges, DesignError> {
            let mut e = Edges::default();
            for (a, b) in runs(keep) {
                let side =
                    |pick: &dyn Fn((f64, f64)) -> f64| -> Result<Vec<[f64; 2]>, DesignError> {
                        let pts: Vec<Pt> = st[a..b]
                            .iter()
                            .map(|s| s.at_offset(pick(offsets(s))))
                            .collect();
                        utm.inverse_all(&pts).map_err(DesignError::Input)
                    };
                let (left, right) = (side(&|o| o.0)?, side(&|o| o.1)?);
                let mut ring = left.clone();
                ring.extend(right.iter().rev());
                ring.push(left[0]);
                e.left.push(left);
                e.right.push(right);
                e.polygons.push(ring);
            }
            Ok(e)
        };
        let on_dem = |s: &ChannelStation| s.on_dem;
        let ws = channel.wet_slope;
        let daylight = if channel.has_ground && channel.from_volumes {
            let cut = |s: &ChannelStation| {
                s.on_dem && (s.daylight_left_m.is_some() || s.daylight_right_m.is_some())
            };
            Some(edges(&cut, &|s| {
                (
                    s.daylight_left_m.unwrap_or(0.0),
                    s.daylight_right_m.unwrap_or(0.0),
                )
            })?)
        } else {
            None
        };
        Ok(ChannelGeometry {
            option_id: channel.option_id.clone(),
            stations: st.len(),
            from_volumes: channel.from_volumes,
            bed: edges(&on_dem, &|s| (-s.half_width_m, s.half_width_m))?,
            water_surface: edges(&on_dem, &|s| (-s.water_edge_m(ws), s.water_edge_m(ws)))?,
            daylight,
        })
    }

    /// The cut of `channel` as a surface over `dem`. `None` when the channel has no cut to show:
    /// its stations are not the volumes stage's, or the ground was not read.
    pub fn cut_surface(
        &self,
        channel: &Channel,
        dem: &Dem,
    ) -> Result<Option<CutSurface>, DesignError> {
        if !(channel.from_volumes && channel.has_ground) {
            return Ok(None);
        }
        let utm = self.project_utm()?;
        let plan = self.section_plan(&Value::Null)?;
        let river: Vec<_> = self
            .river_polygons()
            .map(|ps| ps.into_iter().filter(|p| !p.exterior.is_empty()).collect())
            .unwrap_or_default();
        // A DEM in the project's own zone is read where it is; one in degrees goes through the
        // projection, point by point (the two CRS the stages read a DEM in).
        let to_project: Option<ToProject> = match dem.epsg {
            Some(e) if e == utm.epsg() => None,
            Some(4326) => Some(Box::new(utm.forward_fn().map_err(DesignError::Input)?)),
            _ => {
                return Err(DesignError::Input(
                    "the DEM's CRS is not WGS84 and not the channel design's".into(),
                ));
            }
        };
        CutSurface::new(
            channel,
            utm,
            to_project,
            PolygonIndex::new(&river),
            Session::plan_bed_at(&plan),
        )
        .map(Some)
    }

    fn project_utm(&self) -> Result<Utm, DesignError> {
        self.doc["project"]["crs"]["epsg"]
            .as_u64()
            .and_then(|c| Crs::from_epsg(c as u32).ok())
            .and_then(|c| match c {
                Crs::Utm(u) => Some(u),
                Crs::Wgs84 => None,
            })
            .ok_or_else(|| DesignError::Input("the channel design has no project CRS".into()))
    }
}

type ToProject = Box<dyn Fn([f64; 2]) -> Option<[f64; 2]>>;
type BedAt = Box<dyn Fn(Pt) -> Option<f64>>;

/// The side of a square of the segment index, in metres.
const INDEX_CELL_M: f64 = 128.0;

/// The depth classes of the cut-depth picture: the lower bound of each, in metres.
pub const DEPTH_CLASSES_M: [f64; 6] = [0.0, 1.0, 2.0, 5.0, 10.0, 20.0];
/// The colour of each depth class of a dredge cut (inside the river): blues, darker with depth.
pub const DREDGE_COLOURS: [[u8; 3]; 6] = [
    [0xc6, 0xdb, 0xef],
    [0x9e, 0xca, 0xe1],
    [0x6b, 0xae, 0xd6],
    [0x31, 0x82, 0xbd],
    [0x08, 0x51, 0x9c],
    [0x08, 0x30, 0x6b],
];
/// The colour of each depth class of a dry cut: oranges to brown, darker with depth.
pub const DRY_COLOURS: [[u8; 3]; 6] = [
    [0xfd, 0xd0, 0xa2],
    [0xfd, 0xae, 0x6b],
    [0xfd, 0x8d, 0x3c],
    [0xe6, 0x55, 0x0d],
    [0xa6, 0x36, 0x03],
    [0x7f, 0x27, 0x04],
];

/// The depth class of a depth of cut.
pub fn depth_class(depth_m: f64) -> usize {
    DEPTH_CLASSES_M
        .iter()
        .rposition(|&d| depth_m >= d)
        .unwrap_or(0)
}

/// The cut as a surface over the ground: see the module's notes.
pub struct CutSurface {
    stations: Vec<ChannelStation>,
    /// The reach of the cut each side of each station, in metres from the line (zero: none).
    reach: Vec<(f64, f64)>,
    wet_slope: f64,
    /// The segments (by their first station) whose cut may reach each square of the index.
    index: HashMap<(i32, i32), Vec<u32>>,
    /// The DEM's CRS to the project CRS. `None`: they are the same.
    to_project: Option<ToProject>,
    river: PolygonIndex,
    /// The bed model's bed at a point, where the document has one: the ground the volumes read
    /// inside the water.
    bed: Option<BedAt>,
    /// West, south, east and north of the cut, in degrees.
    bounds: [f64; 4],
}

impl core::fmt::Debug for CutSurface {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CutSurface")
            .field("stations", &self.stations.len())
            .field("index_squares", &self.index.len())
            .field("bounds", &self.bounds)
            .finish()
    }
}

impl CutSurface {
    fn new(
        channel: &Channel,
        utm: Utm,
        to_project: Option<ToProject>,
        river: PolygonIndex,
        bed: Option<BedAt>,
    ) -> Result<CutSurface, DesignError> {
        let stations = channel.stations.clone();
        let reach: Vec<(f64, f64)> = stations
            .iter()
            .map(|s| {
                let side = |d: Option<f64>| {
                    if s.on_dem {
                        d.map_or(0.0, f64::abs)
                    } else {
                        0.0
                    }
                };
                (side(s.daylight_left_m), side(s.daylight_right_m))
            })
            .collect();
        let mut index: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        let cell = |v: f64| libm::floor(v / INDEX_CELL_M) as i32;
        let mut corners: Vec<Pt> = Vec::new();
        for i in 0..stations.len().saturating_sub(1) {
            let (a, b) = (&stations[i], &stations[i + 1]);
            let r = reach[i]
                .0
                .max(reach[i].1)
                .max(reach[i + 1].0)
                .max(reach[i + 1].1);
            if r <= 0.0 || ![a.x, a.y, b.x, b.y].iter().all(|v| v.is_finite()) {
                continue;
            }
            let (x0, x1) = (a.x.min(b.x) - r, a.x.max(b.x) + r);
            let (y0, y1) = (a.y.min(b.y) - r, a.y.max(b.y) + r);
            for cx in cell(x0)..=cell(x1) {
                for cy in cell(y0)..=cell(y1) {
                    index.entry((cx, cy)).or_default().push(i as u32);
                }
            }
            corners.extend([[x0, y0], [x0, y1], [x1, y0], [x1, y1]]);
        }
        let ll = utm.inverse_all(&corners).map_err(DesignError::Input)?;
        let mut bounds = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for p in ll {
            bounds = [
                bounds[0].min(p[0]),
                bounds[1].min(p[1]),
                bounds[2].max(p[0]),
                bounds[3].max(p[1]),
            ];
        }
        Ok(CutSurface {
            stations,
            reach,
            wet_slope: channel.wet_slope,
            index,
            to_project,
            river,
            bed,
            bounds,
        })
    }

    /// The bytes this surface holds, about: the stations and the index.
    pub fn memory_bytes(&self) -> usize {
        self.stations.len() * (core::mem::size_of::<ChannelStation>() + 16)
            + self.index.values().map(|v| v.len() * 4 + 48).sum::<usize>()
    }

    /// West, south, east and north of the cut, in degrees.
    pub fn bounds(&self) -> [f64; 4] {
        self.bounds
    }

    /// The cut at a point of the project CRS where the DEM's ground is `ground_m`: `None` where
    /// nothing is cut (outside the reach of the cut, or the ground at or below the face).
    pub fn at(&self, p: Pt, ground_m: f64) -> Option<CutAt> {
        let key = (
            libm::floor(p[0] / INDEX_CELL_M) as i32,
            libm::floor(p[1] / INDEX_CELL_M) as i32,
        );
        let segments = self.index.get(&key)?;
        // The nearest segment of the line: the point's chainage and offset are read on it.
        let last = self.stations.len() - 2;
        let mut best: Option<(f64, usize, f64, f64)> = None;
        for &i in segments {
            let i = i as usize;
            let (a, b) = (&self.stations[i], &self.stations[i + 1]);
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            let len2 = dx * dx + dy * dy;
            if len2 <= 0.0 {
                continue;
            }
            let t = ((p[0] - a.x) * dx + (p[1] - a.y) * dy) / len2;
            // Beyond the ends of the line there is no channel.
            if (i == 0 && t < 0.0) || (i == last && t > 1.0) {
                continue;
            }
            let t = t.clamp(0.0, 1.0);
            let (qx, qy) = (a.x + t * dx, a.y + t * dy);
            let dist = libm::hypot(p[0] - qx, p[1] - qy);
            // Positive on the side the section's positive offsets are on.
            let cross = dx * (p[1] - a.y) - dy * (p[0] - a.x);
            let offset = if cross < 0.0 { -dist } else { dist };
            if best.is_none_or(|(d, ..)| dist < d) {
                best = Some((dist, i, t, offset));
            }
        }
        let (dist, i, t, offset) = best?;
        let (a, b) = (&self.stations[i], &self.stations[i + 1]);
        let mix = |u: f64, v: f64| u + (v - u) * t;
        let left = offset < 0.0;
        let reach = if left {
            mix(self.reach[i].0, self.reach[i + 1].0)
        } else {
            mix(self.reach[i].1, self.reach[i + 1].1)
        };
        if dist > reach {
            return None;
        }
        let near = if t < 0.5 { a } else { b };
        let bench = if left {
            near.bench_left
        } else {
            near.bench_right
        };
        let (hb, wl, grade) = (
            mix(a.half_width_m, b.half_width_m),
            mix(a.water_level_m, b.water_level_m),
            mix(a.invert_m, b.invert_m),
        );
        let face = cut_model_a_face(dist, bench, hb, grade, wl, wl - grade, self.wet_slope);
        let ground = self.bed.as_ref().and_then(|bed| bed(p)).unwrap_or(ground_m);
        let depth = ground - face;
        if !(depth.is_finite() && depth > 0.0) {
            return None;
        }
        Some(CutAt {
            chainage_m: mix(a.chainage_m, b.chainage_m),
            offset_m: offset,
            ground_m: ground,
            after_m: face,
            depth_m: depth,
            water_level_m: wl,
            kind: if self.river.contains_xy(p) {
                "dredge"
            } else {
                "dry"
            },
            station_chainage_m: near.chainage_m,
            dredge_area_m2: near.dredge_area_m2,
            excav_area_m2: near.excav_area_m2,
            max_face_h_m: near.max_face_h_m,
        })
    }

    /// [`CutSurface::at`] for a point of the DEM's CRS.
    pub fn at_dem(&self, x: f64, y: f64, ground_m: f64) -> Option<CutAt> {
        let p = match &self.to_project {
            None => [x, y],
            Some(f) => {
                // In degrees: the box of the cut is a cheap first test.
                let [w, s, e, n] = self.bounds;
                if x < w || x > e || y < s || y > n {
                    return None;
                }
                f([x, y])?
            }
        };
        self.at(p, ground_m)
    }

    /// The level at a point of the DEM's CRS with the cut made and filled with water: the design
    /// water level where the cut face is below it, the face where it is above, and `ground` where
    /// nothing is cut. For the terrain the 3D view stands on while the pool water is shown.
    pub fn filled_level(&self, x: f64, y: f64, ground: f64) -> f64 {
        self.at_dem(x, y, ground)
            .map_or(ground, |c| c.after_m.max(c.water_level_m))
    }

    /// The cut at a place given in longitude and latitude, read on the maker's DEM.
    pub fn at_lon_lat(&self, maker: &TileMaker, lon: f64, lat: f64) -> Option<CutAt> {
        let dem = maker.dem();
        let [x, y] = match (&self.to_project, dem.epsg) {
            (Some(_), _) => [lon, lat],
            (None, Some(e)) => match Crs::from_epsg(e).ok()? {
                Crs::Utm(u) => u.forward(lon, lat).ok()?,
                Crs::Wgs84 => [lon, lat],
            },
            (None, None) => return None,
        };
        let ground = dem.sample_bilinear(x, y);
        self.at_dem(x, y, ground)
    }

    /// The depth of cut over tile `z/x/y` as a picture (RGBA PNG) to lay on the map, in 2D or on
    /// the terrain: the dredge in blues and the dry cut in oranges, darker with depth
    /// ([`DEPTH_CLASSES_M`]); clear where nothing is cut. `None` when the tile holds no cut.
    pub fn depth_tile(
        &self,
        maker: &TileMaker,
        z: u8,
        x: u32,
        y: u32,
    ) -> Result<Option<Vec<u8>>, DesignError> {
        let [w, s, e, n] = super::terrain_tile::tile_bounds(z, x, y);
        let [bw, bs, be, bn] = self.bounds;
        if !(w < be && e > bw && s < bn && n > bs) {
            return Ok(None);
        }
        let dem = maker.dem();
        // The depth at each pixel: positive for a dry cut, negative for a dredge, zero for none.
        let depths = maker.at_pixels(z, x, y, 0, 0.0, |px, py| {
            let ground = dem.sample_bilinear(px, py);
            match self.at_dem(px, py, ground) {
                Some(c) if c.kind == "dredge" => -c.depth_m,
                Some(c) => c.depth_m,
                None => 0.0,
            }
        });
        if depths.iter().all(|d| *d == 0.0) {
            return Ok(None);
        }
        let mut px = Vec::with_capacity(TILE * TILE * 4);
        for d in depths {
            if d == 0.0 {
                px.extend_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            let class = depth_class(f64::from(d.abs()));
            let c = if d < 0.0 {
                DREDGE_COLOURS[class]
            } else {
                DRY_COLOURS[class]
            };
            px.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
        let mut png = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut png, TILE as u32, TILE as u32);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_compression(png::Compression::Fast);
            let input = |e: png::EncodingError| DesignError::Input(e.to_string());
            let mut writer = enc.write_header().map_err(input)?;
            writer.write_image_data(&px).map_err(input)?;
        }
        Ok(Some(png))
    }
}

impl Overlay for CutSurface {
    fn bounds(&self) -> [f64; 4] {
        self.bounds
    }

    fn level(&self, x: f64, y: f64, ground: f64) -> f64 {
        self.at_dem(x, y, ground).map_or(ground, |c| c.after_m)
    }
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;

    /// A straight channel of `n` stations 25 m apart, cut 60 m each side.
    fn long_channel(n: usize) -> Channel {
        let stations = (0..n)
            .map(|i| ChannelStation {
                chainage_m: i as f64 * 25.0,
                x: 300_000.0 + i as f64 * 25.0 * 0.8,
                y: 600_000.0 + i as f64 * 25.0 * 0.6,
                heading_rad: libm::atan2(0.6, 0.8),
                half_width_m: 20.0,
                water_level_m: 10.0,
                invert_m: 7.0,
                bench_left: true,
                bench_right: true,
                top_of_cut_left_m: Some(-60.0),
                top_of_cut_right_m: Some(60.0),
                daylight_left_m: Some(-60.5),
                daylight_right_m: Some(60.5),
                dredge_area_m2: 100.0,
                excav_area_m2: 900.0,
                max_face_h_m: 12.0,
                on_dem: true,
            })
            .collect();
        Channel {
            option_id: "line".into(),
            stations,
            wet_slope: 3.0,
            dx_m: 1.0,
            maxw_m: 200.0,
            from_volumes: true,
            has_ground: true,
        }
    }

    #[test]
    fn the_cut_surface_of_a_200_km_river_is_small_and_quick_to_make_and_to_read() {
        let utm = Utm {
            zone: 30,
            south: false,
        };
        let channel = long_channel(8_000);
        let made = std::time::Instant::now();
        let cut = CutSurface::new(&channel, utm, None, PolygonIndex::new(&[]), None).unwrap();
        let made = made.elapsed();
        // The stations and the index: under 3 MB, whatever the DEM.
        assert!(
            cut.memory_bytes() < 3_000_000,
            "{} bytes",
            cut.memory_bytes()
        );
        // A tile's worth of points across the channel: every one answers, and the face is the rule's.
        let read = std::time::Instant::now();
        let mut hits = 0;
        for k in 0..65_536 {
            let (s, o) = (1_000.0 + (k / 256) as f64 * 4.0, (k % 256) as f64 - 128.0);
            let p = [300_000.0 + s * 0.8 - o * 0.6, 600_000.0 + s * 0.6 + o * 0.8];
            if let Some(c) = cut.at(p, 30.0) {
                hits += 1;
                assert!((c.offset_m - o).abs() < 1e-6 && (c.chainage_m - s).abs() < 1e-6);
                let face = cut_model_a_face(o.abs(), true, 20.0, 7.0, 10.0, 3.0, 3.0);
                assert!((c.after_m - face).abs() < 1e-6);
            } else {
                assert!(
                    o.abs() > 60.5
                        || cut_model_a_face(o.abs(), true, 20.0, 7.0, 10.0, 3.0, 3.0) >= 30.0
                );
            }
        }
        let read = read.elapsed();
        assert!(hits > 20_000, "{hits}");
        println!(
            "cut surface of 8,000 stations: {} bytes, made in {made:?}; 65,536 points read in {read:?}",
            cut.memory_bytes()
        );
    }

    #[test]
    fn the_depth_classes_are_in_order_and_each_has_two_colours() {
        assert!(DEPTH_CLASSES_M.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(depth_class(0.0), 0);
        assert_eq!(depth_class(1.0), 1);
        assert_eq!(depth_class(19.99), 4);
        assert_eq!(depth_class(20.0), 5);
        assert_ne!(DREDGE_COLOURS[5], DRY_COLOURS[5]);
    }
}
