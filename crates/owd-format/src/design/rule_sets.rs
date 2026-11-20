//! The Rule Set selection of a channel design (spec 0007, ADR 0008): which sets size the channel, with
//! which inputs, and whether the user chose each (`typed`) or took the tool's choice (`default`).
//!
//! The sizing settings store each selected set's id, version and content hash. The engine's library
//! (the built-in sets, plus any set file the caller loads at run time) gives the sets themselves. A
//! stored hash that the library does not hold is refused, never silently re-evaluated.

use super::session::{DesignError, Session};
use super::sizing::{self, ChannelSize, SizingIn};
use crate::types::StageError;
use arc_rules::load::Library;
use owd_core::rules::custom::{CustomVessel, Formation};
use owd_core::rules::engine::{self, ChannelDomain, Selection};
use owd_core::rules::rulesets::ChannelShape;
use serde_json::{Value, json};

/// The set a new design selects, with its inputs, all marked `default`.
pub const DEFAULT_SETS: &[&str] = &["pianc-wg141"];
/// The inputs of the default selection.
pub const DEFAULT_INPUTS: &[(&str, &str)] = &[(engine::SE_QUALITY, "C")];

/// The inputs a user types for a set. The vessel gives `formation` and `vessel_class`.
const SET_INPUTS: &[&str] = &[engine::SE_QUALITY, engine::RANGE_END, engine::GB_CLASS];
/// The set inputs a selected set may go without: `gb_class` comes from a Chinese vessel, and a
/// vessel with none leaves the GB width and depth Rules not evaluated. Not offered in the picker.
const OPTIONAL_INPUTS: &[&str] = &[engine::GB_CLASS];

/// A set input's name, its words, and the words of each value.
type InputWords = (
    &'static str,
    &'static str,
    &'static [(&'static str, &'static str)],
);

/// What each set input means, for a picker: its words, and the words of each value.
const INPUT_WORDS: &[InputWords] = &[
    (
        engine::SE_QUALITY,
        "Safety and ease quality",
        &[
            ("A", "A: the most room"),
            ("B", "B: between A and C"),
            ("C", "C: the least room"),
        ],
    ),
    (
        engine::RANGE_END,
        "End of each range the standard gives",
        &[
            ("permissive", "permissive: the less demanding end"),
            ("strict", "strict: the more demanding end"),
        ],
    ),
];

/// Whether a set has a Rule that applies on input `name`.
fn set_reads(set: &arc_rules::model::RuleSet, name: &str) -> bool {
    set.rules
        .iter()
        .any(|r| r.applies_when.conditions.iter().any(|(k, _)| k == name))
}

/// The built-in Rule Sets a user can pick, each with the set inputs its Rules apply on (values,
/// words and the default, if there is one), and the sets that are always selected.
pub fn rule_set_choices() -> (Vec<Value>, Vec<Value>) {
    rule_set_choices_in(engine::built_in())
}

/// The Rule Sets of `lib` a user can pick, as [`rule_set_choices`] lists them, and the sets that
/// are always selected (ticket #499). The built-in sets come first, listed as
/// [`rule_set_choices`] lists them; a set loaded from a file follows, marked `"from_file": true`,
/// its label saying so.
pub fn rule_set_choices_in(lib: &Library) -> (Vec<Value>, Vec<Value>) {
    let built_in = engine::built_in();
    let from_file =
        |set: &arc_rules::model::RuleSet| built_in.find(&set.id, &set.version).is_none();
    let (mut pick, mut always) = (Vec::new(), Vec::new());
    let ordered = lib
        .sets()
        .iter()
        .filter(|s| !from_file(s))
        .chain(lib.sets().iter().filter(|s| from_file(s)));
    for set in ordered {
        let mut entry = json!({
            "id": set.id,
            "version": set.version,
            "title": set.title,
            "label": format!("{} ({}@{})", set.title, set.id, set.version),
        });
        if from_file(set) {
            entry["from_file"] = json!(true);
            entry["label"] = json!(format!(
                "{} ({}@{}, from a file)",
                set.title, set.id, set.version
            ));
        }
        if set.id == engine::OWD_SET && !from_file(set) {
            always.push(entry);
            continue;
        }
        let inputs: Vec<Value> = SET_INPUTS
            .iter()
            .filter(|name| !OPTIONAL_INPUTS.contains(name) && set_reads(set, name))
            .map(|name| {
                let words = INPUT_WORDS.iter().find(|(n, _, _)| n == name);
                let values = engine::INPUTS
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, v)| *v)
                    .unwrap_or_default();
                json!({
                    "name": name,
                    "label": words.map_or(*name, |w| w.1),
                    "values": values.iter().map(|v| json!({
                        "value": v,
                        "label": words
                            .and_then(|w| w.2.iter().find(|(x, _)| x == v))
                            .map_or(*v, |(_, l)| l),
                    })).collect::<Vec<_>>(),
                    "default": DEFAULT_INPUTS.iter().find(|(n, _)| n == name).map(|(_, v)| *v),
                })
            })
            .collect();
        entry["inputs"] = json!(inputs);
        pick.push(entry);
    }
    (pick, always)
}

