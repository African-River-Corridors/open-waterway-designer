//! The long section and the cross-section (spec 0003): the stations of the design line, shaped for
//! drawing. They are the volumes stage's stored stations (spec 0017 ticket 10): what is drawn is what
//! the table counts, and nothing walks the terrain again while the stored record is fresh and holds
//! what a sheet needs.
//!
//! [`Session::section_stations`] is the one door the drawings, the report and the web app go
//! through. It gives the stored stations when the record is fresh and keeps the section profile of
//! every station asked for. A station whose profile was not kept (a sheet drawn at another section
//! spacing, or a station picked by hand) is never drawn from a neighbour's profile: the line is
//! walked again on the terrain, the same walk the volumes stage summed, and the stations say so
//! ([`StationSource::Walked`]). With no terrain to walk, it refuses by name.

use super::crs::{Crs, Utm};
use super::export::SCREENING_LABEL;
use super::session::{DesignError, Session, Stage, StageStatus};
use super::volumes_stage::{Volumes, top_of_cut};
use crate::types::TerrainSampler;
use owd_core::geom::Pt;
use owd_core::volumes::sections::{CrossSection, SectionVolumes};
use serde_json::{Value, json};

/// Where a set of stations came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StationSource {
    /// The volumes stage's stored record, read as it is.
    Stored,
    /// A new walk of the line on the terrain, and why: a station asked for had no stored profile
    /// ([`WALKED_FOR_PROFILE`]), or the record holds no stations ([`WALKED_NO_STATIONS`]).
    Walked { reason: &'static str },
}

/// Why a sheet was drawn from a new walk: a section it draws has no stored profile.
pub const WALKED_FOR_PROFILE: &str = "the volumes stage keeps the section profile only at the \
    stations the default sheet style draws (every 500 m, the tight bends and the worst stations): \
    the sections here are drawn from the line walked again on the DEM, the same walk the volumes \
    stage summed";

/// Why a sheet was drawn from a new walk: the volumes record holds no stations.
pub const WALKED_NO_STATIONS: &str = "the volumes stage's record holds no stations (it was saved \
    before OWD stored them): this is drawn from the line walked again on the DEM; run the volumes \
    stage again to store them";

/// Why a sheet was drawn from a new walk: the record's stations were saved without the top of cut
/// (by PR 564's format, before each station kept it). Read as they are, the plan would draw no
/// top-of-cut lines.
pub const WALKED_STATIONS_INCOMPLETE: &str = "the volumes stage's stored stations have no top of \
    cut (they were saved before OWD stored it): this is drawn from the line walked again on the \
    DEM; run the volumes stage again to store it";

/// True when every stored station holds both top-of-cut keys. A key that is present but `null` is a
/// station with no top of cut on that side; a key that is missing is a record saved before OWD kept
/// it.
fn stations_complete(sections: &Value) -> bool {
    sections["stations"].as_array().is_some_and(|a| {
        a.iter()
            .all(|s| s.get("top_of_cut_left_m").is_some() && s.get("top_of_cut_right_m").is_some())
    })
}

/// The stations of the design line, as the drawings and the report read them.
///
/// `walk` holds every station. Where the source is [`StationSource::Stored`], a station whose profile
/// was not kept has empty `offsets`, `terrain`, `wet` and `cutface`: [`Stations::has_profile`] says
/// which. The ground on the line and the top of cut are kept for every station, beside the walk.
#[derive(Debug, Clone, PartialEq)]
pub struct Stations {
    pub walk: SectionVolumes,
    /// The ground on the line at each station (NaN where unknown).
    pub bed_m: Vec<f64>,
    /// The top of cut each side of each station, as offsets (left negative).
    pub top_of_cut: Vec<(Option<f64>, Option<f64>)>,
    pub source: StationSource,
}

impl From<SectionVolumes> for Stations {
    /// The stations of a walk: every station has its profile.
    fn from(walk: SectionVolumes) -> Self {
        let bed_m = walk.stations.iter().map(bed).collect();
        let top_of_cut = walk.stations.iter().map(top_of_cut).collect();
        Stations {
            walk,
            bed_m,
            top_of_cut,
            source: StationSource::Walked {
                reason: "walked on the terrain",
            },
        }
    }
}

