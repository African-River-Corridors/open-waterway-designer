//! The design session: the one interface between the engine and its users (the CLI and the app).
//! It takes and returns text and values. The callers do all file, network and clock access.
//!
//! The document is a JSON value (`schema/owd.schema.json`). A stage's status is never stored:
//! it is worked out on load from the stage's recorded dependency hash.

use super::crs::{Crs, Utm};
use super::geojson::{self, Role};
use super::hash::hash_value;
use super::river::{RiverGeometry, RiverIn, river};
use super::rule_sets::default_settings;
use super::schema;
use crate::types::{Polyline, RiverPolygon, StageError, TerrainSampler};
use arc_rules::load::Library;
use owd_core::rules::engine;
use serde_json::{Value, json};

/// The engine version written into documents and stages.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The document format version this engine writes: the string `"0.5"` since spec 0017 ticket 6 (one
/// design line). A 0.4 file opens and is saved as 0.5 ([`super::migrate::from_0_4`]): its design
/// line options become the one design line; a stage that was fresh under the 0.4 rules stays fresh,
/// a stale one stays stale; and it is turned to run from its mouth when that can be told. A file with
/// the number `3` (format 0.3, spec 0007) opens as 0.4 first. An older file is refused; there is no
/// migration. Until format 1.0 (after an external review) a format change raises the minor number.
pub const FORMAT_VERSION: &str = "0.5";

/// Format 0.4: the last format with a list of design line options.
pub(super) const FORMAT_0_4: &str = "0.4";

/// The format 0.3 files wrote as a number.
const FORMAT_0_3: i64 = 3;

/// A format version string as `(major, minor)`, such as `"0.4"` -> `(0, 4)`.
fn format_number(v: &str) -> Option<(u64, u64)> {
    let (a, b) = v.split_once('.')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

/// The UTC calendar date, `YYYY-MM-DD`, of a Unix time in seconds. The session has no clock: a caller
/// reads one and passes the seconds here when it needs a date to write into a new document.
pub fn date_from_unix(secs: u64) -> String {
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// A stage of a channel design, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Sizing,
    Alignment,
    Optimise,
    Volumes,
}

impl Stage {
    pub const ALL: [Stage; 4] = [
        Stage::Sizing,
        Stage::Alignment,
        Stage::Optimise,
        Stage::Volumes,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Stage::Sizing => "sizing",
            Stage::Alignment => "alignment",
            Stage::Optimise => "optimise",
            Stage::Volumes => "volumes",
        }
    }

    pub fn from_name(name: &str) -> Option<Stage> {
        Stage::ALL.into_iter().find(|s| s.name() == name)
    }
}

/// Where a stage stands. Worked out on load, never stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageStatus {
    NotRun,
    Fresh,
    Stale,
}

impl StageStatus {
    pub fn name(self) -> &'static str {
        match self {
            StageStatus::NotRun => "not run",
            StageStatus::Fresh => "fresh",
            StageStatus::Stale => "stale",
        }
    }
}

/// Why a session call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesignError {
    /// The caller gave something the tool cannot use (a bad file, an unknown ruleset, a CRS it
    /// does not support). The message says what to change.
    Input(String),
    /// The document breaks the schema or its own rules; one line per break.
    Document(Vec<String>),
    /// A stage that cannot run yet: a later ticket builds it.
    NotAvailable(String),
    /// A stage whose inputs are stale refuses to run, and names the stage to run first.
    StaleUpstream { stage: Stage, run_first: Stage },
}

impl core::fmt::Display for DesignError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            DesignError::Input(m) | DesignError::NotAvailable(m) => f.write_str(m),
            DesignError::Document(errs) => {
                write!(f, "the channel design is not valid: {}", errs.join("; "))
            }
            DesignError::StaleUpstream { stage, run_first } => write!(
                f,
                "the {} stage cannot run: {} is stale. Run {} first.",
                stage.name(),
                run_first.name(),
                run_first.name()
            ),
        }
    }
}

impl std::error::Error for DesignError {}

/// Refuse a design file named with the old extension, `.cdesign` (spec 0006, T10). Its content is a
/// `.owd` file's, but the old name is no longer read: the message says to rename the file. Every
/// caller that opens or creates a design by file name checks the name here first.
pub fn check_file_name(name: &str) -> Result<(), DesignError> {
    let old = std::path::Path::new(name)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("cdesign"));
    if old {
        return Err(DesignError::Input(format!(
            "{name}: `.cdesign` is the old name of the design file: rename it to `.owd` (the content is the same)"
        )));
    }
    Ok(())
}

/// The state of a channel design.
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub stages: Vec<(Stage, StageStatus)>,
}

/// The share of the checked stations over which a failed side check is wholesale: the banks are
/// named the wrong way round for the way they run, one fact, not a fault at each station.
pub const SIDE_WHOLESALE_SHARE: f64 = 0.8;

/// The one plain message for a side check that fails at `side_fails` of `checked` stations, when
/// that is over [`SIDE_WHOLESALE_SHARE`]; `None` otherwise.
pub fn side_wholesale_message(side_fails: usize, checked: usize) -> Option<String> {
    (checked > 0 && side_fails as f64 > SIDE_WHOLESALE_SHARE * checked as f64).then(|| {
        format!(
            "the banks fail the side check at {side_fails} of {checked} stations: they look named \
             the wrong way round for the direction they run. If left and right are map sides, or the \
             banks run upstream, give the downstream point (`--downstream LON,LAT` on the command \
             line, or the downstream point in the app), or swap the two files"
        )
    })
}

/// What an import changed.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportReport {
    /// The project CRS after the import (EPSG code).
    pub project_epsg: u32,
    /// `(input name, source CRS as EPSG:nnnn, positions read)` per input.
    pub inputs: Vec<(String, String, usize)>,
    /// What the bank checks found, one line per failed check; empty when every check passes. The
    /// checks report: they never repair or refuse the banks.
    pub warnings: Vec<String>,
}

/// What a note from opening a file asks of the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NoteKind {
    /// It says what opening changed. Nothing was lost and there is nothing to do.
    Info,
    /// The user has something to do or to check: a stage to run again, a line that was not kept.
    Action,
}

