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

#[test]
fn masks_are_added_changed_and_removed_and_a_bad_one_saves_nothing() {
    let dir = std::env::temp_dir().join(format!("polishcast-masks-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(take::SCREEN), b"").unwrap();
    let project = dir.join(take::PROJECT);
    Project::new(10.0, 1920, 1080, &EventLog::default()).save(&project).unwrap();
    let take = dir.to_string_lossy();
    let area = json!({ "start": 1.0, "end": 3.0, "x": 0.1, "y": 0.2, "w": 0.3, "h": 0.1 });

    let got = set_masks(&json!({ "take": take, "add": [area] })).unwrap();
    assert_eq!(got["masks"][0]["cover"], "solid", "solid unless asked");
    let got = set_masks(&json!({ "take": take, "update": [{ "mask": 1, "cover": "blur", "end": 4.0 }] })).unwrap();
    assert_eq!((got["masks"][0]["cover"].as_str(), got["masks"][0]["end"].as_f64()), (Some("blur"), Some(4.0)));

    // An area missing a side, or running backwards, is refused rather than guessed.
    assert!(set_masks(&json!({ "take": take, "add": [{ "start": 1.0, "end": 2.0, "x": 0.1 }] })).is_err());
    let backwards = json!({ "start": 5.0, "end": 4.0, "x": 0.1, "y": 0.1, "w": 0.1, "h": 0.1 });
    assert!(set_masks(&json!({ "take": take, "remove": [1], "add": [backwards] })).is_err());
    assert_eq!(Project::load(&project).unwrap().masks.len(), 1, "the removal wasn't saved either");

    let got = set_masks(&json!({ "take": take, "remove": [1] })).unwrap();
    assert_eq!(got["masks"].as_array().unwrap().len(), 0);
    std::fs::remove_dir_all(&dir).unwrap();
}
