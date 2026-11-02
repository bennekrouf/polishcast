use super::*;
use std::collections::HashSet;

#[test]
fn every_tool_has_a_unique_name_and_an_object_schema() {
    let tools = tools();
    let tools = tools.as_array().unwrap();
    let names: HashSet<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names.len(), tools.len());
    for t in tools {
        assert_eq!(t["inputSchema"]["type"], "object", "{}", t["name"]);
        for required in t["inputSchema"]["required"].as_array().into_iter().flatten() {
            let key = required.as_str().unwrap();
            assert!(t["inputSchema"]["properties"].get(key).is_some(), "{} requires {key} undescribed", t["name"]);
        }
    }
}

#[test]
fn a_take_must_exist_with_its_project() {
    assert!(Take::from(&json!({})).is_err());
    assert!(Take::from(&json!({ "take": "/nowhere/2026-01-01 10.00.00" })).is_err());
}