/// One note from opening a file made by an older OWD: a plain sentence and its kind.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct OpenNote {
    pub kind: NoteKind,
    pub text: String,
}

impl OpenNote {
    pub fn info(text: impl Into<String>) -> Self {
        OpenNote {
            kind: NoteKind::Info,
            text: text.into(),
        }
    }

    pub fn action(text: impl Into<String>) -> Self {
        OpenNote {
            kind: NoteKind::Action,
            text: text.into(),
        }
    }
}

/// One channel design.
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub(super) doc: Value,
    pub(super) water_sampling: super::water::WaterSampling,
    /// The Rule Sets the session can select: the built-in ones and any loaded at run time.
    pub(super) library: Library,
    /// What opening the file changed, in plain words (a move from an older format). Not stored.
    pub(super) notes: Vec<OpenNote>,
    /// The cut-rate multiplier raster the document names (spec 0017 ticket 13), read from the file
    /// whose hash it recorded, when the host has attached it. Not stored: the document holds only
    /// its reference, as for the DEM.
    pub(super) multiplier: Option<std::sync::Arc<crate::geotiff::Dem>>,
    /// A file moved from format 0.4 whose mouth only the elevation model can tell, while the host
    /// has not loaded it yet ([`Session::settle_old_mouth`]). Not stored.
    pub(super) old_mouth_pending: bool,
}

fn empty_stage() -> Value {
    json!({"settings": null, "results": null, "depends_on": null, "engine_version": null})
}

/// The sizing stage of a new design: the default Rule Set selection, no vessel yet.
fn sizing_stage() -> Value {
    let mut st = empty_stage();
    st["settings"] = default_settings();
    st
}

impl Session {
    /// An empty channel design with a title. `created` is `YYYY-MM-DD`, given by the caller.
    pub fn new(title: &str, created: &str) -> Session {
        Session {
            water_sampling: Default::default(),
            library: engine::built_in().clone(),
            notes: Vec::new(),
            multiplier: None,
            old_mouth_pending: false,
            doc: json!({
                "format_version": FORMAT_VERSION,
                "engine_version": ENGINE_VERSION,
                "project": {"title": title, "created": created, "crs": null, "source_crs": {}},
                "inputs": {
                    "left_bank": null, "right_bank": null, "centreline": null,
                    "river_polygon": null, "dem": null, "water_level": null
                },
                "stages": {
                    "sizing": sizing_stage(), "alignment": empty_stage(),
                    "optimise": empty_stage(), "volumes": empty_stage()
                }
            }),
        }
    }

    /// Parse and validate `.owd` text.
    ///
    /// A format 0.4 (or 0.3) file moves to this format ([`super::migrate`]): each stage that was
    /// fresh under the 0.4 rules when it opened stays fresh, with its numbers; a stale one stays
    /// stale. Its mouth is worked out from its water level and stored, and when the mouth is the far
    /// end the river and the stored results are turned to run from it. With no water level to tell,
    /// the elevation model tells, once the host has loaded it ([`Session::settle_old_mouth`]).
    pub fn open(text: &str) -> Result<Session, DesignError> {
        let mut doc: Value = serde_json::from_str(text)
            .map_err(|e| DesignError::Input(format!("the file is not valid JSON: {e}")))?;
        let mut notes = Vec::new();
        // The stages kept fresh by a move from format 0.4, if the file is one.
        let mut moved: Option<Vec<Stage>> = None;
        if let Value::Number(n) = &doc["format_version"]
            && n.as_i64() == Some(FORMAT_0_3)
        {
            // Format 0.3 reads as 0.4 (spec 0015), which moves to 0.5 below.
            doc["format_version"] = json!(FORMAT_0_4);
        }
        if doc["format_version"] == FORMAT_0_4 {
            let fresh = super::migrate::fresh_0_4(&doc);
            let (n, keep) = super::migrate::from_0_4(&mut doc, &fresh);
            notes = n;
            moved = Some(keep);
        }
        // Answers stored by an older OWD say why they show stale (read before the line moves: the
        // old hashes read `inputs.reference_line`).
        notes.extend(super::fits::older_answer_note(&doc).map(OpenNote::action));
        notes.extend(super::envelope::older_answer_note(&doc).map(OpenNote::action));
        // A reference line stored the old way moves to its stage record (spec 0017 ticket 7).
        let (moved_line, moved_fresh) = super::reference_line::move_from_inputs(&mut doc);
        notes.extend(moved_line);
        match &doc["format_version"] {
            Value::Number(n) => {
                return Err(DesignError::Input(format!(
                    "the file is document format version {n}; this OWD opens format \
                     {FORMAT_VERSION}, 0.4 and 0.3 only, and moves no older file (spec 0007). Rebuild \
                     the design: start a new one, import its inputs, size it and run its stages \
                     again."
                )));
            }
            Value::String(v)
                if format_number(v).is_some_and(|n| Some(n) > format_number(FORMAT_VERSION)) =>
            {
                return Err(DesignError::Input(format!(
                    "the file is document format {v}, made by a newer OWD; this OWD opens format \
                     {FORMAT_VERSION} and older. Update OWD to open it."
                )));
            }
            // Anything else is for the schema to name.
            _ => {}
        }
        let mut s = Session {
            doc,
            water_sampling: Default::default(),
            library: engine::built_in().clone(),
            notes,
            multiplier: None,
            old_mouth_pending: false,
        };
        // A moved line that was fresh keeps a fresh record: its hash becomes the record's own.
        if moved_fresh && let Some(h) = s.reference_line_record_hash() {
            s.doc["stages"][super::reference_line::STAGE]["depends_on"] = json!(h);
        }
        // The stages a move from 0.4 kept fresh get the hash of this format.
        if let Some(keep) = &moved {
            s.finish_moved_lines(keep, false);
            s.stamp_fresh(keep);
        }
        if s.doc["inputs"].is_object() && s.validate_schema_only().is_empty() {
            s.migrate_water_settings();
            s.canonical_vessel_ids();
            if moved.is_some() {
                let n = s.old_file_mouth(None, true);
                s.notes.extend(n);
            }
        }
        let errors = s.validate();
        if errors.is_empty() {
            Ok(s)
        } else {
            Err(DesignError::Document(errors))
        }
    }

