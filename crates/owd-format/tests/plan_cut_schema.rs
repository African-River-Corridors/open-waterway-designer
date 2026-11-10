//! The plan cut has a place in the published schema: `stages.plan_cut`, optional.
//!
//! Label: property

use owd_format::design::Session;
use serde_json::Value;

#[test]
fn the_schema_has_an_optional_plan_cut_member_under_stages() {
    let schema: Value = serde_json::from_str(Session::schema_text()).unwrap();
    let stages = &schema["properties"]["stages"];
    assert!(
        stages["properties"]["plan_cut"].is_object(),
        "the schema has no `stages.plan_cut`"
    );
    let required: Vec<&str> = stages["required"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(
        required,
        ["sizing", "alignment", "optimise", "volumes"],
        "the plan cut is optional: a document made before it still opens"
    );
}
