//! A given design line in the app shell (ticket C21): added on a copy of the design.

use crate::state::AppState;

impl AppState {
    /// Add a given design line (GeoJSON text, already read by the shell). Returns the option's id.
    /// A refusal changes nothing.
    pub fn load_design_line(&mut self, geojson: &str, name: &str) -> Result<String, String> {
        self.refuse_while_running()?;
        let mut s = self.session().clone();
        let id = s
            .import_design_line(geojson, name)
            .map_err(|e| e.to_string())?;
        self.replace_session(s);
        self.set_dirty();
        Ok(id)
    }
}
