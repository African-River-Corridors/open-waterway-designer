//! The volumes stage and the DEM reference (spec 0002). The DEM is named by relative path, SHA-256,
//! CRS and nodata value; its bytes are never in the document. The stage gives the design line (spec
//! 0017: one per run) its dredge, dry cut and tallest face by calling the core's `section_volumes` on the DEM,
//! under the cut model chosen in the settings.

use super::geojson::all_positions;
use super::hash::sha256_hex;
use super::session::{DesignError, ENGINE_VERSION, Session, Stage, StageStatus};
use crate::geotiff::Dem;
use crate::types::{TerrainSampler, WaterProfile};
use owd_core::cut_model::CUT_MODEL;
use owd_core::geom::{Polygon, Pt};
use owd_core::volumes::sections::{
    DEFAULT_DX_M, DEFAULT_MAXW_M, DEFAULT_STATION_M, DEFAULT_WET_SLOPE,
};
use serde_json::{Value, json};

/// How the DEM file the caller found compares with the document's reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemCheck {
    /// The document names no DEM.
    NoDem,
    /// The file is the one whose hash was recorded.
    Matches,
    /// No file at the recorded path.
    Missing,
    /// A file whose hash differs from the recorded one.
    Changed,
}

/// The settings of the volumes stage, typed. A key the stage does not know is refused by name.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VolumesSettings {
    /// The cut model (`a` or `b`).
    pub cut_model: String,
    /// The station spacing in metres.
    pub station_m: f64,
    /// The half-width of each cross-section in metres.
    pub maxw_m: f64,
    /// The sample spacing across each cross-section in metres.
    pub dx_m: f64,
    /// The slope of the cut below water, horizontal per vertical.
    pub wet_slope: f64,
    /// A fixed bottom half-width in metres; `None`: widened in bends from the sizing.
    pub half_width_m: Option<f64>,
}

impl Default for VolumesSettings {
    fn default() -> Self {
        VolumesSettings {
            cut_model: CUT_MODEL.to_string(),
            station_m: DEFAULT_STATION_M,
            maxw_m: DEFAULT_MAXW_M,
            dx_m: DEFAULT_DX_M,
            wet_slope: DEFAULT_WET_SLOPE,
            half_width_m: None,
        }
    }
}

impl VolumesSettings {
    /// The settings as the stage writes them into the document.
    pub fn to_value(&self) -> Value {
        json!({
            "cut_model": self.cut_model,
            "station_m": self.station_m,
            "maxw_m": self.maxw_m,
            "dx_m": self.dx_m,
            "wet_slope": self.wet_slope,
            "half_width_m": self.half_width_m,
        })
    }

    /// Lay the settings of `source` (an object, or null for none) over these. A key the stage does
    /// not know, or a value of the wrong kind, is refused by name; nothing is half applied.
    fn apply(&self, source: &Value) -> Result<VolumesSettings, DesignError> {
        let mut out = self.clone();
        let Some(o) = source.as_object() else {
            return Ok(out);
        };
        for (k, v) in o {
            match k.as_str() {
                "cut_model" => {
                    out.cut_model = v
                        .as_str()
                        .ok_or_else(|| {
                            DesignError::Input("the setting `cut_model` must be text".into())
                        })?
                        .to_string();
                }
                "station_m" => out.station_m = setting(o, k)?,
                "maxw_m" => out.maxw_m = setting(o, k)?,
                "dx_m" => out.dx_m = setting(o, k)?,
                "wet_slope" => out.wet_slope = setting(o, k)?,
                "half_width_m" => {
                    out.half_width_m = match v {
                        Value::Null => None,
                        v => Some(v.as_f64().filter(|h| *h > 0.0).ok_or_else(|| {
                            DesignError::Input(
                                "the setting `half_width_m` must be a positive number of metres or null"
                                    .into(),
                            )
                        })?),
                    };
                }
                other => {
                    return Err(DesignError::Input(format!(
                        "the volumes stage has no setting `{other}`: its settings are cut_model, \
                         station_m, maxw_m, dx_m, wet_slope and half_width_m"
                    )));
                }
            }
        }
        Ok(out)
    }
}

