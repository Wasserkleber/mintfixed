//! Version dispatch happens before deserialization: unknown versions must never
//! fall through an untagged legacy parser and then be overwritten on startup.
use super::*;
use serde_json::Value;
use std::{collections::HashSet, io::Write, path::Path};

const VERSION: &str = "0.3.0";

fn invalid(reason: impl Into<String>) -> StateError {
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

fn validate_ids(value: &Value) -> Result<(), StateError> {
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

fn validate_links(data: &ModData_v0_1_0) -> Result<(), StateError> {
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

fn decode(bytes: &[u8]) -> Result<(VersionAnnotatedModData, bool), StateError> {
    let value: Value = serde_json::from_slice(bytes).context(ModDataDeserializationFailedSnafu)?;
    let version = value.get("version");
    let current = version.and_then(Value::as_str) == Some(VERSION);
    let data: ModData_v0_1_0 = match version {
        None | Some(Value::String(_))
            if version.is_none() || version.and_then(Value::as_str) == Some("0.0.0") =>
        {
            let legacy: ModData_v0_0_0 =
                serde_json::from_value(value).context(ModDataDeserializationFailedSnafu)?;
            legacy.into()
        }
        Some(Value::String(v)) if v == "0.1.0" || v == "0.2.0" => {
            serde_json::from_value(value).context(ModDataDeserializationFailedSnafu)?
        }
        Some(Value::String(v)) if v == VERSION => {
            // Serde defaults are for migration only, never repair malformed current data silently.
            validate_ids(&value)?;
            serde_json::from_value(value).context(ModDataDeserializationFailedSnafu)?
        }
        _ => {
            return Err(StateError::UnsupportedModDataVersion {
                version: version.map(ToString::to_string).unwrap_or_default(),
            });
        }
    };
    validate_links(&data)?;
    validate_ids(&serde_json::to_value(&data).context(CfgSerializationFailedSnafu)?)?;
    Ok((VersionAnnotatedModData::V0_3_0(data), !current))
}

/// A fixed, exclusive backup prevents retries from replacing the original.
/// Reuse only a byte-identical backup (e.g. after an interrupted migration).
fn backup(source: &Path, bytes: &[u8]) -> Result<(), StateError> {
    let name = source
        .file_name()
        .ok_or_else(|| invalid("missing source filename"))?;
    let mut backup_name = name.to_os_string();
    backup_name.push(".pre-0.3.0.bak");
    let path = source.with_file_name(backup_name);
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut file) => {
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if fs::read(&path)? != bytes {
                return Err(invalid(format!(
                    "migration backup already exists with different contents: {}",
                    path.display()
                )));
            }
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

pub(super) fn load(path: &Path, legacy_path: &Path) -> Result<VersionAnnotatedModData, StateError> {
    let (source, bytes) = match fs::read(path) {
        Ok(bytes) => (path, bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => match fs::read(legacy_path) {
            Ok(bytes) => (legacy_path, bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(VersionAnnotatedModData::default());
            }
            Err(error) => return Err(error.into()),
        },
        Err(error) => return Err(error.into()),
    };
    let (data, migrated) = decode(&bytes)?;
    if migrated || source != path {
        backup(source, &bytes)?;
        config::write_atomic(path, &data)?;
        // Keep profiles.json intact. Once mod_data.json exists it takes precedence.
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn old(version: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({"version": version, "active_profile":"A",
            "profiles": {
                "A": {"mods":[
                    {"group_name":"Tools","enabled":true,"member_overrides":{"old-member":false}},
                    {"group_name":"Tools","enabled":false},
                    {"spec":{"url":"same"},"required":false,"priority":9},
                    {"spec":{"url":"same"},"required":false,"enabled":false}
                ]},
                "B": {"mods":[{"group_name":"Tools","enabled":true}]}
            }, "groups":{"Tools":{"description":"Tools help","mods":[
                {"spec":{"url":"shared"},"description":"Member help","member_id":"old-member","required":true,"enabled":true,"priority":7},
                {"spec":{"url":"shared"},"required":false,"enabled":false}
            ]}}
        })).unwrap()
    }

    fn strip_ids(value: &mut Value) {
        match value {
            Value::Object(map) => {
                map.remove("id");
                for child in map.values_mut() {
                    strip_ids(child);
                }
            }
            Value::Array(items) => {
                for child in items {
                    strip_ids(child);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn migrate_preserves_order_duplicates_switches_and_reloads_identically() {
        for version in ["0.1.0", "0.2.0"] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("mod_data.json");
            let bytes = old(version);
            fs::write(&path, &bytes).unwrap();
            let data = load(&path, &dir.path().join("profiles.json")).unwrap();
            assert_eq!(
                fs::read(dir.path().join("mod_data.json.pre-0.3.0.bak")).unwrap(),
                bytes
            );
            let original: ModData_v0_1_0 = serde_json::from_slice(&bytes).unwrap();
            let mut expected = serde_json::to_value(original).unwrap();
            let mut actual = serde_json::to_value(data.deref()).unwrap();
            strip_ids(&mut expected);
            strip_ids(&mut actual);
            assert_eq!(actual, expected);
            let saved = fs::read(&path).unwrap();
            assert_eq!(
                serde_json::from_slice::<Value>(&saved).unwrap()["version"],
                VERSION
            );
            let again = load(&path, &dir.path().join("profiles.json")).unwrap();
            assert_eq!(
                serde_json::to_value(&again).unwrap(),
                serde_json::to_value(&data).unwrap()
            );
            assert_eq!(fs::read(&path).unwrap(), saved);
            let mut enabled = Vec::new();
            data.for_each_enabled_mod("A", |m| enabled.push(m.spec.url.clone()));
            assert_eq!(enabled, ["same"]);
        }
    }

    #[test]
    fn legacy_profiles_are_backed_up_and_retained() {
        for tagged in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("mod_data.json");
            let legacy = dir.path().join("profiles.json");
            let mut value = json!({"active_profile":"p", "profiles":{"p":{"mods":[
                {"spec":{"url":"a"}, "required":false, "enabled":false, "priority":4}
            ]}}});
            if tagged {
                value["version"] = json!("0.0.0");
            }
            let bytes = serde_json::to_vec(&value).unwrap();
            fs::write(&legacy, &bytes).unwrap();
            let data = load(&path, &legacy).unwrap();
            assert_eq!(data.profiles["p"].mods.len(), 1);
            assert_eq!(fs::read(&legacy).unwrap(), bytes);
            assert_eq!(
                fs::read(dir.path().join("profiles.json.pre-0.3.0.bak")).unwrap(),
                bytes
            );
            assert!(path.exists());
        }
    }

    #[test]
    fn unsupported_or_malformed_data_is_never_rewritten() {
        let (current, _) = decode(&old("0.1.0")).unwrap();
        let good = serde_json::to_value(current).unwrap();
        let mut missing = good.clone();
        missing["profiles"]["A"]
            .as_object_mut()
            .unwrap()
            .remove("id");
        let mut duplicate = good.clone();
        duplicate["profiles"]["B"]["id"] = duplicate["profiles"]["A"]["id"].clone();
        let mut bad_id = good.clone();
        bad_id["groups"]["Tools"]["id"] = json!("broken");
        let mut dangling = good.clone();
        dangling["groups"] = json!({});
        let mut invalid_version = good;
        invalid_version["version"] = json!(null);
        for bytes in [
            old("99.0.0"),
            b"{broken".to_vec(),
            serde_json::to_vec(&missing).unwrap(),
            serde_json::to_vec(&duplicate).unwrap(),
            serde_json::to_vec(&bad_id).unwrap(),
            serde_json::to_vec(&dangling).unwrap(),
            serde_json::to_vec(&invalid_version).unwrap(),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("mod_data.json");
            fs::write(&path, &bytes).unwrap();
            assert!(load(&path, &dir.path().join("profiles.json")).is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
            assert!(!dir.path().join("mod_data.json.pre-0.3.0.bak").exists());
        }
    }

    #[test]
    fn interrupted_migration_reuses_only_identical_backups() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mod_data.json");
        let bytes = old("0.1.0");
        fs::write(&path, &bytes).unwrap();
        let backup_path = dir.path().join("mod_data.json.pre-0.3.0.bak");
        fs::write(&backup_path, b"an earlier original").unwrap();
        assert!(load(&path, &dir.path().join("profiles.json")).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(fs::read(&backup_path).unwrap(), b"an earlier original");
        fs::write(&backup_path, &bytes).unwrap();
        load(&path, &dir.path().join("profiles.json")).unwrap();
        assert_eq!(fs::read(&backup_path).unwrap(), bytes);
    }

    #[test]
    fn copy_profile_uses_fresh_occurrence_ids_and_retains_shared_switches() {
        let (data, _) = decode(&old("0.1.0")).unwrap();
        let original = &data.profiles["A"];
        let copy = original.duplicate();
        assert_ne!(copy.id, original.id);
        for (a, b) in original.mods.iter().zip(&copy.mods) {
            assert_ne!(a.id(), b.id());
        }
        let mut expected = serde_json::to_value(original).unwrap();
        let mut actual = serde_json::to_value(copy).unwrap();
        strip_ids(&mut expected);
        strip_ids(&mut actual);
        assert_eq!(actual, expected);
    }
    #[test]
    fn failed_backup_or_commit_leaves_original_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join("profiles.json");
        let bytes = old("0.1.0");
        fs::write(&legacy, &bytes).unwrap();
        let backup_path = dir.path().join("profiles.json.pre-0.3.0.bak");
        fs::create_dir(&backup_path).unwrap();
        let path = dir.path().join("mod_data.json");
        assert!(load(&path, &legacy).is_err());
        assert!(!path.exists());
        assert_eq!(fs::read(&legacy).unwrap(), bytes);
        fs::remove_dir(&backup_path).unwrap();
        let bad_target = dir.path().join("missing-directory/mod_data.json");
        assert!(load(&bad_target, &legacy).is_err());
        assert!(!bad_target.exists());
        assert_eq!(fs::read(&legacy).unwrap(), bytes);
        assert_eq!(fs::read(&backup_path).unwrap(), bytes);
    }
    #[test]
    fn first_run_preset_has_valid_persistent_ids_and_is_not_reapplied() {
        let dir = tempfile::tempdir().unwrap();
        let dirs = crate::Dirs::from_path(dir.path()).unwrap();
        let state = State::init(dirs).unwrap();
        let original = serde_json::to_value(state.mod_data.deref()).unwrap();
        assert_eq!(original["version"], VERSION);
        validate_ids(&original).unwrap();
        drop(state);
        let reloaded = State::init(crate::Dirs::from_path(dir.path()).unwrap()).unwrap();
        assert_eq!(
            serde_json::to_value(reloaded.mod_data.deref()).unwrap(),
            original
        );
        assert!(
            !dir.path()
                .join("config/mod_data.json.pre-0.3.0.bak")
                .exists()
        );
    }
}
