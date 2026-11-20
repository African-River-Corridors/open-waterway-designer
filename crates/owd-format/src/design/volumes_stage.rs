//! The volumes stage's typed output (spec 0017, ADR 0015, ticket 9): its stations, its volumes by
//! reach and its totals, so drawings and reports read them instead of walking the line again.
//!
//! [`volumes_stage`] is a pure function from a [`VolumesIn`] and the terrain sampler to a
//! [`Volumes`]. The session's volumes run walks the line with the same code ([`walk`]) and stores
//! the same [`Volumes`] as `stages.volumes.results.sections`, beside the numbers it has always
//! stored.
//!
//! An island (a hole in the river polygon) is dry land: its samples are not wet, so their cut is dry
//! cut, not dredge. The river polygon reaches the walk with its holes (ticket 3).
//!
//! **What a station keeps.** Every station keeps its numbers: chainage, place, half-width, water
//! level, invert, depth, the bed on the line, the areas by kind, the tallest face and the benches.
//! That is all the long section draws. The section profile (offsets, terrain, wet, cut face: about
//! 400 samples) is kept only at the stations a cross-section sheet draws by default: the station
//! nearest each multiple of [`PROFILE_EVERY_M`] (the default style's section spacing), and the
//! station nearest each chainage in `profile_at` (the session passes the middles of the tight bends
//! and the worst station of each constraint stretch, the default style's extra sections).

use super::sizing::ChannelSize;
use super::volumes::VolumesSettings;
use super::water_stage::{DesignWater, WaterIn, WaterSource, level_reader, water};
use crate::types::{Chainage, Polyline, RiverPolygon, StageError, TerrainSampler};
use owd_core::cut_model::{WIDEN_RMAX, widened_hb_from_shape};
use owd_core::geom::{Polygon, Pt};
use owd_core::volumes::sections::{
    CrossSection, HalfWidth, Level, SectionVolumes, section_volumes,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The stage's name, as the document stores it.
pub const STAGE: &str = "volumes";

/// The spacing of the stations whose section profile is kept, in metres of design-line chainage:
/// the default drawing style's cross-section spacing (`sections.spacing_m`). An OWD choice, not a
/// standard.
pub const PROFILE_EVERY_M: f64 = 500.0;

/// What the volumes stage reads.
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct VolumesIn {
    /// The design line the volumes are taken along.
    pub design_line: Polyline,
    /// The channel's size, from the sizing stage.
    pub size: ChannelSize,
    /// What the water stage reads: the level at each station is read as it reads it.
    pub water: WaterIn,
    /// The river as water, with its islands.
    #[serde(default)]
    pub river_polygon: Option<RiverPolygon>,
    /// The stage's settings.
    #[serde(default)]
    pub settings: VolumesSettings,
    /// Chainages (design line) whose nearest station keeps its section profile, beside one every
    /// [`PROFILE_EVERY_M`].
    #[serde(default)]
    pub profile_at: Vec<f64>,
}

/// A station's cross-section, as the cross-section sheet draws it. Missing terrain or cut face is
/// `null`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SectionProfile {
    /// Offsets across the line, metres (left negative).
    pub offsets_m: Vec<f64>,
    /// The ground (or bed) at each offset.
    pub terrain_m: Vec<Option<f64>>,
    /// True inside the present river.
    pub wet: Vec<bool>,
    /// The design face at each offset, where there is a cut face.
    pub cut_face_m: Vec<Option<f64>>,
}

/// One station of the walk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct VolumeStation {
    /// Design-line chainage.
    pub chainage_m: f64,
    /// The station's place in the project CRS.
    pub x: f64,
    pub y: f64,
    /// The line's heading at the station, radians.
    pub heading_rad: f64,
    /// The bottom half-width.
    pub half_width_m: f64,
    /// The design water level.
    pub water_level_m: f64,
    /// The design invert.
    pub invert_m: f64,
    /// The design depth.
    pub depth_m: f64,
    /// The ground on the line (the sample nearest offset zero), when known.
    pub bed_m: Option<f64>,
    pub dredge_area_m2: f64,
    pub excav_area_m2: f64,
    pub in_river_area_m2: f64,
    pub widening_area_m2: f64,
    pub cut_through_area_m2: f64,
    pub max_face_h_m: f64,
    pub bench_left: bool,
    pub bench_right: bool,
    /// True when the section has water of its own (ticket C23).
    pub has_water: bool,
    /// The section's tag.
    pub tag: String,
    /// The section profile, at the stations that keep one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<SectionProfile>,
}

/// The volumes of one reach: a stretch of the water stage's (one source, one pool).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ReachVolumes {
    /// Design-line chainage where the reach starts.
    pub from: Chainage,
    /// Design-line chainage where it ends.
    pub to: Chainage,
    /// Where its water level comes from.
    pub source: WaterSource,
    /// The pool, by its index in the water stage's input, when it is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pool: Option<usize>,
    /// The number of stations in it. A station at the boundary of two reaches is in the upper one.
    pub stations: usize,
    pub dredge_m3: f64,
    pub excav_m3: f64,
    pub total_m3: f64,
}

