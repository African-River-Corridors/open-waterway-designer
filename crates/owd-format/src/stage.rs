//! How a stage is stored, and how its schema is kept honest (spec 0017, ADR 0015).
//!
//! A stage is stored as a [`StageRecord`]: the contract `version`, the hash of what it was made from
//! (`depends_on`), its `settings` and its `output`. The record is generic over the settings and the
//! output, so a stage with typed contracts gets a typed record, and the runner can read any stage as
//! `StageRecord<Value, Value>`.
//!
//! The JSON Schema of each stage contract is generated from its Rust type and checked in under
//! `schema/stages/<stage>.<version>.schema.json`. [`check_dir`] compares the folder with
//! [`contracts`]; the test `stage_schemas` runs it, so a change to a type without the matching file
//! fails the build. A change that alters what an output means raises the version, which is a new file.

use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

/// The contract version every stage is at today. A stage's own version rises with the ticket that
/// changes what its output means.
pub const STAGE_CONTRACT_VERSION: u32 = 1;

/// One stage as the document stores it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StageRecord<S, O> {
    /// The version of the stage's contract: which schema its settings and output follow.
    pub version: u32,
    /// The hash of the input the stage read, so staleness can be worked out on load. Absent until
    /// the stage has run.
    pub depends_on: Option<String>,
    /// The settings the stage ran with (or will run with).
    pub settings: S,
    /// What the stage returned. Absent until the stage has run.
    pub output: Option<O>,
}

/// A stage contract that has a schema file: the stage's name, its version and a function that makes
/// the schema.
#[derive(Clone, Copy)]
pub struct StageContract {
    /// The stage's name, as the document stores it.
    pub stage: &'static str,
    /// The contract version.
    pub version: u32,
    schema: fn() -> Value,
}

fn schema_of<T: JsonSchema>() -> Value {
    serde_json::to_value(schema_for!(T)).expect("a schema is JSON")
}

impl StageContract {
    /// The contract of `stage` at `version` whose schema is the one generated for `T`.
    pub fn of<T: JsonSchema>(stage: &'static str, version: u32) -> Self {
        Self {
            stage,
            version,
            schema: schema_of::<T>,
        }
    }

    /// The schema, as the generator makes it now.
    pub fn schema(&self) -> Value {
        (self.schema)()
    }

    /// The name of the schema file: `<stage>.<version>.schema.json`.
    pub fn file_name(&self) -> String {
        file_name(self.stage, self.version)
    }

    /// The text of the schema file: pretty JSON with a final newline.
    pub fn text(&self) -> String {
        let mut t = serde_json::to_string_pretty(&self.schema()).expect("a schema is JSON");
        t.push('\n');
        t
    }
}

/// The name of the schema file of `stage` at `version`.
pub fn file_name(stage: &str, version: u32) -> String {
    format!("{stage}.{version}.schema.json")
}

/// Every stage contract with a schema file. A stage joins this list in the ticket that gives it typed
/// input and output.
pub fn contracts() -> Vec<StageContract> {
    use crate::design::design_line::{self, AlignmentIn, DesignLine};
    use crate::design::envelope::{self, EnvelopeIn, EnvelopeResult};
    use crate::design::reference_line::{self, ReferenceLine, ReferenceLineIn};
    use crate::design::river::{self, RiverGeometry, RiverIn};
    use crate::design::sizing::{self, ChannelSize, SizingIn};
    vec![
        StageContract::of::<StageRecord<Value, Value>>("stage-record", STAGE_CONTRACT_VERSION),
        StageContract::of::<StageRecord<RiverIn, RiverGeometry>>(river::STAGE, 1),
        StageContract::of::<StageRecord<SizingIn, ChannelSize>>(sizing::STAGE, 1),
        StageContract::of::<StageRecord<AlignmentIn, DesignLine>>(design_line::STAGE, 1),
        StageContract::of::<StageRecord<ReferenceLineIn, ReferenceLine>>(reference_line::STAGE, 1),
        StageContract::of::<StageRecord<EnvelopeIn, EnvelopeResult>>(envelope::STAGE, 1),
    ]
}

/// Write the schema file of each contract into `dir`, making the folder if it is missing. A file
/// that no contract owns is removed, so the folder holds exactly the contracts' files.
pub fn write_dir(dir: &Path, contracts: &[StageContract]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for c in contracts {
        std::fs::write(dir.join(c.file_name()), c.text())?;
    }
    for name in schema_files(dir)? {
        if !contracts.iter().any(|c| c.file_name() == name) {
            std::fs::remove_file(dir.join(name))?;
        }
    }
    Ok(())
}

fn schema_files(dir: &Path) -> std::io::Result<Vec<String>> {
    let mut names = Vec::new();
    if dir.is_dir() {
        for e in std::fs::read_dir(dir)? {
            let name = e?.file_name().to_string_lossy().into_owned();
            if name.ends_with(".schema.json") {
                names.push(name);
            }
        }
    }
    names.sort();
    Ok(names)
}

/// Every way `dir` differs from the contracts, one line each. Empty when the folder holds exactly
/// the contracts' schemas, byte for byte.
pub fn check_dir(dir: &Path, contracts: &[StageContract]) -> Vec<String> {
    let mut problems = Vec::new();
    for c in contracts {
        let name = c.file_name();
        match std::fs::read_to_string(dir.join(&name)) {
            Err(_) => problems.push(format!("{name}: missing")),
            Ok(on_disk) if on_disk != c.text() => {
                problems.push(format!(
                    "{name}: differs from the schema its type generates"
                ));
            }
            Ok(_) => {}
        }
    }
    match schema_files(dir) {
        Ok(names) => {
            for name in names {
                if !contracts.iter().any(|c| c.file_name() == name) {
                    problems.push(format!("{name}: no contract makes this file"));
                }
            }
        }
        Err(e) => problems.push(format!("{}: cannot be read ({e})", dir.display())),
    }
    problems
}
