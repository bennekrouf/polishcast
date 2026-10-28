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
fn the_desktop_entry_starts_this_app_in_mcp_mode() {
    let config: Value =
        serde_json::from_str(&desktop_config("/Applications/PolishCast.app/Contents/MacOS/polishcast")).unwrap();
    assert_eq!(config["mcpServers"]["polishcast"]["command"], "/Applications/PolishCast.app/Contents/MacOS/polishcast");
    assert_eq!(config["mcpServers"]["polishcast"]["args"], json!(["mcp"]));
    let path = r"C:\Program Files\PolishCast\polishcast.exe";
    let config: Value = serde_json::from_str(&desktop_config(path)).unwrap();
    assert_eq!(config["mcpServers"]["polishcast"]["command"], path);
    assert_eq!(code_command("/A B/polishcast"), "claude mcp add polishcast --scope user -- \"/A B/polishcast\" mcp");
}

#[test]
fn a_take_must_exist_with_its_project() {
    assert!(Take::from(&json!({})).is_err());
    assert!(Take::from(&json!({ "take": "/nowhere/2026-01-01 10.00.00" })).is_err());
}

#[test]
fn the_codex_command_quotes_a_path_with_spaces() {
    assert_eq!(codex_command("/A B/polishcast"), "codex mcp add polishcast -- \"/A B/polishcast\" mcp");
}