/// The totals over every station.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct VolumeTotals {
    pub dredge_m3: f64,
    pub excav_m3: f64,
    pub total_m3: f64,
    pub in_river_m3: f64,
    pub widening_m3: f64,
    pub cut_through_m3: f64,
    pub max_face_h_m: f64,
    pub stations: usize,
}

/// What the volumes stage returns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Volumes {
    /// The station step, metres: each station stands for this length.
    pub station_m: f64,
    /// Every station, in order of chainage.
    pub stations: Vec<VolumeStation>,
    /// The volumes by reach, in order.
    pub by_reach: Vec<ReachVolumes>,
    /// The totals.
    pub totals: VolumeTotals,
}

fn refuse(code: &str, message: &str) -> StageError {
    StageError::new(STAGE, code, message)
}

/// The station walk along `line`: the core's `section_volumes` with the settings, the size, the
/// river and the level at each station. The session's walk and the pure stage both call it.
///
/// Refusals: `VOLUMES_CUT_MODEL` (a cut model the core does not know), `VOLUMES_NO_ELEVATION` (no
/// station has a height at any sample: the terrain does not reach the line; at the first station),
/// `VOLUMES_OUTSIDE_RASTER` (a station with no height at any sample, at its design-line chainage),
/// and `VOLUMES_WALK` for what is left (a terrain sampler that breaks its contract).
pub(super) fn walk(
    line: &[Pt],
    size: &ChannelSize,
    river: &[Polygon],
    level: &dyn Fn(f64) -> f64,
    settings: &VolumesSettings,
    terrain: TerrainSampler,
) -> Result<SectionVolumes, StageError> {
    let model = owd_core::cut_model::normalise_model(&settings.cut_model);
    if model != owd_core::cut_model::CUT_MODEL && model != owd_core::cut_model::CUT_MODEL_B {
        return Err(refuse(
            "VOLUMES_CUT_MODEL",
            &format!(
                "the cut model `{}` is not one OWD knows: use `a` or `b`",
                settings.cut_model
            ),
        ));
    }
    // A fixed half-width needs no channel shape: read the shape only when the bends widen it.
    let engine_shape;
    let from_shape;
    let half_width = match settings.half_width_m {
        Some(h) => HalfWidth::Fixed(h),
        None => {
            engine_shape = size.shape()?;
            from_shape = |xy: &[Pt]| widened_hb_from_shape(xy, engine_shape, WIDEN_RMAX);
            HalfWidth::FromStations(&from_shape)
        }
    };
    let v = section_volumes(
        line,
        terrain,
        river,
        &Level::AtChainage(level),
        size.design_depth_m,
        &half_width,
        &settings.cut_model,
        settings.wet_slope,
        settings.station_m,
        settings.maxw_m,
        settings.dx_m,
        None,
        None,
        owd_core::volumes::sections::DEFAULT_CHUNK,
    )
    .map_err(|e| refuse("VOLUMES_WALK", &e.to_string()))?;
    let blind = |s: &CrossSection| s.terrain.iter().all(|z| !z.is_finite());
    if let Some(first) = v.stations.first()
        && v.stations.iter().all(blind)
    {
        return Err(refuse(
            "VOLUMES_NO_ELEVATION",
            "the terrain has no height anywhere along the design line: the DEM does not reach it",
        )
        .at(Chainage(first.chainage_m)));
    }
    if let Some(s) = v.stations.iter().find(|s| blind(s)) {
        return Err(refuse(
            "VOLUMES_OUTSIDE_RASTER",
            &format!(
                "the station at {:.1} m of the design line is outside the DEM: no height at any \
                 point of its section",
                s.chainage_m
            ),
        )
        .at(Chainage(s.chainage_m)));
    }
    Ok(v)
}

fn opt(v: f64) -> Option<f64> {
    v.is_finite().then_some(v)
}

/// The terrain on the line: the sample at the offset nearest zero.
fn bed(s: &CrossSection) -> f64 {
    s.offsets
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .map_or(f64::NAN, |(i, _)| s.terrain[i])
}

fn profile(s: &CrossSection) -> SectionProfile {
    SectionProfile {
        offsets_m: s.offsets.clone(),
        terrain_m: s.terrain.iter().copied().map(opt).collect(),
        wet: s.wet.clone(),
        cut_face_m: s.cutface.iter().copied().map(opt).collect(),
    }
}

/// The index of the station nearest `c`: the first of two equally near (as the gallery picks).
fn nearest(st: &[CrossSection], c: f64) -> Option<usize> {
    (0..st.len()).min_by(|&a, &b| {
        (st[a].chainage_m - c)
            .abs()
            .total_cmp(&(st[b].chainage_m - c).abs())
    })
}