    /// Older studies name a vessel class by an id the shared catalogue replaced (`CEMT_Va`). The
    /// class still works; the study moves to the catalogue id (`CEMT-Va`), and stages that were fresh
    /// stay fresh (spec 0015 T0).
    fn canonical_vessel_ids(&mut self) {
        let fresh: Vec<Stage> = Stage::ALL
            .iter()
            .copied()
            .filter(|s| self.stage_status(*s) == StageStatus::Fresh)
            .collect();
        let mut moved = false;
        for part in ["settings", "results"] {
            // `get_mut`, not indexing: indexing would turn a missing part into an empty object.
            let Some(at) = self
                .doc
                .pointer_mut(&format!("/stages/sizing/{part}/vessel_standard_id"))
            else {
                continue;
            };
            if let Some(id) = at.as_str()
                && let Some(new) = owd_core::rules::classes::canonical_id(id)
                && new != id
            {
                *at = json!(new);
                moved = true;
            }
        }
        if moved {
            for s in fresh {
                let h = self.dependency_hash(s);
                self.doc["stages"][s.name()]["depends_on"] = json!(h);
            }
        }
    }

    /// What opening the file changed, one plain sentence each: empty unless the file was made by an
    /// older OWD and moved to this format. The command line prints them; the app shows them.
    pub fn open_notes(&self) -> Vec<String> {
        self.notes.iter().map(|n| n.text.clone()).collect()
    }

    /// The same notes with their kind: information, or something for the user to do. The app
    /// shows the two kinds apart.
    pub fn open_notes_by_kind(&self) -> &[OpenNote] {
        &self.notes
    }

    /// The document text: two-space indented JSON with a final newline.
    pub fn to_text(&self) -> String {
        let mut t = serde_json::to_string_pretty(&self.doc).expect("a JSON value serialises");
        t.push('\n');
        t
    }

    /// One stage as a [`StageRecord`](crate::stage::StageRecord): the settings, the output (the
    /// stage's `results`), the dependency hash and the contract version. Format 0.5 still keeps the
    /// keys `results` and `engine_version` (the rename to `output` and `version` touches every reader,
    /// and is left to its own change), so this is a typed read of them, not a second copy.
    pub fn stage_record(&self, stage: Stage) -> crate::stage::StageRecord<Value, Value> {
        let st = &self.doc["stages"][stage.name()];
        crate::stage::StageRecord {
            version: crate::stage::STAGE_CONTRACT_VERSION,
            depends_on: st["depends_on"].as_str().map(str::to_string),
            settings: st["settings"].clone(),
            output: (!st["results"].is_null()).then(|| st["results"].clone()),
        }
    }

    /// The document as a value.
    pub fn document(&self) -> &Value {
        &self.doc
    }

