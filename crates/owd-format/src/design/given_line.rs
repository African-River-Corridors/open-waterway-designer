//! A given design line (ticket C21): a line the user already has, from an earlier study or drawn
//! by hand, given as the design line with the method `given`. It gets the radius check every design
//! line gets; a line under the minimum radius is reported, not refused. The volumes stage, the
//! sections and the exports then read it as they read any design line.

use super::alignment::{AlignMethod, pts_of};
use super::crs::Crs;
use super::design_line::{GivenLine, given_id};
use super::geojson::{self, Role};
use super::session::{DesignError, Session, Stage, StageStatus};
use crate::types::Polyline;

/// The method of a given line, as `stages.alignment.settings.method` writes it.
pub const GIVEN: &str = "given";

impl Session {
    /// Why a given line cannot be checked now, if it cannot: the sizing must be fresh.
    fn sizing_ready_for_given(&self) -> Result<(), DesignError> {
        match self.stage_status(Stage::Sizing) {
            StageStatus::Fresh => Ok(()),
            StageStatus::NotRun => Err(DesignError::Input(
                "a given design line is checked against the minimum radius of the sizing: \
                 choose a vessel class and size the channel first (`owd size`)"
                    .into(),
            )),
            StageStatus::Stale => Err(DesignError::Input(
                "the sizing is stale: run `owd size` (or `owd run`) before giving a design line"
                    .into(),
            )),
        }
    }

    /// Make a line from GeoJSON text the design line, with the method `given`: it replaces the
    /// design line any method made before. Returns the line's id.
    pub fn import_design_line(&mut self, geojson: &str, name: &str) -> Result<String, DesignError> {
        let name = name.trim();
        let id = given_id(name).ok_or_else(|| {
            DesignError::Input("a given design line needs a name with a letter or a digit".into())
        })?;
        let parsed = geojson::parse(Role::Centreline, geojson)
            .map_err(|e| DesignError::Input(format!("the design line: {e}")))?;
        self.sizing_ready_for_given()?;
        let utm = match self.doc["project"]["crs"]["epsg"]
            .as_u64()
            .and_then(|c| Crs::from_epsg(c as u32).ok())
        {
            Some(Crs::Utm(u)) => u,
            _ => {
                return Err(DesignError::Input(
                    "the design has no project CRS: import the river first (`owd import`)".into(),
                ));
            }
        };
        let geometry = geojson::to_project_crs(&parsed, utm)
            .map_err(|e| DesignError::Input(format!("the design line: {e}")))?;
        let line = pts_of(&geometry).filter(|l| l.len() >= 2).ok_or_else(|| {
            DesignError::Input("the design line needs at least two points".into())
        })?;
        let mut settings = self.alignment_settings()?;
        settings.method = AlignMethod::Given;
        settings.given = Some(GivenLine {
            name: name.to_string(),
            line: Polyline(line),
        });
        let input = self.alignment_input(settings.clone())?;
        let design = self.run_alignment(&input, None)?;
        self.store_design_line(&settings, AlignMethod::Given, &design);
        Ok(id)
    }
}