fn set_entry(library: &Library, id: &str, source: &str) -> Result<Value, DesignError> {
    let set = library
        .lookup(id)
        .ok_or_else(|| DesignError::Input(format!("unknown Rule Set `{id}`; see `owd rules`")))?;
    Ok(json!({"id": set.id, "version": set.version, "hash": set.hash, "source": source}))
}

/// The sizing settings of a new design: the default selection and no vessel yet.
pub(super) fn default_settings() -> Value {
    let lib = engine::built_in();
    let mut sets: Vec<Value> = DEFAULT_SETS
        .iter()
        .map(|id| set_entry(lib, id, "default").expect("a built-in set"))
        .collect();
    sets.push(set_entry(lib, engine::OWD_SET, "default").expect("a built-in set"));
    json!({
        "rule_sets": sets,
        "set_inputs": DEFAULT_INPUTS
            .iter()
            .map(|(k, v)| json!({"name": k, "value": v, "source": "default"}))
            .collect::<Vec<_>>(),
        "vessel_standard_id": null,
        "draught_m": null,
    })
}

/// A custom vessel as the study keeps it (`stages.sizing.settings.custom_vessel`, spec 0015 T1):
/// the optional fields only when given.
pub fn custom_vessel_json(v: &CustomVessel) -> Value {
    let mut o = json!({
        "name": v.name.trim(),
        "loa_m": v.loa_m,
        "beam_m": v.beam_m,
        "draught_m": v.draught_m,
        "formation": v.formation.as_str(),
    });
    for (k, x) in [
        ("n_abreast", v.n_abreast.map(|n| json!(n))),
        ("n_in_line", v.n_in_line.map(|n| json!(n))),
        ("gb_class", v.gb_class.as_ref().map(|c| json!(c))),
        ("air_draught_m", v.air_draught_m.map(|x| json!(x))),
        ("deadweight_t", v.deadweight_t.map(|x| json!(x))),
    ] {
        if let Some(x) = x {
            o[k] = x;
        }
    }
    o
}

/// A custom vessel from the study's JSON; `None` when `v` is not one.
pub fn custom_vessel_of(v: &Value) -> Option<CustomVessel> {
    let num = |k: &str| v[k].as_f64();
    Some(CustomVessel {
        name: v["name"].as_str()?.to_string(),
        loa_m: num("loa_m")?,
        beam_m: num("beam_m")?,
        draught_m: num("draught_m")?,
        formation: Formation::parse(v["formation"].as_str()?).ok()?,
        n_abreast: v["n_abreast"].as_u64().map(|n| n as u32),
        n_in_line: v["n_in_line"].as_u64().map(|n| n as u32),
        gb_class: v["gb_class"].as_str().map(str::to_string),
        air_draught_m: num("air_draught_m"),
        deadweight_t: num("deadweight_t"),
    })
}

/// A stage's refusal as the session reports it: the message, as an input error.
pub(super) fn stage_error(e: StageError) -> DesignError {
    DesignError::Input(e.message)
}

impl Session {
    /// The Rule Sets this session can select: the built-in ones and any loaded at run time.
    pub fn rule_set_library(&self) -> &Library {
        &self.library
    }

    /// Load a Rule Set file at run time, through the same loader as the built-in sets. A file with
    /// the id and version of a loaded set but other content is refused. Returns `id@version`.
    pub fn load_rule_set(&mut self, text: &str) -> Result<String, DesignError> {
        self.library
            .add_text(text, &ChannelDomain)
            .map_err(|e| DesignError::Input(format!("the Rule Set file is refused: {e}")))
    }