    /// The published JSON Schema text.
    pub fn schema_text() -> &'static str {
        schema::SCHEMA_TEXT
    }

    fn validate_schema_only(&self) -> Vec<String> {
        schema::validate(&schema::schema(), &self.doc)
    }

    /// Every way the document breaks the schema or its own rules. Empty when it is valid.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = schema::validate(&schema::schema(), &self.doc);
        if !errors.is_empty() {
            return errors;
        }
        for stage in Stage::ALL {
            let st = &self.doc["stages"][stage.name()];
            if !st["results"].is_null() && st["depends_on"].is_null() {
                errors.push(format!(
                    "/stages/{}: has results but no `depends_on` hash",
                    stage.name()
                ));
            }
            if !st["results"].is_null() && st["engine_version"].is_null() {
                errors.push(format!(
                    "/stages/{}: has results but no `engine_version`",
                    stage.name()
                ));
            }
        }
        // The run inputs are typed and checked beyond what the schema states (ticket 11).
        if let Err(e) = self.run_inputs() {
            errors.push(format!("/inputs/run: {e}"));
        }
        // The rates table too (ticket 12).
        if let Err(e) = self.rates() {
            errors.push(format!("/inputs/rates: {e}"));
        }
        // And the constraints (ticket 13).
        if let Err(e) = self.constraints() {
            errors.push(format!("/inputs/constraints: {e}"));
        }
        // The one design line, and the lattice's, are typed: each must read as a `DesignLine`.
        for (at, v) in [
            (
                "/stages/alignment/results",
                &self.doc["stages"]["alignment"]["results"],
            ),
            (
                "/stages/optimise/results/design_line",
                &self.doc["stages"]["optimise"]["results"]["design_line"],
            ),
        ] {
            if !v.is_null()
                && let Err(e) = serde_json::from_value::<super::design_line::DesignLine>(v.clone())
            {
                errors.push(format!("{at}: is not a design line: {e}"));
            }
        }
        errors
    }

    // --- status -------------------------------------------------------------------------

    /// What a stage depended on, hashed: the inputs, the project CRS, and the settings and results
    /// of the stage and of every stage before it.
    pub(super) fn dependency_hash(&self, stage: Stage) -> String {
        self.dependency_hash_with(stage, self.selected_set_hashes())
    }

    /// [`Session::dependency_hash`] with `rule_sets` as the content hashes of the selected Rule Sets.
    pub(super) fn dependency_hash_with(&self, stage: Stage, rule_sets: Value) -> String {
        let st = |s: Stage| &self.doc["stages"][s.name()];
        let part = |s: Stage| json!({"settings": st(s)["settings"], "results": st(s)["results"]});
        let upto = |s: Stage| Stage::ALL.iter().take_while(|x| **x != s).count();
        let mut chain = serde_json::Map::new();
        for s in Stage::ALL.iter().take(upto(stage) + 1) {
            if *s == stage {
                chain.insert(s.name().into(), json!({"settings": st(*s)["settings"]}));
            } else {
                chain.insert(s.name().into(), part(*s));
            }
        }
        let mut dep = json!({
            "stage": stage.name(),
            "chain": Value::Object(chain),
        });
        // The content of every selected Rule Set, as the engine holds it now: a changed set makes the
        // sizing stage, and so every stage after it, stale.
        dep["rule_sets"] = rule_sets;
        // The marked reaches decide which radius Rule applies where, so sizing depends on them.
        if let Some(r) = self.doc["inputs"].get("waterway_reaches") {
            dep["waterway_reaches"] = r.clone();
        }
        if stage != Stage::Sizing {
            dep["inputs"] = self.doc["inputs"].clone();
            // The DEM is identified by its hash, not by where the file sits: moving a design and its
            // DEM to another folder must not make its results stale.
            if let Some(dem) = dep["inputs"]["dem"].as_object_mut() {
                dem.remove("path");
            }
            // The rolling circle reads neither the DEM nor the water level: one added after it ran
            // leaves it fresh. The re-route, the optimise stage and the volumes read them.
            let reads_dem = match stage {
                Stage::Alignment => {
                    self.doc["stages"]["alignment"]["settings"]["method"] == "reroute"
                }
                _ => true,
            };
            if !reads_dem && let Some(inputs) = dep["inputs"].as_object_mut() {
                inputs.remove("dem");
                inputs.remove("water_level");
                // The cut-rate multiplier stays (ticket 13): every method's line is walked over it
                // for hard-exclusion ground.
            }
            // The multiplier raster is identified by its hash, as the DEM is, not by where it sits.
            // (`get_mut`, not indexing: indexing would add the key to a document without it.)
            if let Some(m) = dep
                .get_mut("inputs")
                .and_then(|i| i.get_mut(super::constraints::INPUT))
                .and_then(|c| c.get_mut("cut_rate_multiplier"))
                .and_then(Value::as_object_mut)
            {
                m.remove("path");
            }
            dep["crs"] = self.doc["project"]["crs"].clone();
            // Only the volumes read the bed model (spec 0012): a new depth leaves the stages
            // before them fresh.
            if stage != Stage::Volumes
                && let Some(inputs) = dep["inputs"].as_object_mut()
            {
                inputs.remove("bed_model");
            }
            // No stage reads the reference line (spec 0016): storing it leaves them fresh. The
            // volumes read the assumed bed, which is centred on it (#398).
            if let Some(inputs) = dep["inputs"].as_object_mut() {
                inputs.remove("reference_line");
                // No stage of the chain reads the run inputs yet (spec 0017, ticket 11): setting
                // them leaves every stage fresh, and a file saved before them keeps its hashes. The
                // stage that first reads them adds them to its own hash.
                inputs.remove(super::run_inputs::INPUT);
                // Nor the rates table (ticket 12): only the cost reads it, and cost is not stored.
                inputs.remove(super::cost::INPUT);
            }
            if stage == Stage::Volumes
                && self.bed_model_value().is_some()
                && let Some(a) = self.bed_axis_hash()
            {
                dep["bed_axis"] = a;
            }
        }
        hash_value(&dep)
    }

    /// The hash of what `stage` depends on now: a stage is fresh while it equals the one recorded.
    pub fn stage_hash(&self, stage: Stage) -> String {
        self.dependency_hash(stage)
    }

    pub fn stage_status(&self, stage: Stage) -> StageStatus {
        let st = &self.doc["stages"][stage.name()];
        if st["results"].is_null() {
            return StageStatus::NotRun;
        }
        if st["depends_on"].as_str() == Some(self.dependency_hash(stage).as_str()) {
            StageStatus::Fresh
        } else {
            StageStatus::Stale
        }
    }

    pub fn status(&self) -> Status {
        Status {
            stages: Stage::ALL
                .iter()
                .map(|s| (*s, self.stage_status(*s)))
                .collect(),
        }
    }

    /// A warning when the DEM the document names is missing or is not the file whose hash was
    /// recorded. `bytes` is the file the caller found at the recorded path, if any.
    pub fn dem_warning<S: crate::sectioned::ByteSource + ?Sized>(
        &self,
        bytes: Option<&S>,
    ) -> Option<String> {
        let dem = self.doc["inputs"]["dem"].as_object()?;
        let path = dem["path"].as_str().unwrap_or("?");
        match self.dem_check(bytes) {
            super::volumes::DemCheck::Missing => Some(format!(
                "the DEM `{path}` is missing: the volumes may no longer hold"
            )),
            super::volumes::DemCheck::Changed => Some(format!(
                "the DEM `{path}` has changed since its hash was recorded: the volumes may no longer hold"
            )),
            _ => None,
        }
    }

    // --- sizing -------------------------------------------------------------------------

    /// The sizing results for a Rule Set selection (set ids, and set inputs as `(name, value)`),
    /// a vessel class and a draught, without touching any document. `owd` is always selected.
    pub fn preview_sizing(
        rule_sets: &[&str],
        set_inputs: &[(&str, &str)],
        vessel_standard_id: &str,
        draught_m: Option<f64>,
    ) -> Result<Value, DesignError> {
        let mut s = Session::new("", "");
        s.set_rule_sets(rule_sets, set_inputs)?;
        s.set_vessel(vessel_standard_id, draught_m)?;
        s.sizing_results()
    }

    /// The sizing results of the sizing settings: the channel shape, and B, the design basis (spec
    /// 0007), each quantity with its governing Rule.
    fn sizing_results(&self) -> Result<Value, DesignError> {
        let size = super::sizing::size(&self.sizing_in(), &self.library)
            .map_err(|e| DesignError::Input(e.message))?;
        Ok(serde_json::to_value(&size).expect("a channel size is JSON"))
    }

    pub(super) fn run_sizing(&mut self) -> Result<(), DesignError> {
        self.doc["stages"]["sizing"]["results"] = self.sizing_results()?;
        self.doc["stages"]["sizing"]["engine_version"] = json!(ENGINE_VERSION);
        let h = self.dependency_hash(Stage::Sizing);
        self.doc["stages"]["sizing"]["depends_on"] = json!(h);
        Ok(())
    }

    /// Run one stage with no terrain. See [`Session::run_stage_with`].
    pub fn run_stage(&mut self, stage: Stage) -> Result<(), DesignError> {
        self.run_stage_with(stage, None)
    }

    /// Run one stage. A stage whose earlier stages are stale refuses, and names the one to run
    /// first. `terrain` is the DEM in the project CRS, for the stages that need it (the re-route and
    /// the volumes); the caller reads the file.
    pub fn run_stage_with(
        &mut self,
        stage: Stage,
        terrain: Option<TerrainSampler>,
    ) -> Result<(), DesignError> {
        for earlier in Stage::ALL.iter().take_while(|s| **s != stage) {
            if self.stage_status(*earlier) == StageStatus::Stale {
                return Err(DesignError::StaleUpstream {
                    stage,
                    run_first: *earlier,
                });
            }
        }
        match stage {
            Stage::Sizing => self.run_sizing(),
            Stage::Alignment => self.rerun_alignment(terrain),
            Stage::Volumes => self.run_volumes(&Value::Null, terrain),
            Stage::Optimise => self.run_optimise(&Value::Null, terrain),
        }
    }

    /// Whether `stage` is due now: stale, or not run yet with settings to run on. A new design
    /// has a Rule Set selection but no vessel, so sizing is not due until it has one.
    fn is_due(&self, stage: Stage) -> bool {
        match self.stage_status(stage) {
            StageStatus::Stale => true,
            StageStatus::NotRun => {
                let settings = &self.doc["stages"][stage.name()]["settings"];
                !settings.is_null()
                    && (stage != Stage::Sizing
                        || !settings["vessel_standard_id"].is_null()
                        || settings["custom_vessel"].is_object())
            }
            StageStatus::Fresh => false,
        }
    }

    /// The stages due now, in order, by the rule [`Session::run_stale_with`] runs. Running one can
    /// make a later one due, so a surface that steps through them asks again after each.
    pub fn due_stages(&self) -> Vec<Stage> {
        Stage::ALL.into_iter().filter(|s| self.is_due(*s)).collect()
    }

    /// Run every stage that is due (stale, or with settings but no results), in order, on
    /// `terrain`. Returns the stages that ran. This is the one "run what is due" loop: the command
    /// line and the app shells all call it. A stage that cannot run is an error.
    pub fn run_stale_with(
        &mut self,
        terrain: Option<TerrainSampler>,
    ) -> Result<Vec<Stage>, DesignError> {
        let mut ran = Vec::new();
        for stage in Stage::ALL {
            if self.is_due(stage) {
                self.run_stage_with(stage, terrain)?;
                ran.push(stage);
            }
        }
        Ok(ran)
    }

    /// The Rule Sets, their set inputs and the vessel classes a user can choose for sizing, for
    /// the lists in the app: ids, display labels and the dimensions that matter. `owd` is listed
    /// apart, as `always`: it is always selected.
    pub fn sizing_choices() -> Value {
        sizing_choices_of(super::rule_sets::rule_set_choices())
    }

    /// The sizing choices of this session (ticket #499): as [`Session::sizing_choices`], with the
    /// Rule Sets loaded from a file listed after the built-in ones, each marked `"from_file":
    /// true`.
    pub fn sizing_choices_for(&self) -> Value {
        sizing_choices_of(super::rule_sets::rule_set_choices_in(
            self.rule_set_library(),
        ))
    }
}