impl Stations {
    /// The stored stations of the volumes stage, under its cut model.
    pub fn from_stored(v: &Volumes, cut_model: &str) -> Self {
        let model = owd_core::cut_model::normalise_model(cut_model);
        let nan = |x: Option<f64>| x.unwrap_or(f64::NAN);
        let stations = v
            .stations
            .iter()
            .map(|st| {
                let (offsets, terrain, wet, cutface) = match &st.profile {
                    Some(p) => (
                        p.offsets_m.clone(),
                        p.terrain_m.iter().copied().map(nan).collect(),
                        p.wet.clone(),
                        p.cut_face_m.iter().copied().map(nan).collect(),
                    ),
                    None => Default::default(),
                };
                CrossSection {
                    chainage_m: st.chainage_m,
                    x: st.x,
                    y: st.y,
                    heading_rad: st.heading_rad,
                    offsets,
                    terrain,
                    wet,
                    hb: st.half_width_m,
                    wl: st.water_level_m,
                    grade: st.invert_m,
                    depth: st.depth_m,
                    model: model.clone(),
                    cutface,
                    dredge_area_m2: st.dredge_area_m2,
                    excav_area_m2: st.excav_area_m2,
                    max_face_h_m: st.max_face_h_m,
                    bench_left: st.bench_left,
                    bench_right: st.bench_right,
                    tag: st.tag.clone(),
                    has_water: st.has_water,
                }
            })
            .collect();
        Stations {
            walk: SectionVolumes {
                stations,
                station_m: v.station_m,
            },
            bed_m: v.stations.iter().map(|st| nan(st.bed_m)).collect(),
            top_of_cut: v
                .stations
                .iter()
                .map(|st| (st.top_of_cut_left_m, st.top_of_cut_right_m))
                .collect(),
            source: StationSource::Stored,
        }
    }

    /// True when station `i` has its section profile.
    pub fn has_profile(&self, i: usize) -> bool {
        self.walk
            .stations
            .get(i)
            .is_some_and(|s| !s.offsets.is_empty())
    }

    /// The index of the station nearest `chainage_m`: the first of two equally near.
    pub fn nearest(&self, chainage_m: f64) -> Option<usize> {
        let st = &self.walk.stations;
        (0..st.len()).min_by(|&a, &b| {
            (st[a].chainage_m - chainage_m)
                .abs()
                .total_cmp(&(st[b].chainage_m - chainage_m).abs())
        })
    }

    /// True when the station nearest each of `chainages` has its profile.
    pub fn profiles_at(&self, chainages: &[f64]) -> bool {
        chainages
            .iter()
            .filter(|c| c.is_finite())
            .all(|&c| self.nearest(c).is_some_and(|i| self.has_profile(i)))
    }
}

impl Session {
    /// The station walk of the option `option_id`, on the DEM: the line walked again. Needs the
    /// volumes stage fresh: the walk must be the one whose sums the table shows. The drawings and
    /// the report read [`Session::section_stations`]; this is the walk it falls back on.
    pub fn option_section_walk(
        &self,
        option_id: &str,
        terrain: TerrainSampler,
    ) -> Result<SectionVolumes, DesignError> {
        self.volumes_fresh()?;
        let lines = self.design_lines();
        let option = lines
            .iter()
            .find(|o| o["id"] == option_id)
            .ok_or_else(|| DesignError::Input(format!("there is no design line `{option_id}`")))?;
        let plan = self.section_plan(&Value::Null)?;
        self.option_sections(&plan, option, terrain)
    }

    /// Refuse unless the volumes stage is fresh: a stale record is not drawn as fresh.
    fn volumes_fresh(&self) -> Result<(), DesignError> {
        match self.stage_status(Stage::Volumes) {
            StageStatus::Fresh => Ok(()),
            StageStatus::Stale => Err(DesignError::StaleUpstream {
                stage: Stage::Volumes,
                run_first: Stage::Volumes,
            }),
            StageStatus::NotRun => Err(DesignError::Input(
                "run the volumes stage first: the sections are drawn from its station walk".into(),
            )),
        }
    }