/// The stations that keep their profile: nearest each multiple of [`PROFILE_EVERY_M`] up to the
/// last station, and nearest each of `profile_at`.
fn profiled(st: &[CrossSection], profile_at: &[f64]) -> std::collections::BTreeSet<usize> {
    let mut keep = std::collections::BTreeSet::new();
    let Some(last) = st.last().map(|s| s.chainage_m) else {
        return keep;
    };
    let mut i = 0usize;
    loop {
        let at = i as f64 * PROFILE_EVERY_M;
        if at > last + 1e-9 {
            break;
        }
        keep.extend(nearest(st, at));
        i += 1;
    }
    for &c in profile_at {
        if c.is_finite() {
            keep.extend(nearest(st, c));
        }
    }
    keep
}

/// The output of a walk: its stations (with profiles where kept), its volumes by the water stage's
/// stretches, and its totals.
pub(super) fn summarise(v: &SectionVolumes, water: &DesignWater, profile_at: &[f64]) -> Volumes {
    let keep = profiled(&v.stations, profile_at);
    let stations: Vec<VolumeStation> = v
        .stations
        .iter()
        .enumerate()
        .map(|(i, s)| VolumeStation {
            chainage_m: s.chainage_m,
            x: s.x,
            y: s.y,
            heading_rad: s.heading_rad,
            half_width_m: s.hb,
            water_level_m: s.wl,
            invert_m: s.grade,
            depth_m: s.depth,
            bed_m: opt(bed(s)),
            dredge_area_m2: s.dredge_area_m2,
            excav_area_m2: s.excav_area_m2,
            in_river_area_m2: s.in_river_area_m2(),
            widening_area_m2: s.widening_area_m2(),
            cut_through_area_m2: s.cut_through_area_m2(),
            max_face_h_m: s.max_face_h_m,
            bench_left: s.bench_left,
            bench_right: s.bench_right,
            has_water: s.has_water,
            tag: s.tag.clone(),
            profile: keep.contains(&i).then(|| profile(s)),
        })
        .collect();

    // Each station in the last stretch that starts at or before it (the upper one at a boundary).
    let stretches = &water.stretches;
    let reach_of = |c: f64| -> usize { stretches.iter().rposition(|r| r.from.0 <= c).unwrap_or(0) };
    let mut by_reach: Vec<ReachVolumes> = stretches
        .iter()
        .map(|r| ReachVolumes {
            from: r.from,
            to: r.to,
            source: r.source,
            pool: r.pool,
            stations: 0,
            dredge_m3: 0.0,
            excav_m3: 0.0,
            total_m3: 0.0,
        })
        .collect();
    if !by_reach.is_empty() {
        let mut dredge: Vec<Vec<f64>> = vec![vec![]; by_reach.len()];
        let mut excav: Vec<Vec<f64>> = vec![vec![]; by_reach.len()];
        for s in &v.stations {
            let k = reach_of(s.chainage_m);
            dredge[k].push(s.dredge_area_m2);
            excav[k].push(s.excav_area_m2);
        }
        for (k, r) in by_reach.iter_mut().enumerate() {
            r.stations = dredge[k].len();
            r.dredge_m3 =
                owd_core::alignment::bends::py_sum(dredge[k].iter().copied()) * v.station_m;
            r.excav_m3 = owd_core::alignment::bends::py_sum(excav[k].iter().copied()) * v.station_m;
            r.total_m3 = r.dredge_m3 + r.excav_m3;
        }
        // A stretch with no station (two points at one chainage) is not a reach.
        by_reach.retain(|r| r.stations > 0);
    }
    Volumes {
        station_m: v.station_m,
        totals: VolumeTotals {
            dredge_m3: v.dredge_m3(),
            excav_m3: v.excav_m3(),
            total_m3: v.total_m3(),
            in_river_m3: v.in_river_m3(),
            widening_m3: v.widening_m3(),
            cut_through_m3: v.cut_through_m3(),
            max_face_h_m: v.max_face_h_m(),
            stations: v.stations.len(),
        },
        stations,
        by_reach,
    }
}

/// The volumes stage: the walk along the design line, kept as stations, by reach and in total.
pub fn volumes_stage(input: &VolumesIn, terrain: TerrainSampler) -> Result<Volumes, StageError> {
    if input.design_line.len() < 2 {
        return Err(refuse(
            "VOLUMES_NO_LINE",
            "the volumes are taken along the design line: it needs two points",
        ));
    }
    let w = water(&input.water).map_err(|e| refuse("VOLUMES_NO_WATER", &e.message))?;
    let river: Vec<Polygon> = input
        .river_polygon
        .iter()
        .filter(|p| !p.shell.is_empty())
        .map(RiverPolygon::to_polygon)
        .collect();
    let level = level_reader(&input.water).map_err(|e| refuse("VOLUMES_NO_WATER", &e.message))?;
    let v = walk(
        &input.design_line.0,
        &input.size,
        &river,
        &level,
        &input.settings,
        terrain,
    )?;
    Ok(summarise(&v, &w, &input.profile_at))
}
