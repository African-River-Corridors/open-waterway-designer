//! The plan cut of each design line option (ticket C2b): the fairway at the water surface less
//! the river polygon, from `owd_core::plan_cut`. It needs the river polygon, a fresh sizing and a
//! design line; no terrain and no water level.
//!
//! It is not a fifth stage of the chain. Like the largest-vessel sweep (`stages.envelope`), it is
//! an optional member of `stages` with its own status, and the run loop never runs it.

use super::crs::{Crs, Utm};
use super::geojson::all_positions;
use super::hash::hash_value;
use super::screening::SCREENING_NOTICE;
use super::session::{DesignError, ENGINE_VERSION, Session, Stage, StageStatus};
use owd_core::cut_model::{WIDEN_RMAX, widened_hb_from_shape};
use owd_core::geom::{Polygon, Pt};
use owd_core::plan_cut::{Bank, Fairway, PLAN_ONLY_LABEL, plan_cut};
use owd_core::volumes::sections::{DEFAULT_STATION_M, DEFAULT_WET_SLOPE, HalfWidth};
use serde_json::{Map, Value, json};

/// The default smallest piece, in square metres: a smaller piece is not a cut.
const DEFAULT_MIN_PIECE_M2: f64 = 50.0;

/// The settings of the plan cut.
struct Settings {
    min_piece_m2: f64,
    station_m: f64,
    wet_slope: f64,
}

impl Settings {
    fn to_value(&self) -> Value {
        json!({
            "min_piece_m2": self.min_piece_m2,
            "station_m": self.station_m,
            "wet_slope": self.wet_slope,
        })
    }

    /// Lay `source` (an object, or null for none) over these. A key that is not a setting, or a
    /// value that breaks its rule, is refused by name.
    fn apply(mut self, source: &Value) -> Result<Settings, DesignError> {
        let Some(o) = source.as_object() else {
            return Ok(self);
        };
        for (k, v) in o {
            let x = v.as_f64();
            match k.as_str() {
                "min_piece_m2" => {
                    self.min_piece_m2 = x.filter(|x| x.is_finite() && *x >= 0.0).ok_or_else(|| {
                        DesignError::Input(
                            "the setting `min_piece_m2` must be a number of square metres, 0 or more"
                                .into(),
                        )
                    })?
                }
                "station_m" => {
                    self.station_m = x.filter(|x| x.is_finite() && *x > 0.0).ok_or_else(|| {
                        DesignError::Input(
                            "the setting `station_m` must be a number of metres above 0".into(),
                        )
                    })?
                }
                "wet_slope" => {
                    self.wet_slope = x.filter(|x| x.is_finite() && *x > 0.0).ok_or_else(|| {
                        DesignError::Input("the setting `wet_slope` must be a number above 0".into())
                    })?
                }
                other => {
                    return Err(DesignError::Input(format!(
                        "the plan cut has no setting `{other}`: its settings are min_piece_m2, \
                         station_m and wet_slope"
                    )));
                }
            }
        }
        Ok(self)
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            min_piece_m2: DEFAULT_MIN_PIECE_M2,
            station_m: DEFAULT_STATION_M,
            wet_slope: DEFAULT_WET_SLOPE,
        }
    }
}

/// A polygon as GeoJSON rings: the exterior, then each hole.
fn rings(p: &Polygon) -> Vec<Vec<Pt>> {
    std::iter::once(p.exterior.clone())
        .chain(p.holes.iter().cloned())
        .collect()
}

