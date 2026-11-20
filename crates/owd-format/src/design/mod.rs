//! The channel design document and the design session (spec 0002, ADR 0004).

pub mod alignment;
pub mod bank_edit;
pub mod bank_pair;
pub mod banks;
pub mod bed;
pub mod crs;
pub mod dem_source;
pub mod design_line;
pub mod envelope;
pub mod export;
pub mod fits;
pub mod geojson;
pub mod geopackage;
pub mod given_line;
pub mod hash;
pub mod hillshade;
pub mod map;
pub mod midline;
pub mod migrate;
pub mod mouth;
pub mod optimise;
pub mod plan_cut;
pub mod provenance;
pub mod reach;
pub mod reference_line;
pub mod river;
pub mod rule_sets;
pub mod schema;
pub mod screening;
pub mod sections;
pub mod session;
pub mod sizing;
pub mod stage_terrain;
pub mod table;
pub mod terrain;
pub mod volumes;
pub mod water;
pub mod water_stage;

pub use alignment::{
    AlignMethod, AlignmentSettings, CentrelineSettings, RerouteSettings, RollingCircleSettings,
};
pub use bed::DEPTH_FROM_DEM_LABEL;
pub use design_line::{DesignLine, design_lines_of, final_line_of, volume_rows_of};
pub use geojson::Role;
pub use mouth::{MouthAt, MouthReport, parse_mouth};
pub use optimise::OptimiseSettings;
pub use session::{
    DesignError, ImportReport, Session, Stage, StageStatus, Status, check_file_name, date_from_unix,
};
pub use stage_terrain::{NoTerrain, stage_terrain};
pub use volumes::{DemCheck, VolumesSettings};
pub use water::WaterSampling;