/// The sizing choices with these Rule Sets (to pick, and always selected) and every vessel class.
fn sizing_choices_of((rule_sets, always): (Vec<Value>, Vec<Value>)) -> Value {
    // Every class a channel can be sized for (spec 0015 T0), with its family for the picker's
    // groups and the words its search matches; `default` marks the 12 `owd fits` checks.
    let defaults = owd_core::rules::classes::default_class_ids();
    let vessels: Vec<Value> = owd_core::rules::full_catalogue()
        .into_iter()
        .map(|(id, v)| {
            let (lo, hi) = v.loa_m.unwrap_or((f64::NAN, f64::NAN));
            let loa = if lo == hi {
                format!("{hi}")
            } else {
                format!("{lo}-{hi}")
            };
            json!({
                "id": id,
                "label": format!(
                    "{id} ({}, LOA {loa} m, beam {} m)",
                    v.formation.replace('_', " "),
                    v.beam_m.unwrap_or(f64::NAN)
                ),
                "family": v.family,
                "region": v.region,
                "formation": v.formation,
                "loa_m": v.loa_m.map(|(a, b)| [a, b]),
                "beam_m": v.beam_m,
                "draught_nominal_m": v.draught_nominal_m(),
                "default": defaults.contains(&id.as_str()),
            })
        })
        .collect();
    json!({"rule_sets": rule_sets, "always": always, "vessels": vessels})
}

impl Session {
    // --- inputs -------------------------------------------------------------------------

    /// Import inputs from GeoJSON text. Reads every input first and changes nothing if one is
    /// refused. The first import chooses the project CRS: the UTM zone of a UTM input, or the zone
    /// and hemisphere at the centroid of the bank lines for WGS84 input. Later imports use it.
    ///
    /// After the import the two banks are a sound pair (ticket B2): both run the way the centreline
    /// runs (downstream), or with no centreline the way the left bank runs; left and right are named
    /// looking downstream when a centreline says which way that is; two banks that cross are
    /// refused; and two banks with no given river polygon give one (the left bank, then the right
    /// bank backwards). A polygon built that way is built again when a bank is imported again.
    pub fn import(&mut self, items: &[(Role, &str)]) -> Result<ImportReport, DesignError> {
        self.import_with(items, None, true)
    }

    /// Import as [`Session::import`] does, with no centreline needed to tell which way the river
    /// flows: both banks run towards `downstream_lon_lat` (longitude, latitude, WGS84), and the
    /// sides are named looking that way. A point that is no longitude and latitude is refused, and
    /// so is a point that disagrees with the centreline.
    pub fn import_downstream(
        &mut self,
        items: &[(Role, &str)],
        downstream_lon_lat: [f64; 2],
    ) -> Result<ImportReport, DesignError> {
        let [lon, lat] = downstream_lon_lat;
        if !(lon.is_finite() && lat.is_finite() && lon.abs() <= 180.0 && lat.abs() <= 90.0) {
            return Err(DesignError::Input(format!(
                "the downstream point ({lon}, {lat}) is not a longitude and latitude"
            )));
        }
        self.import_with(items, Some(downstream_lon_lat), true)
    }

    /// Import as [`Session::import`] does, but keep two banks that cross instead of refusing them.
    ///
    /// This exists only for banks a machine made (the bank extraction from imagery, through
    /// `owd_reach::banks::record_banks_as`, its one caller). The bank checks must run after it and
    /// report the crossing. A user's files never come in this way: the command line and the app
    /// shell use [`Session::import`]. It goes when bank making leaves the repository.
    pub fn import_unchecked(
        &mut self,
        items: &[(Role, &str)],
    ) -> Result<ImportReport, DesignError> {
        self.import_with(items, None, false)
    }