    /// Choose the Rule Sets and set inputs, marked `typed`. With no set given the selection keeps
    /// its sets; each input given replaces one of that name. `owd` is always selected. A
    /// `pianc-wg141` with no quality takes quality C, marked `default`. Nothing runs: the sizing
    /// stage goes stale.
    pub fn set_rule_sets(
        &mut self,
        ids: &[&str],
        inputs: &[(&str, &str)],
    ) -> Result<(), DesignError> {
        let mut settings = self.sizing_settings();
        if !ids.is_empty() {
            let mut sets = Vec::new();
            for id in ids {
                let entry = set_entry(&self.library, id, "typed")?;
                if !sets.iter().any(|s: &Value| s["id"] == entry["id"]) {
                    sets.push(entry);
                }
            }
            if !sets.iter().any(|s| s["id"] == engine::OWD_SET) {
                sets.push(set_entry(&self.library, engine::OWD_SET, "default")?);
            }
            settings["rule_sets"] = json!(sets);
            // Inputs of sets no longer selected go; a default input is chosen again below.
            let keep: Vec<Value> = settings["set_inputs"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|i| {
                    i["source"] == "typed" && self.input_is_read(&settings, i["name"].as_str())
                })
                .collect();
            settings["set_inputs"] = json!(keep);
        }
        let mut list = settings["set_inputs"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for (name, value) in inputs {
            if !SET_INPUTS.contains(name) {
                return Err(DesignError::Input(format!(
                    "unknown set input `{name}`; the set inputs are {}",
                    SET_INPUTS.join(", ")
                )));
            }
            let def = engine::INPUTS
                .iter()
                .find(|(n, _)| n == name)
                .expect("listed");
            if !def.1.contains(value) {
                return Err(DesignError::Input(format!(
                    "set input `{name}` is `{value}`; its values are {}",
                    def.1.join(", ")
                )));
            }
            list.retain(|i| i["name"] != *name);
            list.push(json!({"name": name, "value": value, "source": "typed"}));
        }
        settings["set_inputs"] = json!(list);
        self.add_default_inputs(&mut settings);
        self.check_inputs(&settings)?;
        self.doc["stages"]["sizing"]["settings"] = settings;
        Ok(())
    }

    /// Choose the vessel class and draught. Nothing runs.
    pub fn set_vessel(
        &mut self,
        vessel_standard_id: &str,
        draught_m: Option<f64>,
    ) -> Result<(), DesignError> {
        if let Some(d) = draught_m
            && !(d.is_finite() && d > 0.0)
        {
            return Err(DesignError::Input(
                "the draught must be a positive number of metres".into(),
            ));
        }
        // Any class of the shared catalogue a channel can be sized for (spec 0015 T0), saved by its
        // catalogue id (an older id such as `CEMT_Va` gives `CEMT-Va`).
        let class = owd_core::rules::classes::class(vessel_standard_id).map_err(|e| match e {
            owd_core::rules::RuleError::Key(_) => {
                DesignError::Input(format!("unknown vessel standard `{vessel_standard_id}`"))
            }
            e => DesignError::Input(e.raw_message().to_string()),
        })?;
        let mut settings = self.sizing_settings();
        settings["vessel_standard_id"] = json!(class.standard_id);
        settings["draught_m"] = json!(draught_m);
        // A class replaces a custom vessel (spec 0015 T1).
        if let Some(o) = settings.as_object_mut() {
            o.remove("custom_vessel");
        }
        // The shape must size before the choice is kept, so a set that cannot size it is refused here.
        self.shape_for_settings(&settings)?;
        self.doc["stages"]["sizing"]["settings"] = settings;
        Ok(())
    }

    /// Choose a custom vessel (spec 0015 T1): it replaces the class. A vessel that cannot be right
    /// is refused, naming the field; the warnings of an unusual one are returned. Nothing runs.
    pub fn set_custom_vessel(&mut self, v: &CustomVessel) -> Result<Vec<String>, DesignError> {
        let warnings = v
            .check()
            .map_err(|e| DesignError::Input(e.raw_message().to_string()))?;
        let mut settings = self.sizing_settings();
        settings["vessel_standard_id"] = Value::Null;
        settings["draught_m"] = Value::Null;
        settings["custom_vessel"] = custom_vessel_json(v);
        self.shape_for_settings(&settings)?;
        self.doc["stages"]["sizing"]["settings"] = settings;
        Ok(warnings)
    }

    /// The custom vessel of the sizing settings, if there is one.
    pub fn custom_vessel(&self) -> Option<CustomVessel> {
        custom_vessel_of(&self.doc["stages"]["sizing"]["settings"]["custom_vessel"])
    }

    /// What every output calls the vessel: the class id, or a custom vessel's label
    /// (`Custom: My barge, 140 × 29 × 4.7 m`). `None` before a vessel is chosen.
    pub fn vessel_label(&self) -> Option<String> {
        match self.custom_vessel() {
            Some(c) => Some(c.label()),
            None => self.doc["stages"]["sizing"]["settings"]["vessel_standard_id"]
                .as_str()
                .map(str::to_string),
        }
    }

    /// The sizing settings, or the default ones if the document has none.
    pub(super) fn sizing_settings(&self) -> Value {
        let s = &self.doc["stages"]["sizing"]["settings"];
        if s.is_object() {
            s.clone()
        } else {
            default_settings()
        }
    }

    /// Whether a selected set has a Rule that applies on input `name`.
    fn input_is_read(&self, settings: &Value, name: Option<&str>) -> bool {
        let Some(name) = name else { return false };
        selected(settings).iter().any(|(id, version)| {
            self.library.find(id, version).is_some_and(|set| {
                set.rules
                    .iter()
                    .any(|r| r.applies_when.conditions.iter().any(|(k, _)| k == name))
            })
        })
    }

    /// A set input a selected set reads but the user did not type takes its default, marked
    /// `default`; a default input no selected set reads goes.
    fn add_default_inputs(&self, settings: &mut Value) {
        let snapshot = settings.clone();
        let list = settings["set_inputs"].as_array_mut().expect("a list");
        for (name, value) in DEFAULT_INPUTS {
            if !list.iter().any(|i| i["name"] == *name) {
                list.push(json!({"name": name, "value": value, "source": "default"}));
            }
        }
        list.retain(|i| {
            i["source"] == "typed" || self.input_is_read(&snapshot, i["name"].as_str())
        });
    }

    /// Each set input a selected set's Rules apply on has a value.
    fn check_inputs(&self, settings: &Value) -> Result<(), DesignError> {
        for name in SET_INPUTS.iter().filter(|n| !OPTIONAL_INPUTS.contains(n)) {
            if self.input_is_read(settings, Some(name))
                && !settings["set_inputs"]
                    .as_array()
                    .is_some_and(|l| l.iter().any(|i| i["name"] == *name))
            {
                let def = engine::INPUTS
                    .iter()
                    .find(|(n, _)| n == name)
                    .expect("listed");
                return Err(DesignError::Input(format!(
                    "a selected Rule Set applies on `{name}`: choose it with --set-input {name}=<{}>",
                    def.1.join("|")
                )));
            }
        }
        Ok(())
    }

    /// The sizing stage's input, from the document: the sizing settings and the marked reaches.
    pub fn sizing_in(&self) -> SizingIn {
        SizingIn::from_document(&self.sizing_settings(), &self.waterway_reaches())
    }

    /// The engine selection of the sizing settings. A set the library does not hold, or holds with
    /// another content hash, is refused.
    pub fn selection(&self) -> Result<Selection, DesignError> {
        let s = sizing::selection(&self.library, &self.sizing_in()).map_err(stage_error)?;
        // A custom vessel's GB class is the `gb_class` set input (spec 0015 T1).
        Ok(match self.custom_vessel() {
            Some(c) => owd_core::rules::rulesets::custom_selection(&s, &c),
            None => s,
        })
    }

    /// The channel shape of the sizing settings.
    pub(super) fn shape_for_settings(&self, settings: &Value) -> Result<ChannelShape, DesignError> {
        let input = SizingIn::from_document(settings, &self.waterway_reaches());
        sizing::shape(&self.library, &input).map_err(stage_error)
    }

    /// The channel size the sizing stage stored: what every later stage reads. It refuses when sizing
    /// has not been run or is stale, and says to run it. The size carries the engine's shape for the
    /// bend widening at any radius ([`ChannelSize::shape`]).
    pub fn channel_size(&self) -> Result<ChannelSize, DesignError> {
        match self.stage_status(super::session::Stage::Sizing) {
            super::session::StageStatus::NotRun => {
                return Err(DesignError::Input(
                    "the sizing stage has not been run: choose a ruleset and a vessel class first"
                        .into(),
                ));
            }
            super::session::StageStatus::Stale => {
                return Err(DesignError::Input(
                    "the sizing stage is stale: run it again first".into(),
                ));
            }
            super::session::StageStatus::Fresh => {}
        }
        let stored = &self.doc["stages"]["sizing"]["results"];
        let size: ChannelSize = serde_json::from_value(stored.clone()).map_err(|e| {
            DesignError::Input(format!("the stored sizing result cannot be read: {e}"))
        })?;
        Ok(size.with_shape(self.shape_for_settings(&self.sizing_settings())?))
    }

    /// The content hash the library holds for each selected set, or null: part of every stage's
    /// dependency hash, so a changed set makes the stages that used it stale.
    pub(super) fn selected_set_hashes(&self) -> Value {
        let settings = self.sizing_settings();
        json!(
            selected(&settings)
                .iter()
                .map(|(id, version)| json!({
                    "set": format!("{id}@{version}"),
                    "hash": self.library.find(id, version).map(|s| s.hash.clone()),
                }))
                .collect::<Vec<_>>()
        )
    }

    /// As [`Session::selected_set_hashes`], but a set the library does not hold yet (one loaded from a
    /// file after the design opens) takes the hash the design was sized with. For a move from an older
    /// format, which runs before the host loads the files: a file that does not match is refused
    /// later ([`Session::rule_set_problem`]), so nothing is kept fresh in error.
    pub(super) fn set_hashes_as_sized(&self) -> Value {
        let settings = self.sizing_settings();
        let stored = |id: &str, version: &str| {
            settings["rule_sets"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|s| s["id"] == id && s["version"] == version)
                .and_then(|s| s["hash"].as_str().map(str::to_string))
        };
        json!(
            selected(&settings)
                .iter()
                .map(|(id, version)| json!({
                    "set": format!("{id}@{version}"),
                    "hash": self
                        .library
                        .find(id, version)
                        .map(|s| s.hash.clone())
                        .or_else(|| stored(id, version)),
                }))
                .collect::<Vec<_>>()
        )
    }

    /// Why the design's Rule Sets cannot be confirmed now, or `None` (ticket #491). Every surface
    /// asks this before a stage after sizing, so the user is told what to give rather than that
    /// sizing is stale:
    /// - a selected set that is neither built in nor loaded: its id, version and hash, and the
    ///   option and the app control that load its file;
    /// - a set loaded from a file (not built in) whose content hash is not the one the design was
    ///   sized with: the file changed, with both hashes.
    ///
    /// A built-in set whose content changed with the engine is not a problem here: the stages go
    /// stale and rerun, as before.
    pub fn rule_set_problem(&self) -> Option<DesignError> {
        let settings = self.sizing_settings();
        for s in settings["rule_sets"].as_array().into_iter().flatten() {
            let (id, version) = (
                s["id"].as_str().unwrap_or(""),
                s["version"].as_str().unwrap_or(""),
            );
            let stored = s["hash"].as_str();
            match self.library.find(id, version) {
                None => {
                    return Some(DesignError::Input(format!(
                        "the design is sized with Rule Set {id}@{version} (content hash {}), which \
                         is not built in and not loaded: give its file with `--rule-set-file FILE` \
                         (in the app: Load Rule Set file, under Rule Sets)",
                        stored.unwrap_or("not recorded")
                    )));
                }
                Some(set)
                    if engine::built_in().find(id, version).is_none()
                        && stored.is_some_and(|h| h != set.hash) =>
                {
                    return Some(DesignError::Input(format!(
                        "the Rule Set file {id}@{version} has changed since the design was sized: \
                         the design was sized with content hash {}, the loaded file has {}. Give the \
                         file it was sized with, or size it again with this one (`owd size \
                         --rule-set {id}@{version}`)",
                        stored.unwrap_or(""),
                        set.hash
                    )));
                }
                Some(_) => {}
            }
        }
        None
    }
}

/// The `(id, version)` of each stored set, in order.
fn selected(settings: &Value) -> Vec<(String, String)> {
    settings["rule_sets"]
        .as_array()
        .map(|l| {
            l.iter()
                .map(|s| {
                    (
                        s["id"].as_str().unwrap_or("").to_string(),
                        s["version"].as_str().unwrap_or("").to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The reaches of the river as the rules read them, each with its chainage range on the river
/// centreline: the marked reaches in chainage order, with the chainage they leave uncovered as
/// free-flowing reaches with no claim. No reach marked: the whole river is one free-flowing reach.
pub(super) fn reach_ranges(doc: &Value) -> Vec<(f64, f64, engine::WaterwayReach)> {
    let marked = doc["inputs"]["waterway_reaches"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    SizingIn::from_document(&Value::Null, &marked).reach_ranges()
}

impl Session {
    /// Mark a reach of the river, by chainage on its centreline (`end_m` `None`: to the end of the
    /// river), with its waterway kind and the reduced-radius claim. Reaches may not overlap, and the
    /// claim is for a canal reach only. Nothing runs: the sizing stage goes stale.
    pub fn add_waterway_reach(
        &mut self,
        start_m: f64,
        end_m: Option<f64>,
        waterway: &str,
        claim_reduced_radius: bool,
    ) -> Result<(), DesignError> {
        if !(start_m.is_finite() && start_m >= 0.0) {
            return Err(DesignError::Input(
                "a reach starts at a chainage of 0 m or more".into(),
            ));
        }
        if let Some(e) = end_m
            && !(e.is_finite() && e > start_m)
        {
            return Err(DesignError::Input("a reach ends after it starts".into()));
        }
        let kinds = engine::INPUTS
            .iter()
            .find(|(n, _)| *n == engine::WATERWAY)
            .expect("listed")
            .1;
        if !kinds.contains(&waterway) {
            return Err(DesignError::Input(format!(
                "the waterway is `{waterway}`; it must be one of {}",
                kinds.join(", ")
            )));
        }
        if claim_reduced_radius && waterway != "canal" {
            return Err(DesignError::Input(
                "the reduced GB 50139 radius is for canal sections only (WG 141 A.3.2): claim it on \
                 a canal reach"
                    .into(),
            ));
        }
        let mut list = self.doc["inputs"]["waterway_reaches"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let end = end_m.unwrap_or(f64::INFINITY);
        for r in &list {
            let (s, e) = (
                r["start_m"].as_f64().unwrap_or(0.0),
                r["end_m"].as_f64().unwrap_or(f64::INFINITY),
            );
            if start_m < e && s < end {
                return Err(DesignError::Input(format!(
                    "the reach overlaps the one from {s} m; remove that one first"
                )));
            }
        }
        list.push(json!({
            "start_m": start_m,
            "end_m": end_m,
            "waterway": waterway,
            "reduced_radius_claim": if claim_reduced_radius { "yes" } else { "no" },
        }));
        list.sort_by(|a, b| {
            a["start_m"]
                .as_f64()
                .partial_cmp(&b["start_m"].as_f64())
                .unwrap_or(core::cmp::Ordering::Equal)
        });
        self.doc["inputs"]["waterway_reaches"] = json!(list);
        Ok(())
    }

    /// The marked reaches, in chainage order.
    pub fn waterway_reaches(&self) -> Vec<Value> {
        self.doc["inputs"]["waterway_reaches"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    /// Remove the marked reach that starts at `start_m`.
    pub fn remove_waterway_reach(&mut self, start_m: f64) -> Result<(), DesignError> {
        let mut list = self.waterway_reaches();
        let before = list.len();
        list.retain(|r| r["start_m"].as_f64() != Some(start_m));
        if list.len() == before {
            return Err(DesignError::Input(format!(
                "no reach starts at {start_m} m; see `owd waterway list`"
            )));
        }
        if list.is_empty() {
            if let Some(o) = self.doc["inputs"].as_object_mut() {
                o.remove("waterway_reaches");
            }
        } else {
            self.doc["inputs"]["waterway_reaches"] = json!(list);
        }
        Ok(())
    }
}