fn setting(v: &serde_json::Map<String, Value>, key: &str) -> Result<f64, DesignError> {
    v.get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| DesignError::Input(format!("the setting `{key}` must be a number")))
}

impl Session {
    /// The path of the DEM the document names, relative to the document.
    pub fn dem_path(&self) -> Option<&str> {
        self.doc["inputs"]["dem"]["path"].as_str()
    }

    /// Reference a DEM: its path relative to the document, the SHA-256 of its bytes, its CRS and its
    /// nodata value. The bytes are not kept. The stages that depend on the inputs show as stale.
    pub fn set_dem(&mut self, path: &str, bytes: &[u8], dem: &Dem) {
        self.doc["inputs"]["dem"] = json!({
            "path": path,
            "sha256": sha256_hex(bytes),
            "crs": super::terrain::dem_crs_name(dem),
            "nodata": dem.nodata,
        });
    }

    /// Mark the DEM as holding the river bed below the water level (surveyed bathymetry), or take
    /// the mark off (#363). With the mark and a water level, `fits` gives the full answer, reading
    /// the depth from the DEM, even with no bed model. OWD does not check the DEM: the user says so.
    /// A new DEM ([`Session::set_dem`]) clears the mark.
    pub fn set_dem_holds_bathymetry(&mut self, yes: bool) -> Result<(), DesignError> {
        let Some(dem) = self.doc["inputs"]["dem"].as_object_mut() else {
            return Err(DesignError::Input(
                "the design has no DEM to mark: import one with `owd import --dem FILE`".into(),
            ));
        };
        if yes {
            dem.insert("holds_bathymetry".into(), json!(true));
        } else {
            dem.remove("holds_bathymetry");
        }
        Ok(())
    }

    /// Whether the user marked the DEM as holding the river bed.
    pub fn dem_holds_bathymetry(&self) -> bool {
        self.doc["inputs"]["dem"]["holds_bathymetry"] == true
    }

    /// Say where the DEM file now is, relative to the document, when the design is saved to another
    /// folder. The file's hash, CRS and nodata value stay as recorded, and no stage goes stale.
    pub fn set_dem_path(&mut self, path: &str) {
        if self.doc["inputs"]["dem"].is_object() {
            self.doc["inputs"]["dem"]["path"] = json!(path);
        }
    }

    /// Compare the file the caller found at [`Session::dem_path`] with the recorded hash.
    pub fn dem_check(&self, bytes: Option<&[u8]>) -> DemCheck {
        let Some(dem) = self.doc["inputs"]["dem"].as_object() else {
            return DemCheck::NoDem;
        };
        match bytes {
            None => DemCheck::Missing,
            Some(b) if Some(sha256_hex(b).as_str()) == dem["sha256"].as_str() => DemCheck::Matches,
            Some(_) => DemCheck::Changed,
        }
    }

    /// A stage's status given the DEM file the caller found. The recorded hash is in the document, so
    /// a changed or missing file is not visible to [`Session::stage_status`]: here a fresh `volumes`
    /// stage whose DEM does not match shows as stale.
    pub fn stage_status_with_dem(&self, stage: Stage, bytes: Option<&[u8]>) -> StageStatus {
        let status = self.stage_status(stage);
        let dem_bad = matches!(self.dem_check(bytes), DemCheck::Missing | DemCheck::Changed);
        if stage == Stage::Volumes && status == StageStatus::Fresh && dem_bad {
            StageStatus::Stale
        } else {
            status
        }
    }

    /// Every stage's status, given the DEM file the caller found.
    pub fn status_with_dem(&self, bytes: Option<&[u8]>) -> super::session::Status {
        super::session::Status {
            stages: Stage::ALL
                .iter()
                .map(|s| (*s, self.stage_status_with_dem(*s, bytes)))
                .collect(),
        }
    }