    /// An import into a design with a mouth sees the stored lines run towards the mouth, as an import
    /// of a design without one does, and names the banks looking that way; afterwards the lines are
    /// turned to run from the mouth again. With no downstream point given, the mouth is the point the
    /// river runs towards.
    fn import_with(
        &mut self,
        items: &[(Role, &str)],
        downstream: Option<[f64; 2]>,
        refuse_crossing: bool,
    ) -> Result<ImportReport, DesignError> {
        let Some(mouth) = self.stored_mouth() else {
            return self.import_inner(items, downstream, None, refuse_crossing);
        };
        let mut view = self.clone();
        view.turn_lines();
        let report = view.import_inner(items, downstream, Some(mouth.point), refuse_crossing)?;
        view.lines_from_mouth(mouth)?;
        *self = view;
        Ok(report)
    }

    fn import_inner(
        &mut self,
        items: &[(Role, &str)],
        downstream: Option<[f64; 2]>,
        mouth_xy: Option<[f64; 2]>,
        refuse_crossing: bool,
    ) -> Result<ImportReport, DesignError> {
        if items.is_empty() {
            return Err(DesignError::Input("nothing to import".into()));
        }
        let mut parsed = Vec::new();
        for (role, text) in items {
            let p = geojson::parse(*role, text)
                .map_err(|e| DesignError::Input(format!("{}: {e}", role.name())))?;
            parsed.push((*role, p));
        }
        let existing: Option<Utm> = self.doc["project"]["crs"]["epsg"]
            .as_u64()
            .and_then(|c| Crs::from_epsg(c as u32).ok())
            .and_then(|c| match c {
                Crs::Utm(u) => Some(u),
                Crs::Wgs84 => None,
            });
        let target = match existing {
            Some(u) => u,
            None => {
                // A UTM input names the zone; otherwise the centroid of the bank lines (any line
                // or polygon if there are no banks in this import) picks it.
                let utm_input = parsed.iter().find_map(|(_, p)| match p.crs {
                    Crs::Utm(u) => Some(u),
                    Crs::Wgs84 => None,
                });
                match utm_input {
                    Some(u) => u,
                    None => {
                        let pick = |banks_only: bool| {
                            let pts: Vec<[f64; 2]> = parsed
                                .iter()
                                .filter(|(r, _)| !banks_only || r.is_bank())
                                .flat_map(|(_, p)| geojson::all_positions(&p.geometry))
                                .collect();
                            (!pts.is_empty()).then_some(pts)
                        };
                        let pts = pick(true).or_else(|| pick(false)).ok_or_else(|| {
                            DesignError::Input("the input has no positions".into())
                        })?;
                        let n = pts.len() as f64;
                        let lon = pts.iter().map(|p| p[0]).sum::<f64>() / n;
                        let lat = pts.iter().map(|p| p[1]).sum::<f64>() / n;
                        if !(lon.abs() <= 180.0 && lat.abs() <= 84.0) {
                            return Err(DesignError::Input(format!(
                                "the input is marked WGS84 but its centre ({lon:.4}, {lat:.4}) is not a \
                                 longitude and latitude inside the UTM limits (84 N to 80 S). If the \
                                 data is projected, declare its CRS with a `crs` member naming \
                                 WGS84 / UTM."
                            )));
                        }
                        Utm::for_point(lon, lat)
                    }
                }
            }
        };
        let mut converted = Vec::new();
        for (role, p) in &parsed {
            let mut g = geojson::to_project_crs(p, target)
                .map_err(|e| DesignError::Input(format!("{}: {e}", role.name())))?;
            // With a mouth, a new centreline runs towards it, as the stored lines do here.
            if *role == Role::Centreline
                && downstream.is_none()
                && let Some(m) = mouth_xy
                && line_points(&g).is_some_and(|c| super::bank_pair::runs_away_from(&c, m))
            {
                reverse_line(&mut g);
            }
            converted.push((*role, p.crs, g));
        }
        let downstream_xy = match downstream {
            Some([lon, lat]) => Some(
                target
                    .forward(lon, lat)
                    .map_err(|e| DesignError::Input(format!("the downstream point: {e}")))?,
            ),
            None => mouth_xy,
        };
        let pair = self.sound_pair(&converted, downstream_xy, refuse_crossing)?;
        let brings_a_bank = converted.iter().any(|(r, _, _)| r.is_bank());
        let mut report = ImportReport {
            project_epsg: target.epsg(),
            inputs: Vec::new(),
            warnings: Vec::new(),
        };
        self.doc["project"]["crs"] = json!({"epsg": target.epsg(), "name": target.name()});
        for (role, source, g) in converted {
            report.inputs.push((
                role.name().to_string(),
                format!("EPSG:{}", source.epsg()),
                geojson::all_positions(&g).len(),
            ));
            self.doc["project"]["source_crs"][role.name()] =
                json!(format!("EPSG:{}", source.epsg()));
            self.doc["inputs"][role.name()] = g;
        }
        if let Some(pair) = pair {
            let (l, r) = (Role::LeftBank.name(), Role::RightBank.name());
            if pair.swapped {
                let src = &mut self.doc["project"]["source_crs"];
                let (a, b) = (src[l].take(), src[r].take());
                for (k, v) in [(l, b), (r, a)] {
                    if !v.is_null() {
                        src[k] = v;
                    }
                }
                if let Some(m) = src.as_object_mut() {
                    m.retain(|_, v| !v.is_null());
                }
            }
            self.doc["inputs"][l] = pair.left;
            self.doc["inputs"][r] = pair.right;
            if let Some(p) = pair.polygon {
                self.doc["inputs"][Role::RiverPolygon.name()] = p;
            }
        }
        if brings_a_bank {
            report.warnings = self.bank_check_warnings(!refuse_crossing);
        }
        Ok(report)
    }

