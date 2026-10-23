//! Kdenlive action discovery shared by every assignment surface.
use std::path::Path;
use std::process::Command;
use serde_json::Value;

pub(crate) fn parse_actions(value: &Value, into: &mut Vec<(String, String)>) {
    for action in value.get("actions").and_then(Value::as_array).into_iter().flatten() {
        let id = action.get("id").and_then(Value::as_str).unwrap_or("");
        if !id.is_empty() && !into.iter().any(|(known, _)| known == id) {
            let text = action.get("text").and_then(Value::as_str).unwrap_or(id).replace('&', "");
            into.push((id.to_string(), if text.is_empty() { id.to_string() } else { text }));
        }
    }
}

/// Read-only plugin query: no input capture or editing lease.
pub fn running_kdenlive_actions(daemon: Option<&Path>) -> Vec<(String, String)> {
    let Some(daemon) = daemon else { return Vec::new() };
    let Ok(out) = Command::new(daemon).args(["list-capabilities", "--json"]).output() else { return Vec::new() };
    let Ok(value) = serde_json::from_slice::<Value>(&out.stdout) else { return Vec::new() };
    let mut actions = Vec::new();
    for service in value.get("kdenlive").and_then(Value::as_array).into_iter().flatten() {
        parse_actions(service, &mut actions);
    }
    actions
}

pub fn kdenlive_catalog(daemon: Option<&Path>) -> Vec<(String, String)> {
    let mut actions = Vec::new();
    if let Some(out) = daemon.and_then(|d| Command::new(d).args(["list-actions", "--json"]).output().ok()) {
        if let Ok(value) = serde_json::from_slice::<Value>(&out.stdout) {
            parse_actions(&value, &mut actions);
        }
    }
    actions
}

pub fn load(daemon: Option<&Path>) -> Vec<(String, String)> {
    let live = running_kdenlive_actions(daemon);
    if live.is_empty() { kdenlive_catalog(daemon) } else { live }
}