impl Session {
    /// What the plan cut depends on: the river, its banks and centreline, the CRS, the sizing, the
    /// id and line of every option, and the settings.
    fn plan_cut_dependency_hash(&self) -> String {
        let inp = &self.doc["inputs"];
        let options: Vec<Value> = self
            .doc
            .get("options")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|o| json!({"id": o["id"], "line": o["line"]}))
            .collect();
        hash_value(&json!({
            "river_polygon": inp["river_polygon"],
            "centreline": inp["centreline"],
            "left_bank": inp["left_bank"],
            "right_bank": inp["right_bank"],
            "crs": self.doc["project"]["crs"],
            "sizing": self.stage_hash(Stage::Sizing),
            "options": options,
            "settings": self.doc["stages"]["plan_cut"]["settings"],
        }))
    }

    /// Where the plan cut stands: not run, fresh, or stale.
    pub fn plan_cut_status(&self) -> StageStatus {
        let p = &self.doc["stages"]["plan_cut"];
        if p["results"].is_null() {
            return StageStatus::NotRun;
        }
        if p["depends_on"].as_str() == Some(self.plan_cut_dependency_hash().as_str()) {
            StageStatus::Fresh
        } else {
            StageStatus::Stale
        }
    }

    /// The plan cut the study holds, if it has run.
    pub fn plan_cut_result(&self) -> Option<&Value> {
        let r = &self.doc["stages"]["plan_cut"]["results"];
        (!r.is_null()).then_some(r)
    }

    /// Run the plan cut for every design line option and keep it. Settings: the defaults, then the
    /// document's, then `overrides`. A refused run stores nothing.
    pub fn run_plan_cut(&mut self, overrides: &Value) -> Result<(), DesignError> {
        let settings = Settings::default()
            .apply(&self.doc["stages"]["plan_cut"]["settings"])?
            .apply(overrides)?;
        let river = self.river_polygons().ok_or_else(|| {
            DesignError::Input(
                "the plan cut needs the river polygon: import it, or both banks, first \
                 (`owd import --polygon FILE`)"
                    .into(),
            )
        })?;
        match self.stage_status(Stage::Sizing) {
            StageStatus::Fresh => {}
            StageStatus::NotRun => {
                return Err(DesignError::Input(
                    "the plan cut needs the sizing: choose a vessel class and size the channel \
                     first (`owd size`)"
                        .into(),
                ));
            }
            StageStatus::Stale => {
                return Err(DesignError::Input(
                    "the sizing is stale: run `owd size` (or `owd run`) before the plan cut".into(),
                ));
            }
        }
        for (stage, command) in [(Stage::Alignment, "align"), (Stage::Optimise, "optimise")] {
            if self.stage_status(stage) == StageStatus::Stale {
                return Err(DesignError::Input(format!(
                    "the {} stage is stale: run `owd {command}` (or `owd run`) before the plan cut",
                    stage.name()
                )));
            }
        }
        let options: Vec<Value> = self
            .doc
            .get("options")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if options.is_empty() {
            return Err(DesignError::Input(
                "there are no design line options: run the alignment stage first".into(),
            ));
        }
        let shape = self.sized_shape()?;
        let centreline = self.river_centreline()?;
        let widen = |xy: &[Pt]| widened_hb_from_shape(xy, &shape, WIDEN_RMAX);
        let mut rows = Vec::new();
        for o in &options {
            let line = all_positions(&o["line"]);
            let fairway = Fairway {
                bottom_half_width: HalfWidth::FromStations(&widen),
                wet_slope: settings.wet_slope,
                depth_m: shape.design_depth_m,
                station_m: settings.station_m,
            };
            let cut = plan_cut(&line, &fairway, &river, &centreline, settings.min_piece_m2)
                .map_err(|e| {
                    DesignError::Input(format!(
                        "the plan cut of `{}`: {e}",
                        o["id"].as_str().unwrap_or("?")
                    ))
                })?;
            let pieces: Vec<Value> = cut
                .pieces
                .iter()
                .map(|p| json!(rings(&p.polygon)))
                .collect();
            let mut row = json!({
                "id": o["id"],
                "name": o["name"],
                "label": PLAN_ONLY_LABEL,
                "area_m2": cut.area_m2(),
                "by_bank": {"left_m2": cut.left_m2(), "right_m2": cut.right_m2()},
                "pieces": cut.pieces.len(),
                "largest_piece_m2": cut.largest_piece_m2(),
                "dropped": {"pieces": cut.dropped_pieces, "area_m2": cut.dropped_m2},
                "cut": {"type": "MultiPolygon", "coordinates": pieces},
                "piece_banks": cut.pieces.iter().map(|p| match p.bank {
                    Bank::Left => "left",
                    Bank::Right => "right",
                }).collect::<Vec<_>>(),
                "piece_areas_m2": cut.pieces.iter().map(|p| p.area_m2).collect::<Vec<_>>(),
            });
            // As a volumes row: an option with no feasible line keeps its numbers, flagged.
            if super::export::no_feasible_line(o) {
                row["no_feasible_line"] = json!(true);
            }
            rows.push(row);
        }
        if !self.doc["stages"].is_object() {
            self.doc["stages"] = json!({});
        }
        self.doc["stages"]["plan_cut"] = json!({
            "settings": settings.to_value(),
            "results": {
                "label": PLAN_ONLY_LABEL,
                "notice": SCREENING_NOTICE,
                "options": rows,
            },
            "depends_on": null,
            "engine_version": ENGINE_VERSION,
        });
        let h = self.plan_cut_dependency_hash();
        self.doc["stages"]["plan_cut"]["depends_on"] = json!(h);
        Ok(())
    }

    /// The cut pieces as a GeoJSON FeatureCollection, one `Polygon` per piece: in the project CRS,
    /// or with `wgs84` in longitude and latitude. Refused before the plan cut has run.
    pub fn export_plan_cut_geojson(&self, wgs84: bool) -> Result<String, DesignError> {
        let result = self.plan_cut_result().ok_or_else(|| {
            DesignError::Input("there is no plan cut to export: run `owd plan-cut` first".into())
        })?;
        let epsg = self.doc["project"]["crs"]["epsg"]
            .as_u64()
            .map(|c| c as u32);
        let utm: Option<Utm> = epsg
            .and_then(|c| Crs::from_epsg(c).ok())
            .and_then(|c| match c {
                Crs::Utm(u) => Some(u),
                Crs::Wgs84 => None,
            });
        let point = |p: &Value| -> Result<Value, DesignError> {
            let (x, y) = (
                p[0].as_f64().unwrap_or(f64::NAN),
                p[1].as_f64().unwrap_or(f64::NAN),
            );
            if wgs84 {
                let u = utm.ok_or_else(|| {
                    DesignError::Input("the channel design has no project CRS".into())
                })?;
                let [lon, lat] = u.inverse(x, y).map_err(DesignError::Input)?;
                Ok(json!([lon, lat]))
            } else {
                Ok(json!([x, y]))
            }
        };
        let mut features = Vec::new();
        for o in result["options"].as_array().into_iter().flatten() {
            let polys = o["cut"]["coordinates"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            for (i, poly) in polys.iter().enumerate() {
                let mut coords = Vec::new();
                for ring in poly.as_array().into_iter().flatten() {
                    let pts = ring
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(&point)
                        .collect::<Result<Vec<_>, _>>()?;
                    coords.push(Value::Array(pts));
                }
                features.push(json!({
                    "type": "Feature",
                    "geometry": {"type": "Polygon", "coordinates": coords},
                    "properties": {
                        "option": o["id"],
                        "bank": o["piece_banks"][i],
                        "area_m2": o["piece_areas_m2"][i],
                        "label": PLAN_ONLY_LABEL,
                    },
                }));
            }
        }
        let mut fc = Map::new();
        fc.insert("type".into(), json!("FeatureCollection"));
        fc.insert("label".into(), json!(PLAN_ONLY_LABEL));
        fc.insert("notice".into(), json!(SCREENING_NOTICE));
        fc.insert("title".into(), self.doc["project"]["title"].clone());
        fc.insert("features".into(), Value::Array(features));
        if !wgs84 {
            fc.insert(
                "crs".into(),
                json!({"type": "name", "properties": {"name": format!("urn:ogc:def:crs:EPSG::{}", epsg.unwrap_or(0))}}),
            );
        }
        let mut t =
            serde_json::to_string_pretty(&Value::Object(fc)).expect("a JSON value serialises");
        t.push('\n');
        Ok(t)
    }
}