    /// The bank checks on the document's two banks, as one line per failed check (none when the
    /// document lacks a bank). They are the river stage's checks ([`super::river::river`]): the
    /// import keeps no copy. With `report_crossing`, banks that cross (the unchecked import keeps
    /// them) are reported too, with the first crossing point. When the checks cannot run on two
    /// banks, that is a warning too, with the reason: it never reads as "no warnings".
    fn bank_check_warnings(&self, report_crossing: bool) -> Vec<String> {
        let input = self.bank_check_input();
        let two_banks = [&input.left_bank, &input.right_bank]
            .iter()
            .all(|b| b.as_ref().is_some_and(|l| l.len() >= 2));
        match river(&input) {
            Ok(g) => g.warnings(report_crossing),
            // With one bank there is nothing to check yet: the other comes in a later import.
            Err(_) if !two_banks => Vec::new(),
            Err(e) => vec![format!("the bank checks could not run: {e}")],
        }
    }

    /// What the bank checks read: the river stage's input without the polygon. The polygon is not
    /// under test, and a bad one must not hide a bank warning. The import's warnings and
    /// `owd check-banks` both use this, so they cannot differ.
    pub fn bank_check_input(&self) -> RiverIn {
        RiverIn {
            river_polygon: None,
            ..self.river_input()
        }
    }

    /// The bank checks of the document: the river stage on [`Session::bank_check_input`].
    pub fn bank_check_geometry(&self) -> Result<RiverGeometry, StageError> {
        river(&self.bank_check_input())
    }

    /// What the river stage reads, taken from the document.
    pub fn river_input(&self) -> RiverIn {
        let inputs = &self.doc["inputs"];
        let line = |role: Role| line_points(&inputs[role.name()]).map(Polyline);
        RiverIn {
            left_bank: line(Role::LeftBank),
            right_bank: line(Role::RightBank),
            centreline: line(Role::Centreline),
            river_polygon: RiverPolygon::from_geometry(&inputs[Role::RiverPolygon.name()]),
            crs_epsg: self.doc["project"]["crs"]["epsg"]
                .as_u64()
                .map(|c| c as u32),
            // The stored lines already run from the mouth when the design states one, so the river
            // stage finds nothing to turn; it still names the banks looking downstream.
            mouth: self
                .stored_mouth()
                .map(|m| super::river::Mouth::Point(m.point)),
            water_profile: self.input_water_profile(),
            end_levels_m: None,
        }
    }

    /// The river stage on the document's inputs: the geometry, with the bank checks.
    pub fn river_geometry(&self) -> Result<RiverGeometry, StageError> {
        river(&self.river_input())
    }

    /// The banks after an import of `converted` (in the project CRS), as a sound pair; `None` when
    /// the import holds no bank. Refuses two banks that cross, and a downstream point that
    /// disagrees with the centreline. Changes nothing.
    fn sound_pair(
        &self,
        converted: &[(Role, Crs, Value)],
        downstream: Option<[f64; 2]>,
        refuse_crossing: bool,
    ) -> Result<Option<SoundPair>, DesignError> {
        use super::bank_pair::{first_crossing, on_the_right_of, runs_against, runs_away_from};
        if !converted.iter().any(|(r, _, _)| r.is_bank()) {
            return Ok(None);
        }
        let imported = |role: Role| converted.iter().find(|(r, _, _)| *r == role);
        let after = |role: Role| -> Value {
            imported(role)
                .map(|(_, _, g)| g.clone())
                .unwrap_or_else(|| self.doc["inputs"][role.name()].clone())
        };
        let (mut left, mut right) = (after(Role::LeftBank), after(Role::RightBank));
        let centre = line_points(&after(Role::Centreline));
        if refuse_crossing
            && let (Some(l), Some(r)) = (line_points(&left), line_points(&right))
            && let Some([x, y]) = first_crossing(&l, &r)
        {
            return Err(DesignError::Input(format!(
                "the left bank and the right bank cross at ({x:.1}, {y:.1}): a river's banks \
                 must not cross"
            )));
        }
        if let (Some(c), Some(d)) = (&centre, downstream)
            && runs_away_from(c, d)
        {
            return Err(DesignError::Input(
                "the downstream point is at the upstream end of the centreline: the centreline \
                 runs downstream, so the two disagree"
                    .into(),
            ));
        }
        // Direction.
        let turn_if = |v: &mut Value, yes: &dyn Fn(&[[f64; 2]]) -> bool| {
            if line_points(v).is_some_and(|p| yes(&p)) {
                reverse_line(v);
            }
        };
        if let Some(c) = &centre {
            for v in [&mut left, &mut right] {
                turn_if(v, &|p| runs_against(p, c));
            }
        } else if let Some(d) = downstream {
            for v in [&mut left, &mut right] {
                turn_if(v, &|p| runs_away_from(p, d));
            }
        } else if let (Some(l), Some(r)) = (line_points(&left), line_points(&right)) {
            // The bank of this import turns to match the other; with both, the right bank turns.
            if imported(Role::RightBank).is_some() {
                turn_if(&mut right, &|p| runs_against(p, &l));
            } else {
                turn_if(&mut left, &|p| runs_against(p, &r));
            }
        }
        // Sides, looking downstream.
        let mut swapped = false;
        if centre.is_some() || downstream.is_some() {
            let wrong = match (line_points(&left), line_points(&right), &centre) {
                (Some(l), Some(r), _) => on_the_right_of(&l, &r),
                (Some(l), None, Some(c)) => on_the_right_of(&l, c),
                (None, Some(r), Some(c)) => on_the_right_of(c, &r),
                _ => false,
            };
            if wrong {
                std::mem::swap(&mut left, &mut right);
                swapped = true;
            }
        }
        // The river polygon: built from the banks unless one was given.
        let mut polygon = None;
        if imported(Role::RiverPolygon).is_none()
            && let (Some(l), Some(r)) = (line_points(&left), line_points(&right))
        {
            let old = &self.doc["inputs"];
            let built_before = match (
                line_points(&old[Role::LeftBank.name()]),
                line_points(&old[Role::RightBank.name()]),
            ) {
                (Some(ol), Some(or)) => {
                    ring_points(&old[Role::RiverPolygon.name()])
                        == Some(super::bank_edit::polygon(&ol, &or))
                }
                _ => false,
            };
            if old[Role::RiverPolygon.name()].is_null() || built_before {
                polygon = Some(json!({
                    "type": "Polygon",
                    "coordinates": [super::bank_edit::polygon(&l, &r)],
                }));
            }
        }
        Ok(Some(SoundPair {
            left,
            right,
            swapped,
            polygon,
        }))
    }
}