    /// The volumes stage's stored stations for `option_id`, read as they are, or `None` when the
    /// fresh record holds none (it was saved before OWD stored them) or holds stations with no top
    /// of cut (saved before OWD stored that). Refuses when the volumes
    /// stage is stale or has not run, or ran on another line.
    pub fn stored_stations(&self, option_id: &str) -> Result<Option<Stations>, DesignError> {
        self.volumes_fresh()?;
        let res = &self.doc["stages"]["volumes"]["results"];
        if res["id"].as_str().is_some_and(|id| id != option_id) {
            return Err(DesignError::Input(format!(
                "the volumes stage ran on the design line `{}`, not `{option_id}`",
                res["id"].as_str().unwrap_or("?")
            )));
        }
        if res["sections"].is_null() || !stations_complete(&res["sections"]) {
            return Ok(None);
        }
        let v: Volumes = serde_json::from_value(res["sections"].clone()).map_err(|e| {
            DesignError::Input(format!(
                "the volumes stage's stored stations do not read: {e}. Run the volumes stage again"
            ))
        })?;
        let model = self.volumes_settings(&Value::Null)?.cut_model;
        Ok(Some(Stations::from_stored(&v, &model)))
    }

    /// The stations of `option_id` for a drawing or a report, with the section profile of the
    /// station nearest each of `profiles_at`: the stored stations when the fresh record holds them
    /// and those profiles, else the line walked again on `terrain`. With no terrain for a walk, it
    /// refuses and says why. The drawings, the report and the web app all call this.
    pub fn section_stations(
        &self,
        option_id: &str,
        profiles_at: &[f64],
        terrain: Option<TerrainSampler>,
    ) -> Result<Stations, DesignError> {
        let stored = match self.stored_stations(option_id)? {
            Some(st) if st.profiles_at(profiles_at) => return Ok(st),
            stored => stored,
        };
        let incomplete =
            stored.is_none() && !self.doc["stages"]["volumes"]["results"]["sections"].is_null();
        let Some(terrain) = terrain else {
            return Err(DesignError::Input(match stored {
                None if incomplete => "the volumes stage's stored stations have no top of cut \
                                       (they were saved before OWD stored it): run the volumes \
                                       stage again, or give the DEM to walk the line"
                    .into(),
                None => "the volumes stage's record holds no stations (it was saved before OWD \
                         stored them): run the volumes stage again, or give the DEM to walk the line"
                    .into(),
                Some(_) => "the volumes stage keeps the section profile only at the stations a \
                            cross-section sheet draws by default: a section at another station \
                            needs the DEM to walk the line again"
                    .into(),
            }));
        };
        let mut walked = Stations::from(self.option_section_walk(option_id, terrain)?);
        walked.source = StationSource::Walked {
            reason: if stored.is_some() {
                WALKED_FOR_PROFILE
            } else if incomplete {
                WALKED_STATIONS_INCOMPLETE
            } else {
                WALKED_NO_STATIONS
            },
        };
        Ok(walked)
    }

    fn utm(&self) -> Result<Utm, DesignError> {
        self.doc["project"]["crs"]["epsg"]
            .as_u64()
            .and_then(|c| Crs::from_epsg(c as u32).ok())
            .and_then(|c| match c {
                Crs::Utm(u) => Some(u),
                Crs::Wgs84 => None,
            })
            .ok_or_else(|| DesignError::Input("the channel design has no project CRS".into()))
    }

