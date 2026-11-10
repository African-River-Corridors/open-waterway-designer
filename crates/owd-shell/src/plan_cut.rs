//! The plan cut in the app shell (ticket C2b): run on a copy of the design, as the sweep is.

use crate::state::AppState;
use serde_json::Value;

impl AppState {
    /// Run the plan cut of every design line option with `settings` over the document's. A
    /// refusal leaves the design as it was.
    pub fn run_plan_cut(&mut self, settings: &Value) -> Result<(), String> {
        self.refuse_while_running()?;
        let mut s = self.session().clone();
        s.run_plan_cut(settings).map_err(|e| e.to_string())?;
        self.replace_session(s);
        self.set_dirty();
        Ok(())
    }

    /// The cut polygons in longitude and latitude: the text of `owd plan-cut --geojson FILE --wgs84`.
    pub fn plan_cut_geojson(&self) -> Result<String, String> {
        self.session()
            .export_plan_cut_geojson(true)
            .map_err(|e| e.to_string())
    }
}