    /// Run the volumes stage: dredge, dry cut and tallest face of the design line (the lattice's when
    /// the optimise stage has refined it, else the alignment's).
    ///
    /// The water level is the document's profile along the river centreline (ADR 0006), or the pool
    /// levels of the optimise stage once it has run. `terrain` is the DEM in the project CRS, or `None`
    /// when the caller could not read it, which refuses with the DEM's path.
    pub fn run_volumes(
        &mut self,
        overrides: &Value,
        terrain: Option<TerrainSampler>,
    ) -> Result<(), DesignError> {
        // The settings first, before the terrain or any work: an unknown key changes nothing.
        self.volumes_settings(overrides)?;
        match self.stage_status(Stage::Sizing) {
            StageStatus::Stale => {
                return Err(DesignError::StaleUpstream {
                    stage: Stage::Volumes,
                    run_first: Stage::Sizing,
                });
            }
            StageStatus::NotRun => {
                return Err(DesignError::Input(
                    "the sizing stage has not been run: choose a ruleset and a vessel class first"
                        .into(),
                ));
            }
            StageStatus::Fresh => {}
        }
        for earlier in [Stage::Alignment, Stage::Optimise] {
            if self.stage_status(earlier) == StageStatus::Stale {
                return Err(DesignError::StaleUpstream {
                    stage: Stage::Volumes,
                    run_first: earlier,
                });
            }
        }
        let Some(dem_path) = self.dem_path().map(str::to_string) else {
            return Err(DesignError::Input(
                "the volumes stage needs a DEM: import one first (`owd import --dem FILE`)".into(),
            ));
        };
        let terrain = terrain.ok_or_else(|| {
            DesignError::Input(format!(
                "the DEM `{dem_path}` is missing: the volumes stage cannot run without it"
            ))
        })?;
        let Some(o) = self.design_lines().into_iter().next() else {
            return Err(DesignError::Input(
                "there is no design line: run the alignment stage first".into(),
            ));
        };

        let plan = self.section_plan(overrides)?;
        let id = o["id"].as_str().unwrap_or("?");
        // The cut-rate multiplier (ticket 13): a named raster is read, never taken as 1 in silence.
        let raster = self.stage_multiplier().map_err(|path| {
            DesignError::Input(format!(
                "VOLUMES_NO_MULTIPLIER: the cut-rate multiplier raster `{path}` is not there to \
                 read: put it beside the design, or set it again (`owd constraints --multiplier`)"
            ))
        })?;
        let weight = match &raster {
            Some(r) => Some(
                super::constraints::multiplier_sampler(r, self.multiplier_epsg())
                    .map_err(super::rule_sets::stage_error)?,
            ),
            None => None,
        };
        let v = self.option_sections_weighted(
            &plan,
            &o,
            terrain,
            weight.as_ref().map(|w| w as TerrainSampler),
        )?;
        let screening = self.screening_value(&o, &v, &plan.river, &plan.centre)?;
        let mut row = json!({
            "id": id,
            "name": o["name"],
            "dredge_m3": v.dredge_m3(),
            "excav_m3": v.excav_m3(),
            "total_m3": v.total_m3(),
            // The three kinds of cut (ticket C23); they add up to the total.
            "in_river_m3": v.in_river_m3(),
            "widening_m3": v.widening_m3(),
            "cut_through_m3": v.cut_through_m3(),
            "max_face_h_m": v.max_face_h_m(),
            "stations": v.stations.len(),
            // The design sanity check of the cut: stations far above their neighbours, and
            // stations whose cut is huge against the channel's own section.
            "sanity": {
                "checked": true,
                "warnings": cut_warnings(&v, plan.typed.wet_slope),
            },
            // A failed check stays with the line's numbers.
            "radius_ok": o["checks"]["radius_ok"],
            "water_source": plan.water_source,
            // A, the verdict, and C, the constraint stretches (spec 0007).
            "screening": screening,
        });
        // The stations, by reach and in total (spec 0017 ticket 9), for the drawings and the report.
        // A profile is kept where a cross-section sheet draws by default: see `volumes_stage`.
        let mut profile_at: Vec<f64> = super::map::option_tight_bends(&o)
            .into_iter()
            .map(|(c, _)| c)
            .collect();
        profile_at.extend(
            row["screening"]["constraint_stretches"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|x| x["worst_chainage_m"].as_f64()),
        );
        // Issue #366: a line that is not feasible keeps its numbers, flagged: not a design.
        if super::export::no_feasible_line(&o) {
            row["no_feasible_line"] = json!(true);
            row["note"] = json!(super::alignment::NO_FEASIBLE_LINE);
        }
        if let Some(note) = self.depth_note() {
            row["depth_basis"] = json!(note);
        }
        // Build the whole record on a copy and keep it only when every part is made: a failure in
        // the water stage or the sections leaves the design as it was, not fresh volumes with no
        // stations.
        let mut next = self.clone();
        let st = &mut next.doc["stages"]["volumes"];
        st["settings"] = plan.settings;
        st["results"] = row;
        st["engine_version"] = json!(ENGINE_VERSION);
        let h = next.dependency_hash(Stage::Volumes);
        next.doc["stages"]["volumes"]["depends_on"] = json!(h);
        // The water along the line, stored for the drawings and the report (spec 0017 ticket 8),
        // after the settings, so its step is the stations' step. The walk read the same levels.
        next.run_water()?;
        let water = next.design_water().ok_or_else(|| {
            DesignError::Input("the water stage did not store the water level".into())
        })?;
        next.doc["stages"]["volumes"]["results"]["sections"] =
            json!(super::volumes_stage::summarise(&v, &water, &profile_at));
        *self = next;
        Ok(())
    }
}

