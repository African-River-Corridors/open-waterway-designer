//! The design session in the browser (spec 0011).
//!
//! A thin forwarder: each method calls `owd_shell::state::AppState` and returns the same JSON the
//! desktop app's commands return. It holds no calculation, validation or default logic. Errors are
//! thrown to JS as plain strings. A stage runs to its end inside one call: the page runs this in a
//! Web Worker, and Cancel ends the Worker (wasm cannot unwind out of a stage).

use owd_format::design::date_from_unix;
use owd_shell::input;
use owd_shell::state::AppState;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

pub mod terrain;

type Js<T> = Result<T, JsValue>;

fn err(message: String) -> JsValue {
    JsValue::from_str(&message)
}

fn ok<T>(result: Result<T, String>) -> Js<T> {
    result.map_err(err)
}

fn today() -> String {
    date_from_unix((js_sys::Date::now() / 1000.0) as u64)
}

fn new_design() -> AppState {
    AppState::new_design("Untitled channel design", &today())
}

/// The network for the reach steps (spec 0012): a JS function `(url, start, length)` that returns
/// the bytes as a `Uint8Array`, or throws. `start` is -1 for the whole file. The page's Worker gives
/// one that makes a synchronous request, which a Worker may do.
struct JsFetch(js_sys::Function);

impl JsFetch {
    fn call(&self, url: &str, start: f64, len: f64) -> Result<Vec<u8>, owd_reach::FetchError> {
        self.0
            .call3(
                &JsValue::NULL,
                &JsValue::from_str(url),
                &JsValue::from_f64(start),
                &JsValue::from_f64(len),
            )
            .map(|v| js_sys::Uint8Array::new(&v).to_vec())
            .map_err(|e| {
                // A thrown Error gives its message, without the stack; anything else as it is.
                let reason = e
                    .as_string()
                    .or_else(|| e.dyn_ref::<js_sys::Error>().map(|err| err.message().into()))
                    .unwrap_or_else(|| format!("{e:?}"));
                fetch_error(url, &reason)
            })
    }
}

/// The fetch error for `url` from the reason the page's fetch threw (`HTTP <status> from <url>`,
/// perhaps with a stack after it). Only a status of 404 on the first line is a missing file; any
/// other answer, such as a busy server's 502, is a failure, so a reach step tries the next server.
/// Searching the whole text for "404" (before #499) also matched a stack's line and column numbers.
pub fn fetch_error(url: &str, reason: &str) -> owd_reach::FetchError {
    let first = reason.lines().next().unwrap_or("");
    let words: Vec<&str> = first.split_whitespace().collect();
    let not_found = words.windows(2).any(|w| w[0] == "HTTP" && w[1] == "404");
    if not_found {
        owd_reach::FetchError::NotFound(url.to_string())
    } else {
        owd_reach::FetchError::Failed {
            url: url.to_string(),
            reason: reason.to_string(),
        }
    }
}

impl owd_reach::Fetch for JsFetch {
    fn get(&self, url: &str) -> Result<Vec<u8>, owd_reach::FetchError> {
        self.call(url, -1.0, -1.0)
    }

    fn get_range(&self, url: &str, start: u64, len: u64) -> Result<Vec<u8>, owd_reach::FetchError> {
        self.call(url, start as f64, len as f64)
    }
}

/// Progress for the page (#320): the Worker sets `self.owdReachProgress(done, total, what)`, which
/// posts it to the page. Cancel on the web ends the Worker, so nothing here stops the work.
fn report_to_page(s: &owd_reach::Step) {
    let hook = js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("owdReachProgress"));
    if let Ok(f) = hook.and_then(|h| h.dyn_into::<js_sys::Function>()) {
        let _ = f.call3(
            &JsValue::NULL,
            &JsValue::from_f64(s.done as f64),
            &JsValue::from_f64(s.total as f64),
            &JsValue::from_str(&s.line()),
        );
    }
}

fn never() -> bool {
    false
}

fn endpoints(json: &str) -> Result<owd_shell::reach::ReachEndpoints, String> {
    if json.trim().is_empty() {
        return Ok(Default::default());
    }
    let v: Value = serde_json::from_str(json).map_err(|e| format!("not endpoints: {e}"))?;
    Ok(owd_shell::reach::ReachEndpoints::from_json(&v))
}