    /// The long section as series along the chainage of the option's line: the bed on the line,
    /// the water level (steps are the pools), the design invert and the cut depth below the bed,
    /// with the station's quantities and its place on the map (`[lon, lat]`).
    pub fn long_section(&self, option_id: &str, stations: &Stations) -> Result<Value, DesignError> {
        let walk = &stations.walk;
        let xy: Vec<Pt> = walk.stations.iter().map(|s| [s.x, s.y]).collect();
        let positions = self.utm()?.inverse_all(&xy).map_err(DesignError::Input)?;
        let col = |f: &dyn Fn(&CrossSection) -> f64| -> Vec<Value> {
            walk.stations.iter().map(|s| json!(f(s))).collect()
        };
        Ok(self.with_depth_basis(json!({
            "screening": SCREENING_LABEL,
            "option_id": option_id,
            "station_m": walk.station_m,
            "chainage_m": col(&|s| s.chainage_m),
            "bed_m": stations.bed_m,
            "water_level_m": col(&|s| s.wl),
            "design_invert_m": col(&|s| s.grade),
            "cut_depth_m": walk
                .stations
                .iter()
                .zip(&stations.bed_m)
                .map(|(s, b)| json!((b - s.grade).max(0.0)))
                .collect::<Vec<_>>(),
            "dredge_area_m2": col(&|s| s.dredge_area_m2),
            "excav_area_m2": col(&|s| s.excav_area_m2),
            "in_river_area_m2": col(&|s| s.in_river_area_m2()),
            "widening_area_m2": col(&|s| s.widening_area_m2()),
            "cut_through_area_m2": col(&|s| s.cut_through_area_m2()),
            "max_face_h_m": col(&|s| s.max_face_h_m),
            "positions": positions,
        })))
    }

    /// The cross-section at the station nearest to `chainage_m`. A station whose profile was not
    /// kept is refused, never drawn from a neighbour: ask [`Session::section_stations`] for it.
    pub fn cross_section(
        &self,
        stations: &Stations,
        chainage_m: f64,
    ) -> Result<Value, DesignError> {
        let walk = &stations.walk;
        let s = nearest(walk, |s| (s.chainage_m - chainage_m).abs())?;
        if s.offsets.is_empty() {
            return Err(DesignError::Input(format!(
                "the station at {:.1} m has no stored section profile: walk the line again for it",
                s.chainage_m
            )));
        }
        let [lon, lat] = self.utm()?.inverse(s.x, s.y).map_err(DesignError::Input)?;
        Ok(self.with_depth_basis(json!({
            "screening": SCREENING_LABEL,
            "chainage_m": s.chainage_m,
            "position": [lon, lat],
            "offsets_m": s.offsets,
            "terrain_m": s.terrain,
            "wet": s.wet,
            "cut_face_m": s.cutface,
            "water_level_m": s.wl,
            "design_invert_m": s.grade,
            "half_width_m": s.hb,
            "depth_m": s.depth,
            "model": s.model,
            "dredge_area_m2": s.dredge_area_m2,
            "excav_area_m2": s.excav_area_m2,
            "in_river_area_m2": s.in_river_area_m2(),
            "widening_area_m2": s.widening_area_m2(),
            "cut_through_area_m2": s.cut_through_area_m2(),
            "max_face_h_m": s.max_face_h_m,
            "bench_left": s.bench_left,
            "bench_right": s.bench_right,
        })))
    }

    /// The chainage of the station nearest to a point picked on the map (`lon`, `lat`).
    pub fn station_near(
        &self,
        stations: &Stations,
        lon: f64,
        lat: f64,
    ) -> Result<f64, DesignError> {
        let [x, y] = self.utm()?.forward(lon, lat).map_err(DesignError::Input)?;
        nearest(&stations.walk, |s| libm::hypot(s.x - x, s.y - y)).map(|s| s.chainage_m)
    }
}

/// The terrain on the option's line: the sample at the offset nearest zero.
fn bed(s: &CrossSection) -> f64 {
    s.offsets
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .map_or(f64::NAN, |(i, _)| s.terrain[i])
}

fn nearest(
    walk: &SectionVolumes,
    dist: impl Fn(&CrossSection) -> f64,
) -> Result<&CrossSection, DesignError> {
    walk.stations
        .iter()
        .filter(|s| dist(s).is_finite())
        .min_by(|a, b| dist(a).total_cmp(&dist(b)))
        .ok_or_else(|| DesignError::Input("the option has no stations".into()))
}