/// The design sanity warnings of an option's cut: the spike check (relative) and the extreme check
/// (absolute, against the design channel's own wetted section at each station).
fn cut_warnings(v: &owd_core::volumes::sections::SectionVolumes, wet_slope: f64) -> Vec<String> {
    use owd_core::sanity::{CutStation, cut_spikes, extreme_cuts};
    let cuts: Vec<(f64, f64)> = v
        .stations
        .iter()
        .map(|s| (s.chainage_m, s.dredge_area_m2 + s.excav_area_m2))
        .collect();
    let stations: Vec<CutStation> = v
        .stations
        .iter()
        .map(|s| CutStation {
            chainage_m: s.chainage_m,
            cut_m2: s.dredge_area_m2 + s.excav_area_m2,
            face_h_m: s.max_face_h_m,
            channel_area_m2: owd_core::volumes::sections::trapezoid_area(s.hb, s.depth, wet_slope),
            depth_m: s.depth,
        })
        .collect();
    cut_spikes(&cuts)
        .iter()
        .chain(extreme_cuts(&stations).iter())
        .map(owd_core::sanity::Finding::message)
        .collect()
}

/// What every option's station walk shares: the settings, the water, the sizing and the river.
pub(super) struct SectionPlan {
    pub(super) settings: Value,
    typed: VolumesSettings,
    profile: WaterProfile,
    pub(super) water_source: &'static str,
    pub(super) centre: Vec<Pt>,
    model: String,
    shape: super::sizing::ChannelSize,
    pub(super) river: Vec<Polygon>,
    fixed_hb: Option<f64>,
    /// The bed model, when the document has one (spec 0012), with the water level it hangs from.
    bed: Option<(owd_core::bed::BedModel, WaterProfile)>,
}

impl Session {
    /// Read the volumes settings (the document's, then `overrides`), the water level, the sizing and
    /// the river, once, for the station walk of every option.
    /// The volumes settings: the defaults, then the document's, then `overrides`. A key the stage
    /// does not know, in the document or from the caller, is refused by name.
    pub(super) fn volumes_settings(
        &self,
        overrides: &Value,
    ) -> Result<VolumesSettings, DesignError> {
        VolumesSettings::default()
            .apply(&self.doc["stages"]["volumes"]["settings"])?
            .apply(overrides)
    }