fn point(v: &Value) -> Result<[f64; 2], String> {
    match (v[0].as_f64(), v[1].as_f64()) {
        (Some(a), Some(b)) => Ok([a, b]),
        _ => Err(format!("`{v}` is not [lon, lat]")),
    }
}

/// The engine version, as written in a `.owd` file.
#[wasm_bindgen]
pub fn engine_version() -> String {
    owd_format::design::session::ENGINE_VERSION.to_string()
}

/// One open channel design, as in the desktop app.
#[wasm_bindgen]
pub struct Session {
    state: AppState,
    /// The terrain tiles of the DEM, for the map's 3D view: made on demand, never stored.
    terrain: terrain::TerrainTiles,
    /// The channel of the design line as the map draws it (MAP-5), as JSON, for the design named
    /// by the terrain's cut key: made when the design changes, never stored.
    channel: String,
}

impl Session {
    /// Edit, then return the snapshot JSON.
    fn edit(&mut self, f: impl FnOnce(&mut AppState) -> Result<(), String>) -> Js<String> {
        ok(f(&mut self.state))?;
        Ok(self.snapshot())
    }

    /// The stations of an option, read now or taken from the cache, with the section profile of
    /// the station nearest each of `profiles_at`.
    fn walked(&mut self, option: &str, profiles_at: &[f64]) -> Js<owd_shell::state::Walked> {
        let walked = ok(self
            .state
            .begin_walk(option)
            .with_profiles_at(profiles_at)
            .run())?;
        self.state.keep_walk(&walked);
        Ok(walked)
    }
}

impl Session {
    /// Make the channel's geometry and its cut surface again when the design has changed since
    /// they were made (MAP-5). The geometry is `{key, geometry, note, legend}`: `geometry` is null with
    /// `note` saying why when the channel cannot be drawn yet.
    fn sync_channel(&mut self) {
        let key = self.state.channel_key();
        if !self.channel.is_empty() && self.terrain.cut_key() == key {
            return;
        }
        let (geometry, note, cut) = match self.state.channel() {
            Ok((g, cut)) => (json!(g), serde_json::Value::Null, cut),
            Err(why) => (serde_json::Value::Null, json!(why), None),
        };
        self.channel = json!({
            "key": key,
            "geometry": geometry,
            "note": note,
            "legend": terrain::TerrainTiles::cut_legend(),
        })
        .to_string();
        // The maker of the DEM in memory first, so the cut is laid on it.
        let _ = self.terrain.info(self.state.dem());
        self.terrain.set_cut(key, cut);
    }
}

