use super::*;
use serde_json::Value;
use std::collections::HashSet;

pub(super) fn invalid(reason: impl Into<String>) -> StateError {
    StateError::InvalidModData {
        reason: reason.into(),
    }
}

fn check_id(value: &Value, seen: &mut HashSet<String>) -> Result<(), StateError> {
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("missing stable ID"))?;
    if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("malformed stable ID"));
    }
    if !seen.insert(id.to_ascii_lowercase()) {
        return Err(invalid("duplicate stable ID"));
    }
    Ok(())
}

pub(super) fn validate_ids(value: &Value) -> Result<(), StateError> {
    let mut seen = HashSet::new();
    for collection in ["profiles", "groups"] {
        let entries = value
            .get(collection)
            .and_then(Value::as_object)
            .ok_or_else(|| invalid(format!("missing {collection}")))?;
        for entry in entries.values() {
            check_id(entry, &mut seen)?;
            for row in entry
                .get("mods")
                .and_then(Value::as_array)
                .ok_or_else(|| invalid("missing ordered mod list"))?
            {
                check_id(row, &mut seen)?;
            }
        }
    }
    if let Some(scripts) = value.get("bitfix_scripts") {
        let scripts = scripts
            .as_array()
            .ok_or_else(|| invalid("invalid global Bitfix scripts"))?;
        for script in scripts {
            check_id(script, &mut seen)?;
        }
    }
    Ok(())
}

pub(super) fn validate_links(data: &ModData_v0_1_0) -> Result<(), StateError> {
    if !data.profiles.contains_key(&data.active_profile) {
        return Err(invalid("active profile does not exist"));
    }
    for profile in data.profiles.values() {
        for row in &profile.mods {
            if let ModOrGroup::Group { group_name, .. } = row
                && !data.groups.contains_key(group_name)
            {
                return Err(invalid(format!("missing shared group: {group_name}")));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "validation/mod_data_tests.rs"]
mod mod_data_tests;

#[cfg(test)]
#[path = "validation/effective_selection_tests.rs"]
mod effective_selection_tests;

#[cfg(test)]
#[path = "validation/state_reliability_tests.rs"]
mod state_reliability_tests;