    pub(super) fn section_plan(&self, overrides: &Value) -> Result<SectionPlan, DesignError> {
        // The defaults, then the document's settings, then the caller's: each key checked.
        let typed = self.volumes_settings(overrides)?;
        let settings = typed.to_value();
        let (profile, water_source) = self
            .water_profile()
            .ok_or_else(|| self.needs_water_profile(Stage::Volumes))?;
        let centre = self.river_centreline()?;
        let model = typed.cut_model.clone();

        let shape = self.channel_size()?;

        // A polygon with no outer ring is no river: there is no water to split the cut by.
        let river: Vec<Polygon> = self
            .river_polygons()
            .map(|ps| ps.into_iter().filter(|p| !p.exterior.is_empty()).collect())
            .unwrap_or_default();

        // The bottom half-width: widened in bends from the sizing, unless the settings fix one.
        let fixed_hb = typed.half_width_m;
        let bed = match self.bed_on_river()? {
            Some(b) => {
                let level = self.bed_water_profile().ok_or_else(|| {
                    DesignError::Input("the bed model needs a water level".into())
                })?;
                Some((b, level))
            }
            None => None,
        };
        Ok(SectionPlan {
            settings,
            typed,
            profile,
            water_source,
            centre,
            model,
            shape,
            river,
            fixed_hb,
            bed,
        })
    }

    /// The station walk of one option: every cross-section, with its terrain, water level, design
    /// invert and cut face. The volumes stage sums these; the app draws them.
    pub(super) fn option_sections(
        &self,
        plan: &SectionPlan,
        o: &Value,
        terrain: TerrainSampler,
    ) -> Result<owd_core::volumes::sections::SectionVolumes, DesignError> {
        self.option_sections_weighted(plan, o, terrain, None)
    }

    /// The section of one station of a walk, cut as [`Session::option_sections`] cuts it, from the
    /// station's place, heading, half-width and water level: the terrain (with the bed model, when
    /// the document has one) is read across this one section only. Given a station of the walk, it
    /// is the walk's section there, bit for bit.
    pub(super) fn station_section(
        &self,
        plan: &SectionPlan,
        station: &owd_core::volumes::sections::StationAt,
        depth: f64,
        terrain: TerrainSampler,
    ) -> Result<owd_core::volumes::sections::CrossSection, DesignError> {
        let bedded;
        let terrain: TerrainSampler = match &plan.bed {
            Some((bed, level)) => {
                bedded = super::bed::with_bed(bed, level, terrain);
                &bedded
            }
            None => terrain,
        };
        owd_core::volumes::sections::section_at(
            station,
            terrain,
            &plan.river,
            depth,
            &plan.model,
            plan.typed.wet_slope,
            plan.typed.maxw_m,
            plan.typed.dx_m,
            None,
            None,
        )
        .map_err(DesignError::Input)
    }

    /// The stations of one option with no terrain read (MAP-5): where each stands, the bottom
    /// half-width and the water level there, as [`Session::option_sections`] gives them to the walk
    /// (the same station frame, the same half-width rule, the same level reader), and the design
    /// depth. For drawing the channel when the volumes stage holds no stations.
    pub(super) fn option_frame(
        &self,
        plan: &SectionPlan,
        o: &Value,
    ) -> Result<(Vec<owd_core::volumes::sections::StationAt>, f64), DesignError> {
        let line: Vec<Pt> = all_positions(&o["line"]);
        let (chain, xy, th) =
            owd_core::volumes::sections::station_frame(&line, plan.typed.station_m)
                .map_err(DesignError::Input)?;
        let mut settings = plan.typed.clone();
        settings.half_width_m = plan.fixed_hb;
        let hb = super::volumes_stage::half_widths(&xy, &plan.shape, &settings)
            .map_err(super::rule_sets::stage_error)?;
        let water_in = self.water_input_on(
            crate::types::Polyline(line.clone()),
            crate::types::Polyline(plan.centre.clone()),
        );
        let stage_level;
        let port_level;
        let wl: &dyn Fn(f64) -> f64 = match self.water_sampling {
            super::water::WaterSampling::Projection => {
                stage_level = super::water_stage::level_reader(&water_in)
                    .map_err(|e| DesignError::Input(e.message))?;
                &stage_level
            }
            super::water::WaterSampling::DesignLineChainage => {
                port_level = self.water_level_along(&plan.profile, &line, &plan.centre);
                &port_level
            }
        };
        let stations = (0..xy.len())
            .map(|i| owd_core::volumes::sections::StationAt {
                chainage_m: chain[i],
                xy: xy[i],
                heading_rad: th[i],
                hb: hb[i],
                wl: wl(chain[i]),
            })
            .collect();
        Ok((stations, plan.shape.design_depth_m))
    }