#[wasm_bindgen]
impl Session {
    /// A new, empty channel design.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Session {
        Session {
            state: new_design(),
            terrain: terrain::TerrainTiles::default(),
            channel: String::new(),
        }
    }

    pub fn snapshot(&self) -> String {
        self.state.snapshot().to_string()
    }

    /// The Rule Sets (built in, then each loaded from a file), set inputs and vessel classes.
    pub fn sizing_choices(&self) -> String {
        self.state.sizing_choices().to_string()
    }

    /// Load a Rule Set file's text for the life of this session (the app's "Load Rule Set file",
    /// ticket #499). Returns `id@version`; a refused file changes nothing.
    pub fn load_rule_set_text(&mut self, text: &str) -> Js<String> {
        ok(self.state.load_rule_set_text(text))
    }

    pub fn new_design(&mut self) -> String {
        self.state = new_design().with_machine_settings_of(&self.state);
        self.snapshot()
    }

    /// Open `.owd` text. `dem` is the DEM file the document names, when the page has it.
    pub fn open_text(&mut self, name: &str, text: &str, dem: Option<Vec<u8>>) -> Js<String> {
        let opened = ok(AppState::open(Path::new(name), text, dem.as_deref()))?;
        self.state = opened.with_machine_settings_of(&self.state);
        Ok(self.snapshot())
    }

    /// Open a design of a project folder (PROJ-1) with the DEM the project names, in one step.
    /// `design` and `dem_path` are paths from the project folder. The snapshot has `project_note`:
    /// what opening changed in the design (its DEM named, or pointed at the project's), or null.
    pub fn open_project(
        &mut self,
        design: &str,
        text: &str,
        dem_path: Option<String>,
        dem: Option<Vec<u8>>,
    ) -> Js<String> {
        let dem = match (&dem_path, dem) {
            (Some(p), Some(bytes)) => Some((Path::new(p.as_str()), bytes)),
            _ => None,
        };
        let (opened, note) = ok(AppState::open_in_project(
            Path::new(design),
            text,
            dem,
            &today(),
        ))?;
        self.state = opened.with_machine_settings_of(&self.state);
        let mut snap: Value = self.state.snapshot();
        snap["project_note"] = json!(note);
        Ok(snap.to_string())
    }

    /// Read and check the text of a project folder's `owd-project.json` (PROJ-1), as JSON.
    pub fn project_read(&self, text: &str) -> Js<String> {
        ok(owd_shell::project::read(text)).map(|v| v.to_string())
    }

    /// What a project folder with no text file holds in the standard places. `listing` is JSON,
    /// see `owd_shell::project::find`.
    pub fn project_find(&self, listing: &str) -> Js<String> {
        let listing: Value =
            ok(serde_json::from_str(listing).map_err(|e| format!("not a folder listing: {e}")))?;
        Ok(owd_shell::project::find(&listing).to_string())
    }

    /// The text of a new `owd-project.json` for a folder that has none.
    pub fn project_adopt(
        &self,
        listing: &str,
        title: &str,
        design: Option<String>,
        dem: Option<String>,
    ) -> Js<String> {
        let listing: Value =
            ok(serde_json::from_str(listing).map_err(|e| format!("not a folder listing: {e}")))?;
        ok(owd_shell::project::adopt(
            &listing,
            title,
            design.as_deref(),
            dem.as_deref(),
        ))
    }

    /// Where a file OWD writes out goes in a project folder: `exports/<name>`.
    pub fn project_export_path(&self, name: &str) -> Js<String> {
        ok(owd_shell::project::export_path(name))
    }

    /// Set the water level from a pool table (`chainage_m,level_m` a line), as `owd import
    /// --water-pools FILE` does, through the same session call.
    pub fn set_water_pools(&mut self, table: &str) -> Js<String> {
        self.edit(|s| s.set_water_pools(table))
    }

    /// The `.owd` text to save as `name` (the DEM path is said from there).
    pub fn save_text(&mut self, name: &str) -> String {
        self.state.save_text_for(Path::new(name))
    }

    /// Record that the design was saved as `name`.
    pub fn saved(&mut self, name: &str) -> String {
        self.state.saved(Path::new(name));
        self.snapshot()
    }

    pub fn is_dirty(&self) -> bool {
        self.state.is_dirty()
    }

    /// Choose the Rule Sets and set inputs. `ids` is a JSON list of set ids, `inputs` a JSON object
    /// of input name to value.
    pub fn set_rule_sets(&mut self, ids: &str, inputs: &str) -> Js<String> {
        let ids: Vec<String> =
            ok(serde_json::from_str(ids).map_err(|e| format!("not a list of Rule Sets: {e}")))?;
        let inputs: std::collections::BTreeMap<String, String> =
            ok(serde_json::from_str(inputs).map_err(|e| format!("not set inputs: {e}")))?;
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        let inputs: Vec<(&str, &str)> = inputs
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        self.edit(|s| s.set_rule_sets(&ids, &inputs))
    }

    pub fn set_vessel(&mut self, vessel_standard_id: &str, draught: &str) -> Js<String> {
        let draught = ok(AppState::parse_draught(draught))?;
        self.edit(|s| s.set_vessel(vessel_standard_id, draught))
    }

    /// Choose a custom vessel from the form's fields, as JSON text (spec 0015 T2).
    pub fn set_custom_vessel(&mut self, fields: &str) -> Js<String> {
        let fields: serde_json::Value =
            ok(serde_json::from_str(fields).map_err(|e| e.to_string()))?;
        self.edit(|s| s.set_custom_vessel(&fields))
    }

    /// Check a custom vessel's fields, as JSON text: the refusal and its field, or the warnings.
    pub fn check_custom_vessel(&self, fields: &str) -> Js<String> {
        let fields: serde_json::Value =
            ok(serde_json::from_str(fields).map_err(|e| e.to_string()))?;
        Ok(self.state.check_custom_vessel(&fields).to_string())
    }

    /// The constraint stretches as GeoJSON lines in WGS84, for the map.
    pub fn stretches_geojson(&self) -> Js<String> {
        Ok(ok(self.state.stretches_geojson())?.to_string())
    }

    pub fn run_stale(&mut self) -> Js<String> {
        self.edit(|s| s.run_stale().map(|_| ()))
    }

    /// Load one vector input from the bytes of a GeoJSON or GeoPackage file.
    pub fn load_input(
        &mut self,
        role: &str,
        name: &str,
        bytes: Vec<u8>,
        layer: Option<String>,
    ) -> Js<String> {
        let role = ok(input::role_of(role))?;
        let text = ok(input::vector_text(name, bytes, layer.as_deref()))?;
        let date = today();
        self.edit(|s| s.load_inputs(&[(role, text)], &date))
    }

    /// Edit a bank (spec 0012). `edit` is JSON, see `owd_shell::input::bank_edit_of`.
    pub fn edit_bank(&mut self, side: &str, edit: &str) -> Js<String> {
        let side = ok(input::role_of(side))?;
        let edit: Value =
            ok(serde_json::from_str(edit).map_err(|e| format!("not a bank edit: {e}")))?;
        let edit = ok(input::bank_edit_of(&edit))?;
        let date = today();
        self.edit(|s| s.edit_bank(side, edit, &date))
    }

    /// Plan a reach and start a new study from it (spec 0012). `points` is JSON
    /// `{"start": [lon, lat], "end": [lon, lat], "via": [[lon, lat], ...]}`.
    pub fn reach_route(
        &mut self,
        river: &str,
        points: &str,
        at: &str,
        fetch: js_sys::Function,
    ) -> Js<String> {
        let v: Value =
            ok(serde_json::from_str(points).map_err(|e| format!("not reach points: {e}")))?;
        let start = ok(point(&v["start"]))?;
        let end = ok(point(&v["end"]))?;
        let via = ok(v["via"]
            .as_array()
            .map(|a| a.iter().map(point).collect::<Result<Vec<_>, _>>())
            .unwrap_or(Ok(Vec::new())))?;
        let at = ok(endpoints(at))?;
        let date = today();
        let river = Some(river).filter(|r| !r.trim().is_empty());
        let js = JsFetch(fetch);
        let f = owd_reach::Reporting::new(&js, &report_to_page, &never);
        self.edit(|s| s.reach_route(&f, &at, river, start, end, &via, &date))
    }

    /// The rivers and canals whose name starts with or matches a text, as a JSON list (spec 0012,
    /// T13). Changes nothing.
    pub fn reach_search(
        &self,
        text: &str,
        country: &str,
        once: bool,
        at: &str,
        fetch: js_sys::Function,
    ) -> Js<String> {
        let at = ok(endpoints(at))?;
        let hits = ok(owd_shell::reach::search_rivers(
            &JsFetch(fetch),
            &at,
            text,
            Some(country),
            once,
        ))?;
        Ok(hits.to_string())
    }

    /// Start a new study from a whole river chosen from the search. `also` is a JSON list of more
    /// OSM ways of the same river, as the search gives them; `search` the source it was found with.
    pub fn reach_river(
        &mut self,
        osm_id: &str,
        also: &str,
        search: &str,
        at: &str,
        fetch: js_sys::Function,
    ) -> Js<String> {
        let also: Vec<String> =
            ok(serde_json::from_str(also).map_err(|e| format!("not a list of OSM ids: {e}")))?;
        let at = ok(endpoints(at))?;
        let js = JsFetch(fetch);
        let river = ok(owd_shell::reach::read_river(
            &owd_reach::Reporting::new(&js, &report_to_page, &never),
            &at,
            osm_id,
            &also,
        ))?;
        let date = today();
        let search = Some(search).filter(|s| !s.is_empty());
        self.edit(|s| s.start_river(river, search, &date))
    }

    /// Cut a reach out of the chosen river. `points` as for `reach_route`.
    pub fn reach_cut(&mut self, points: &str) -> Js<String> {
        let v: Value =
            ok(serde_json::from_str(points).map_err(|e| format!("not reach points: {e}")))?;
        let start = ok(point(&v["start"]))?;
        let end = ok(point(&v["end"]))?;
        let via = ok(v["via"]
            .as_array()
            .map(|a| a.iter().map(point).collect::<Result<Vec<_>, _>>())
            .unwrap_or(Ok(Vec::new())))?;
        let date = today();
        self.edit(|s| s.reach_cut(start, end, &via, &date))
    }

    /// Draw the reach of an opened study again (#318). The document is not changed.
    pub fn reach_restore(&mut self, at: &str, fetch: js_sys::Function) -> Js<String> {
        if !self.state.reach_needs_restore() {
            return Ok(self.snapshot());
        }
        let at = ok(endpoints(at))?;
        let doc = self.state.snapshot()["document"].clone();
        let js = JsFetch(fetch);
        let restored = ok(owd_shell::reach::plan_restore(
            &owd_reach::Reporting::new(&js, &report_to_page, &never),
            &at,
            &doc,
        ))?;
        self.edit(|s| s.restore_reach(restored))
    }

    /// True while the study holds nothing but its reach.
    pub fn reach_route_only(&self) -> bool {
        self.state.reach_route_only()
    }

    /// Give a JRC occurrence tile the user downloaded and dropped in.
    pub fn reach_add_tile(&mut self, name: &str, bytes: Vec<u8>) -> Js<String> {
        self.edit(|s| s.reach_add_tile(name, bytes))
    }

    /// Run one reach step: `banks`, `edge_fit` or `level` (with an optional percentile).
    pub fn reach_step(
        &mut self,
        step: &str,
        percentile: Option<f64>,
        at: &str,
        fetch: js_sys::Function,
    ) -> Js<String> {
        let step = ok(owd_shell::reach::ReachStep::of(step, percentile))?;
        let at = ok(endpoints(at))?;
        let date = today();
        let job = ok(self.state.begin_reach(step))?;
        let js = JsFetch(fetch);
        let job = ok(job.run(
            &owd_reach::Reporting::new(&js, &report_to_page, &never),
            &at,
            &date,
        ))?;
        self.edit(|s| s.keep_reach(job))
    }

    /// Give the depth of a river with no survey: the depth and the depth table as typed. The shell
    /// reads them.
    pub fn set_bed(&mut self, depth: &str, table: &str, season: &str) -> Js<String> {
        let date = today();
        self.edit(|s| s.set_bed_text(depth, table, Some(season), &date))
    }

    /// Answer "what fits" over every vessel class.
    pub fn run_fits(&mut self) -> Js<String> {
        self.edit(|s| s.run_fits())
    }

    /// The largest vessel the river allows: the size sweep of spec 0016, one size after another.
    pub fn run_envelope(&mut self) -> Js<String> {
        self.edit(|s| s.run_envelope())
    }

    /// Undo the last bank edit.
    pub fn undo_bank_edit(&mut self) -> Js<String> {
        self.edit(|s| s.undo_bank_edit())
    }

    /// Load the DEM from the bytes of a GeoTIFF.
    pub fn load_dem(&mut self, name: &str, bytes: Vec<u8>) -> Js<String> {
        let date = today();
        self.edit(|s| s.load_dem(Path::new(name), bytes, &date))
    }

    /// Set the water level from the level and the fall per metre as typed; the shell reads them.
    /// Set the memory a terrain grid may take in this browser, in GiB (empty: the default).
    pub fn set_terrain_limit(&mut self, gib: &str) -> Js<String> {
        self.edit(|s| s.set_terrain_limit_gib_text(gib))
    }

    /// Set the point the river flows towards for the bank imports that follow, as `LON,LAT`
    /// (empty: none).
    pub fn set_downstream_point(&mut self, point: &str) -> Js<String> {
        self.edit(|s| s.set_downstream_point_text(point))
    }

    /// State the river mouth (spec 0017): `downstream` or `LON,LAT`, as `owd import --mouth` reads
    /// it, through the same shell call.
    pub fn set_mouth(&mut self, mouth: &str) -> Js<String> {
        self.edit(|s| s.set_mouth(mouth))
    }

    /// Set the run inputs (spec 0017, ticket 11): the objective (`SENSE:MEASURE`), the limits (one
    /// `MEASURE<=VALUE` a line) and the target level, each empty for none, as `owd run-inputs`
    /// sets them, through the same session call.
    pub fn set_run_inputs(
        &mut self,
        objective: &str,
        limits: &str,
        target_level: &str,
    ) -> Js<String> {
        self.edit(|s| s.set_run_inputs(objective, limits, target_level))
    }

    /// Set the no-go areas (spec 0017, ticket 13) from GeoJSON text (a FeatureCollection of polygons
    /// in the project CRS, each with `name` and `kind`), as `owd constraints --no-go` reads it,
    /// through the same session call. Empty text removes them.
    pub fn set_no_go(&mut self, geojson: &str) -> Js<String> {
        self.edit(|s| s.set_no_go(geojson))
    }

    /// Set the cut-rate multiplier raster (spec 0017, ticket 13) from the bytes of a GeoTIFF, as
    /// `owd constraints --multiplier` does, through the same session call.
    pub fn set_cut_rate_multiplier(&mut self, name: &str, bytes: Vec<u8>) -> Js<String> {
        self.edit(|s| s.set_cut_rate_multiplier(Path::new(name), bytes))
    }

    /// Remove the cut-rate multiplier raster (spec 0017, ticket 13).
    pub fn clear_cut_rate_multiplier(&mut self) -> Js<String> {
        self.edit(|s| s.clear_cut_rate_multiplier())
    }

    /// Set settings of one alignment method (`rolling_circle`, `reroute`, `centreline`), as JSON
    /// text, as `owd align --use-water` does (ticket #494). Nothing runs.
    pub fn set_alignment_settings(&mut self, method: &str, settings: &str) -> Js<String> {
        let settings: Value =
            ok(serde_json::from_str(settings).map_err(|e| format!("not alignment settings: {e}")))?;
        self.edit(|s| s.set_alignment_settings(method, &settings))
    }

    /// Set optimise settings (`length_weight`, the weights, ...), as JSON text, as `owd optimise
    /// --length-weight` does. Nothing runs.
    pub fn set_optimise_settings(&mut self, settings: &str) -> Js<String> {
        let settings: Value =
            ok(serde_json::from_str(settings).map_err(|e| format!("not optimise settings: {e}")))?;
        self.edit(|s| s.set_optimise_settings(&settings))
    }

    pub fn set_water(&mut self, level: &str, fall: &str) -> Js<String> {
        self.edit(|s| s.set_water_text(level, fall))
    }

    /// Run one stage to its end. `progress`, when given, is called with the number of terrain reads
    /// so far, at most four times a second. The reply is the snapshot, with `run` saying how it ended.
    pub fn run_stage(
        &mut self,
        stage: &str,
        method: Option<String>,
        progress: Option<js_sys::Function>,
    ) -> Js<String> {
        let (job, _control) = ok(self.state.begin_run(stage, method.as_deref()))?;
        // The wasm module has one thread, so the runner's `Sync` bound holds for the JS callback.
        struct OneThread<T>(T);
        // SAFETY: nothing in this module ever starts a second thread.
        unsafe impl<T> Sync for OneThread<T> {}
        let progress = OneThread(progress);
        let last = AtomicU64::new(js_sys::Date::now().to_bits());
        let outcome = job.execute(&|n| {
            if let Some(f) = &progress.0 {
                let now = js_sys::Date::now();
                if now - f64::from_bits(last.load(Ordering::Relaxed)) >= 250.0 {
                    last.store(now.to_bits(), Ordering::Relaxed);
                    let _ = f.call1(&JsValue::NULL, &JsValue::from_f64(n as f64));
                }
            }
        });
        let ended = ok(self.state.finish_run(outcome))?;
        let mut snap: Value = self.state.snapshot();
        snap["run"] = json!({"stage": stage, "outcome": ended});
        Ok(snap.to_string())
    }

    pub fn option_table(&self) -> String {
        self.state.option_table().to_string()
    }

    pub fn map_geometry(&self) -> Js<String> {
        Ok(ok(self.state.map_geometry())?.to_string())
    }

    /// The hillshade of the DEM as a PNG data URL with its corners, or `null` with no DEM.
    pub fn hillshade(&self) -> Js<String> {
        use base64::Engine;
        let Some(dem) = self.state.dem() else {
            return Ok("null".to_string());
        };
        let shade = ok(owd_format::design::hillshade::hillshade(&dem).map_err(|e| e.to_string()))?;
        Ok(json!({
            "url": format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&shade.png)),
            "corners": shade.corners,
            "width": shade.width,
            "height": shade.height,
        })
        .to_string())
    }

    /// What the map needs for the 3D view of the DEM (MAP-2), as JSON: `null` with no DEM in
    /// memory, else the DEM's box in degrees, its lowest and highest height, the deepest zoom
    /// worth asking for, and a number that changes with the DEM.
    pub fn terrain_info(&mut self) -> Js<String> {
        self.sync_channel();
        ok(self.terrain.info(self.state.dem())).map(|v| v.to_string())
    }

    /// Terrain tile `z/x/y` of the ground in `state` (`before`: the DEM; `after`: the DEM with
    /// the channel cut into it, MAP-5): a 256 px Terrarium PNG, made here from the DEM in memory.
    pub fn terrain_tile(&mut self, state: &str, z: u32, x: u32, y: u32) -> Js<Vec<u8>> {
        ok(self.terrain.tile_in(self.state.dem(), state, z, x, y)).map(|png| png.to_vec())
    }

    /// The channel of the design line for the map (MAP-5), as JSON `{key, geometry, note, legend}`:
    /// the bed edges, the water-surface edges and the daylight lines in longitude and latitude.
    /// Derived here; nothing is written to the design.
    pub fn channel_geometry(&mut self) -> String {
        self.sync_channel();
        self.channel.clone()
    }

    /// The cut-depth picture of tile `z/x/y` (MAP-5): an RGBA PNG; no bytes when nothing is cut.
    pub fn cut_tile(&mut self, z: u32, x: u32, y: u32) -> Js<Vec<u8>> {
        ok(self.terrain.cut_tile(self.state.dem(), z, x, y)).map(|png| png.to_vec())
    }

    /// What the cut reads out at a point of the map (MAP-5), as JSON; `null` where nothing is cut.
    pub fn cut_at(&mut self, lon: f64, lat: f64) -> Js<String> {
        ok(self.terrain.cut_at(self.state.dem(), lon, lat)).map(|v| v.to_string())
    }

    /// The contour lines of tile `z/x/y` of the DEM, every `interval_m` metres: a Mapbox Vector
    /// Tile, made here from the DEM in memory (no bytes when the tile has no line).
    pub fn contour_tile(
        &mut self,
        state: &str,
        z: u32,
        x: u32,
        y: u32,
        interval_m: f64,
    ) -> Js<Vec<u8>> {
        ok(self
            .terrain
            .contour_tile_in(self.state.dem(), state, z, x, y, interval_m))
        .map(|tile| tile.to_vec())
    }

    /// The weirs, the locks and the pools of the design, for the map (MAP-4), as JSON: made now
    /// from the water level along the design line. An empty list when the design has no pools.
    pub fn map_structures(&self) -> Js<String> {
        Ok(ok(self.state.map_structures())?.to_string())
    }

    /// The pool water (MAP-4), as JSON: the ground each pool floods, worked out here from the DEM
    /// in memory. `null` with no DEM or no pools. `build_ms` is the time the flood mask took.
    pub fn pool_water_info(&mut self) -> Js<String> {
        let pools = ok(self.state.pool_water_input())?;
        let start = js_sys::Date::now();
        let mut info = ok(self.terrain.pool_water(self.state.dem(), pools))?;
        if info.is_object() {
            info["build_ms"] = json!(js_sys::Date::now() - start);
        }
        Ok(info.to_string())
    }

    /// The water of tile `z/x/y` of pool water `id`: a PNG coloured by depth, clear where dry.
    pub fn pool_water_tile(&mut self, id: u32, z: u32, x: u32, y: u32) -> Js<Vec<u8>> {
        ok(self.terrain.pool_water_tile(id, z, x, y)).map(|png| png.to_vec())
    }

    /// The terrain of tile `z/x/y` with pool water `id` on it: the pool's level where there is
    /// water, the ground elsewhere. A 256 px Terrarium PNG.
    /// `state` names the ground the water stands on: `before`, the DEM; `after`, the DEM with the
    /// channel cut into it and the cut filled to the pool level (MAP-5).
    pub fn pool_surface_tile(
        &mut self,
        id: u32,
        state: &str,
        z: u32,
        x: u32,
        y: u32,
    ) -> Js<Vec<u8>> {
        ok(self.terrain.pool_surface_tile_in(id, state, z, x, y)).map(|png| png.to_vec())
    }

    /// What a click reads on pool water `id`, as JSON: `{pool, water_level_m, ground_m,
    /// depth_m}`, or `null` on dry ground.
    pub fn pool_water_at(&mut self, id: u32, lon: f64, lat: f64) -> Js<String> {
        ok(self.terrain.pool_water_at(id, lon, lat)).map(|v| v.to_string())
    }

    /// The drawing sheets of an option (spec 0004, ticket 8) as JSON: `[{number, title, svg,
    /// warnings}]`, made by the same builder as `owd draw`.
    pub fn sheets(&self, option: &str) -> Js<String> {
        ok(self.state.sheets_job(option).pages()).map(|v| v.to_string())
    }

    /// Each sheet's PDF of an option, as `[[name, bytes], ...]`, with the bytes `owd draw` writes.
    pub fn sheet_pdfs(&self, option: &str) -> Js<js_sys::Array> {
        let pdfs = ok(self.state.sheets_job(option).pdfs())?;
        Ok(pdfs
            .into_iter()
            .map(|(name, bytes)| {
                let pair = js_sys::Array::new();
                pair.push(&JsValue::from_str(&name));
                pair.push(&js_sys::Uint8Array::from(bytes.as_slice()));
                JsValue::from(pair)
            })
            .collect())
    }

    /// The reports of the design (spec 0014, ticket 6), as `[[name, bytes], ...]`, with the bytes
    /// `owd report` writes. `ask` is the JSON `{kind, format, revision?, status?, prepared?, date}`.
    pub fn report_files(&self, option: Option<String>, ask: &str) -> Js<js_sys::Array> {
        let ask: serde_json::Value = ok(serde_json::from_str(ask).map_err(|e| e.to_string()))?;
        let ask = ok(owd_shell::report::ReportAsk::from_json(
            &ask,
            &format!("Open Waterway Designer {} (web)", env!("CARGO_PKG_VERSION")),
        ))?;
        let files = ok(self.state.report_job(option.as_deref(), ask).files())?;
        Ok(files
            .into_iter()
            .map(|(name, bytes)| {
                let pair = js_sys::Array::new();
                pair.push(&JsValue::from_str(&name));
                pair.push(&js_sys::Uint8Array::from(bytes.as_slice()));
                JsValue::from(pair)
            })
            .collect())
    }

    /// The QA overlay of the study's banks (spec 0013): `[[name, bytes], ...]` for the PNG and the
    /// KMZ, the scenes read through `fetch`, as the desktop's export writes them.
    pub fn qa_overlay(
        &self,
        from_km: Option<f64>,
        to_km: Option<f64>,
        at: &str,
        fetch: js_sys::Function,
    ) -> Js<js_sys::Array> {
        let at = ok(endpoints(at))?;
        let job = ok(self.state.qa_job())?;
        let files = ok(job.files(&JsFetch(fetch), &at, from_km, to_km))?;
        Ok(files
            .into_iter()
            .map(|(name, bytes)| {
                let pair = js_sys::Array::new();
                pair.push(&JsValue::from_str(&name));
                pair.push(&js_sys::Uint8Array::from(bytes.as_slice()));
                JsValue::from(pair)
            })
            .collect())
    }

    pub fn long_section(&mut self, option: &str) -> Js<String> {
        let w = self.walked(option, &[])?;
        Ok(ok(w
            .session
            .long_section(option, &w.walk)
            .map_err(|e| e.to_string()))?
        .to_string())
    }

    pub fn cross_section(&mut self, option: &str, chainage: f64) -> Js<String> {
        let w = self.walked(option, &[chainage])?;
        Ok(ok(w
            .session
            .cross_section(&w.walk, chainage)
            .map_err(|e| e.to_string()))?
        .to_string())
    }

    pub fn station_near(&mut self, option: &str, lon: f64, lat: f64) -> Js<f64> {
        let w = self.walked(option, &[])?;
        ok(w.session
            .station_near(&w.walk, lon, lat)
            .map_err(|e| e.to_string()))
    }

    /// The options as GeoJSON in longitude and latitude.
    pub fn export_geojson(&self) -> Js<String> {
        ok(self.state.export_geojson())
    }

    /// The quantity table as CSV.
    pub fn export_csv(&self) -> String {
        self.state.export_csv()
    }
}

impl Default for Session {
    fn default() -> Self {
        Session::new()
    }
}