/// The banks after an import, as [`Session::sound_pair`] makes them.
struct SoundPair {
    left: Value,
    right: Value,
    /// The names were the wrong way round, so the two lines changed places.
    swapped: bool,
    /// The river polygon built from the banks, when it is to be (re)built.
    polygon: Option<Value>,
}

/// The positions of a LineString geometry; `None` for anything else.
fn line_points(v: &Value) -> Option<Vec<[f64; 2]>> {
    if v["type"] != "LineString" {
        return None;
    }
    v["coordinates"]
        .as_array()?
        .iter()
        .map(|p| Some([p[0].as_f64()?, p[1].as_f64()?]))
        .collect()
}

/// The exterior ring of a Polygon geometry; `None` for anything else.
fn ring_points(v: &Value) -> Option<Vec<[f64; 2]>> {
    if v["type"] != "Polygon" {
        return None;
    }
    v["coordinates"][0]
        .as_array()?
        .iter()
        .map(|p| Some([p[0].as_f64()?, p[1].as_f64()?]))
        .collect()
}

/// Turn a LineString geometry round.
fn reverse_line(v: &mut Value) {
    if let Some(a) = v["coordinates"].as_array_mut() {
        a.reverse();
    }
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;

    #[test]
    fn dates_come_from_unix_seconds() {
        assert_eq!(date_from_unix(0), "1970-01-01");
        assert_eq!(date_from_unix(951_782_400), "2000-02-29"); // a leap day
        assert_eq!(date_from_unix(1_792_000_000), "2026-10-14");
        assert_eq!(date_from_unix(4_107_542_400), "2100-03-01"); // 2100 is not a leap year
    }

    #[test]
    fn a_cdesign_file_name_is_refused_with_a_message_to_rename_it_to_owd() {
        for name in ["x.cdesign", "dir/river.CDesign"] {
            let e = check_file_name(name).unwrap_err().to_string();
            assert!(e.starts_with(name), "{e}");
            assert!(e.contains("rename it to `.owd`"), "{e}");
        }
        for name in ["x.owd", "-", "cdesign", "x.cdesign.owd", "notes.txt"] {
            assert!(check_file_name(name).is_ok(), "{name}");
        }
    }

    fn with_sizing_fresh() -> Session {
        let mut s = Session::new("t", "2026-10-16");
        s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
            .unwrap();
        s.set_vessel("CEMT_IV", None).unwrap();
        s.run_stage(Stage::Sizing).unwrap();
        s
    }

    /// Give `stage` results that match what it depends on now, as its own ticket will.
    fn fake_run(s: &mut Session, stage: Stage) {
        s.doc["stages"][stage.name()]["settings"] = json!({});
        let h = s.dependency_hash(stage);
        let st = &mut s.doc["stages"][stage.name()];
        st["results"] = json!({"n": 1});
        st["depends_on"] = json!(h);
        st["engine_version"] = json!(ENGINE_VERSION);
    }

    #[test]
    fn an_edit_marks_exactly_the_stages_that_depend_on_it_stale() {
        let mut s = with_sizing_fresh();
        s.import(&[(
            Role::LeftBank,
            r#"{"type":"LineString","crs":{"type":"name","properties":{"name":"EPSG:32630"}},
                "coordinates":[[500000,100],[500100,100]]}"#,
        )])
        .unwrap();
        // Alignment ran after the import and the sizing.
        s.run_stage(Stage::Sizing).unwrap();
        fake_run(&mut s, Stage::Alignment);
        fake_run(&mut s, Stage::Optimise);
        let fresh = |s: &Session| Stage::ALL.map(|st| s.stage_status(st));
        use StageStatus::*;
        assert_eq!(fresh(&s), [Fresh, Fresh, Fresh, NotRun]);
        // A new vessel: sizing and everything after it that has results is stale. Nothing reruns.
        s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
            .unwrap();
        s.set_vessel("CEMT_Va", None).unwrap();
        assert_eq!(fresh(&s), [Stale, Stale, Stale, NotRun]);
        // Running sizing again changes its results, so the later stages stay stale.
        s.run_stage(Stage::Sizing).unwrap();
        assert_eq!(fresh(&s), [Fresh, Stale, Stale, NotRun]);
    }

    #[test]
    fn a_new_input_marks_alignment_stale_but_not_sizing() {
        let mut s = with_sizing_fresh();
        let line = |y: u32| {
            format!(
                r#"{{"type":"LineString","crs":{{"type":"name","properties":{{"name":"EPSG:32630"}}}},
                "coordinates":[[500000,{y}],[500100,{y}]]}}"#
            )
        };
        s.import(&[(Role::LeftBank, &line(100))]).unwrap();
        fake_run(&mut s, Stage::Alignment);
        assert_eq!(s.stage_status(Stage::Alignment), StageStatus::Fresh);
        s.import(&[(Role::LeftBank, &line(120))]).unwrap();
        assert_eq!(s.stage_status(Stage::Sizing), StageStatus::Fresh);
        assert_eq!(s.stage_status(Stage::Alignment), StageStatus::Stale);
    }

    #[test]
    fn a_stage_whose_inputs_are_stale_refuses_to_run_and_names_the_stage_to_run_first() {
        let mut s = with_sizing_fresh();
        s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
            .unwrap();
        s.set_vessel("CEMT_Va", None).unwrap();
        let e = s.run_stage(Stage::Alignment).unwrap_err();
        assert_eq!(
            e,
            DesignError::StaleUpstream {
                stage: Stage::Alignment,
                run_first: Stage::Sizing
            }
        );
        assert_eq!(
            e.to_string(),
            "the alignment stage cannot run: sizing is stale. Run sizing first."
        );
    }

    #[test]
    fn results_without_a_hash_make_the_document_invalid() {
        let mut s = with_sizing_fresh();
        s.doc["stages"]["alignment"]["results"] = json!({"n": 1});
        let e = s.validate();
        assert!(
            e.iter()
                .any(|m| m.contains("alignment") && m.contains("depends_on")),
            "{e:?}"
        );
    }
}