    /// The slope of the cut below water and the reach and spacing of a section's samples, as the
    /// plan's settings give them.
    pub(super) fn plan_cut_settings(plan: &SectionPlan) -> (f64, f64, f64) {
        (plan.typed.wet_slope, plan.typed.maxw_m, plan.typed.dx_m)
    }

    /// The bed model's bed at a point of the project CRS, as [`Session::plan_terrain`] lays it on
    /// the terrain: `None` outside the water. `None` when the document has no bed model.
    #[allow(clippy::type_complexity)]
    pub(super) fn plan_bed_at(plan: &SectionPlan) -> Option<Box<dyn Fn(Pt) -> Option<f64>>> {
        let (bed, level) = plan.bed.clone()?;
        Some(Box::new(move |p: Pt| {
            let wl = level.level_at(crate::types::Chainage(bed.chainage_of(p)));
            bed.bed_elevation(p, wl)
        }))
    }

    /// `terrain` as the walk reads it: with the bed model laid inside the water, when the document
    /// has one.
    pub(super) fn plan_terrain<'a>(
        plan: &'a SectionPlan,
        terrain: TerrainSampler<'a>,
    ) -> Box<dyn Fn(&[f64], &[f64]) -> Vec<f64> + 'a> {
        match &plan.bed {
            Some((bed, level)) => Box::new(super::bed::with_bed(bed, level, terrain)),
            None => Box::new(move |xs, ys| terrain(xs, ys)),
        }
    }

    /// [`Session::option_sections`] with the cut-rate multiplier at points (ticket 13), for the
    /// volumes stage's own walk.
    pub(super) fn option_sections_weighted(
        &self,
        plan: &SectionPlan,
        o: &Value,
        terrain: TerrainSampler,
        weight: Option<TerrainSampler>,
    ) -> Result<owd_core::volumes::sections::SectionVolumes, DesignError> {
        let id = o["id"].as_str().unwrap_or("?");
        let line: Vec<Pt> = all_positions(&o["line"]);
        // Where the document has a bed model, it is the bed inside the water; the DEM is the rest.
        let bedded;
        let terrain: TerrainSampler = match &plan.bed {
            Some((bed, level)) => {
                bedded = super::bed::with_bed(bed, level, terrain);
                &bedded
            }
            None => terrain,
        };
        // The level at each station: through the water stage's own reader (one path, spec 0017
        // ticket 8). Only the port tests' hidden sampling by design-line chainage reads otherwise.
        let water_in = self.water_input_on(
            crate::types::Polyline(line.clone()),
            crate::types::Polyline(plan.centre.clone()),
        );
        let stage_level;
        let port_level;
        let wl: &dyn Fn(f64) -> f64 = match self.water_sampling {
            super::water::WaterSampling::Projection => {
                stage_level = super::water_stage::level_reader(&water_in)
                    .map_err(|e| DesignError::Input(e.message))?;
                &stage_level
            }
            super::water::WaterSampling::DesignLineChainage => {
                port_level = self.water_level_along(&plan.profile, &line, &plan.centre);
                &port_level
            }
        };
        let mut settings = plan.typed.clone();
        settings.cut_model = plan.model.clone();
        settings.half_width_m = plan.fixed_hb;
        super::volumes_stage::walk(
            &line,
            &plan.shape,
            &plan.river,
            wl,
            &settings,
            terrain,
            weight,
        )
        .map_err(|e| {
            if e.code == "VOLUMES_WALK" {
                DesignError::Input(format!("option `{id}`: {}", e.message))
            } else {
                super::rule_sets::stage_error(e)
            }
        })
    }
}
