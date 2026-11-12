//! Move unused provider metadata out of cache.json without deleting downloaded blobs.
//! Both documents keep the regular versioned cache format.
use std::collections::HashSet;

use serde_json::{Map, Value};

use super::ModSpecification;

// Match the provider URL syntax without modifying modio.rs.
// If a mod.io reference cannot be recognised, refuse to prune that cache.
fn cache_identity(url: &str) -> Option<(&str, Option<u32>, Option<u32>)> {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(
        r"^https://mod\.io/g/drg/m/(?P<name_id>[^/#]+)/?(?:#(?:(?P<mod_id>\d+)(?:/(?P<modfile_id>\d+))?|[a-z]+))?$"
    ).expect("valid mod.io URL regex"));
    let caps = re.captures(url)?;
    let name = caps.name("name_id")?.as_str();
    let mod_id = caps.name("mod_id").and_then(|m| m.as_str().parse().ok());
    let file_id = caps
        .name("modfile_id")
        .and_then(|m| m.as_str().parse().ok());
    Some((name, mod_id, file_id))
}

fn providers(value: &mut Value) -> Result<&mut Map<String, Value>, String> {
    value
        .get_mut("cache")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "expected a versioned cache with a 'cache' object".to_owned())
}

fn field<'a>(
    provider: &'a Map<String, Value>,
    name: &str,
) -> Result<&'a Map<String, Value>, String> {
    provider
        .get(name)
        .and_then(Value::as_object)
        .ok_or_else(|| format!("invalid provider cache field '{name}'"))
}

fn move_unused(
    active: &mut Map<String, Value>,
    archived: &mut Map<String, Value>,
    name: &str,
    keep: impl Fn(&str, &Value) -> bool,
) -> Result<usize, String> {
    let active_entries = active
        .get_mut(name)
        .and_then(Value::as_object_mut)
        .ok_or_else(|| format!("invalid active provider cache field '{name}'"))?;
    let archived_entries = archived
        .entry(name.to_owned())
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| format!("invalid archived provider cache field '{name}'"))?;
    let stale: Vec<String> = active_entries
        .iter()
        .filter_map(|(key, value)| (!keep(key, value)).then_some(key.clone()))
        .collect();
    let count = stale.len();
    for key in stale {
        let value = active_entries.remove(&key).expect("key was present");
        archived_entries.insert(key, value);
    }
    Ok(count)
}

fn archive_provider<'a>(
    archive: &'a mut Map<String, Value>,
    name: &str,
    source: &Map<String, Value>,
    fields: &[&str],
) -> Result<&'a mut Map<String, Value>, String> {
    archive
        .entry(name.to_owned())
        .or_insert_with(|| {
            let mut empty = source.clone();
            for field in fields {
                empty.insert((*field).to_owned(), Value::Object(Map::new()));
            }
            Value::Object(empty)
        })
        .as_object_mut()
        .ok_or_else(|| format!("invalid archived provider '{name}'"))
}

/// Returns the number of archived mod records (mod.io mods plus HTTP URLs).
/// This only mutates the two JSON values supplied; the caller validates and
/// saves the archive before committing the new active cache.
pub(super) fn prune(
    active: &mut Value,
    archive: &mut Value,
    live_specs: &[ModSpecification],
) -> Result<usize, String> {
    let active = providers(active)?;
    let archive = providers(archive)?;
    let mut archived_mods = 0;

    if let Some(provider) = active.get_mut("modio") {
        let provider = provider
            .as_object_mut()
            .ok_or_else(|| "invalid mod.io provider cache".to_owned())?;
        let aliases = field(provider, "mod_id_map")?;
        let mods = field(provider, "mods")?;
        let dependencies = field(provider, "dependencies")?;

        let mut keep_ids: HashSet<String> = HashSet::new();
        let mut keep_files: HashSet<String> = HashSet::new();
        for spec in live_specs {
            let Some((name_id, explicit_id, explicit_file)) = cache_identity(&spec.url) else {
                // An unrecognised mod.io URL is still a profile reference.
                // Refuse to prune rather than accidentally archiving its data.
                if spec.url.starts_with("https://mod.io/g/drg/m/") {
                    return Err(format!(
                        "cannot identify cached mod.io reference: {}",
                        spec.url
                    ));
                }
                continue;
            };
            if let Some(id) = explicit_file {
                keep_files.insert(id.to_string());
            }
            if let Some(id) = explicit_id {
                keep_ids.insert(id.to_string());
            }
            if let Some(id) = aliases.get(name_id).and_then(Value::as_u64) {
                keep_ids.insert(id.to_string());
            }
            // Be conservative with stale name mappings and pinned aliases:
            // keep every cached record whose canonical name is still used.
            for (id, details) in mods {
                if details.get("name_id").and_then(Value::as_str) == Some(name_id) {
                    keep_ids.insert(id.clone());
                }
            }
        }
        // Keep recursively cached dependencies, even when they aren't explicit
        // rows in any profile, so they can still be resolved without extra calls.
        let mut pending: Vec<String> = keep_ids.iter().cloned().collect();
        while let Some(id) = pending.pop() {
            if let Some(entries) = dependencies.get(&id).and_then(Value::as_array) {
                for entry in entries {
                    if let Some(dep) = entry.as_u64().map(|n| n.to_string())
                        && keep_ids.insert(dep.clone())
                    {
                        pending.push(dep);
                    }
                }
            }
        }
        for (id, details) in mods {
            if !keep_ids.contains(id) {
                continue;
            }
            if let Some(file) = details.get("latest_modfile").and_then(Value::as_u64) {
                keep_files.insert(file.to_string());
            }
            for file in details
                .get("modfiles")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(id) = file.get("id").and_then(Value::as_u64) {
                    keep_files.insert(id.to_string());
                }
            }
        }

        // Clear obsolete unavailable warnings alongside unreferenced mod data.
        let mut keep_names = HashSet::new();
        for spec in live_specs {
            if let Some((name, _, _)) = cache_identity(&spec.url) {
                keep_names.insert(name.to_owned());
            }
        }
        for (id, details) in mods {
            if keep_ids.contains(id) {
                if let Some(name) = details.get("name_id").and_then(Value::as_str) {
                    keep_names.insert(name.to_owned());
                }
            }
        }
        let archive = archive_provider(
            archive,
            "modio",
            provider,
            &[
                "mod_id_map",
                "modfile_blobs",
                "dependencies",
                "mods",
                "unavailable_names",
            ],
        )?;
        archived_mods += move_unused(provider, archive, "mods", |id, _| keep_ids.contains(id))?;
        move_unused(provider, archive, "mod_id_map", |_, id| {
            id.as_u64()
                .is_some_and(|id| keep_ids.contains(&id.to_string()))
        })?;
        move_unused(provider, archive, "dependencies", |id, _| {
            keep_ids.contains(id)
        })?;
        move_unused(provider, archive, "modfile_blobs", |file, _| {
            keep_files.contains(file)
        })?;
        // Older caches and test fixtures may not have this field yet.
        if provider.contains_key("unavailable_names") {
            move_unused(provider, archive, "unavailable_names", |name, _| {
                keep_names.contains(name)
            })?;
        }
    }

    if let Some(provider) = active.get_mut("http") {
        let provider = provider
            .as_object_mut()
            .ok_or_else(|| "invalid HTTP provider cache".to_owned())?;
        let live_urls: HashSet<&str> = live_specs.iter().map(|spec| spec.url.as_str()).collect();
        let archive = archive_provider(archive, "http", provider, &["url_blobs"])?;
        archived_mods += move_unused(provider, archive, "url_blobs", |url, _| {
            live_urls.contains(url)
        })?;
    }
    Ok(archived_mods)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec(url: &str) -> ModSpecification {
        ModSpecification::new(url.to_owned())
    }

    #[test]
    fn retains_mods_in_any_profile_or_group_and_their_dependencies() {
        let mut active = json!({
            "version": "0.0.0",
            "cache": {
                "modio": {
                    "type": "ModioCache",
                    "mod_id_map": {"active": 1, "dependency": 2, "old": 3},
                    "mods": {
                        "1": {"name_id": "active", "latest_modfile": 10, "modfiles": [{"id": 10}]},
                        "2": {"name_id": "dependency", "latest_modfile": 20, "modfiles": [{"id": 20}]},
                        "3": {"name_id": "old", "latest_modfile": 30, "modfiles": [{"id": 30}]}
                    },
                    "dependencies": {"1": [2], "2": [], "3": []},
                    "modfile_blobs": {"10": "hash10", "20": "hash20", "30": "hash30"},
                    "unavailable_names": {"active": true, "old": true},
                    "last_update_time": null
                },
                "http": {"type": "HttpProviderCache", "url_blobs": {
                    "https://example.com/active.pak": "hashA",
                    "https://example.com/old.pak": "hashB"
                }}
            }
        });
        let mut archive = json!({"version": "0.0.0", "cache": {}});
        let count = prune(
            &mut active,
            &mut archive,
            &[
                spec("https://mod.io/g/drg/m/active"),
                spec("https://example.com/active.pak"),
            ],
        )
        .unwrap();
        assert_eq!(count, 2);
        assert!(active["cache"]["modio"]["mods"].get("1").is_some());
        assert!(active["cache"]["modio"]["mods"].get("2").is_some());
        assert!(active["cache"]["modio"]["mods"].get("3").is_none());
        assert!(
            active["cache"]["modio"]["modfile_blobs"]
                .get("20")
                .is_some()
        );
        assert!(archive["cache"]["modio"]["mods"].get("3").is_some());
        assert_eq!(
            active["cache"]["modio"]["unavailable_names"]["active"],
            true
        );
        assert!(
            active["cache"]["modio"]["unavailable_names"]
                .get("old")
                .is_none()
        );
        assert_eq!(archive["cache"]["modio"]["unavailable_names"]["old"], true);
        assert!(
            archive["cache"]["http"]["url_blobs"]
                .get("https://example.com/old.pak")
                .is_some()
        );
        assert_eq!(
            prune(
                &mut active,
                &mut archive,
                &[
                    spec("https://mod.io/g/drg/m/active"),
                    spec("https://example.com/active.pak"),
                ]
            )
            .unwrap(),
            0
        );
    }

    #[test]
    fn refuses_to_prune_an_unrecognised_modio_reference() {
        let mut active = json!({"version": "0.0.0", "cache": {"modio": {
            "type": "ModioCache", "mod_id_map": {}, "mods": {},
            "dependencies": {}, "modfile_blobs": {}, "last_update_time": null
        }}});
        let mut archive = json!({"version": "0.0.0", "cache": {}});
        assert!(
            prune(
                &mut active,
                &mut archive,
                &[spec("https://mod.io/g/drg/m/my-mod#invalid_fragment")]
            )
            .is_err()
        );
        assert_eq!(archive, json!({"version": "0.0.0", "cache": {}}));
    }

    #[test]
    fn preserves_exact_pinned_file_even_without_mod_metadata() {
        let mut active = json!({"version": "0.0.0", "cache": {"modio": {
            "type": "ModioCache", "mod_id_map": {}, "mods": {},
            "dependencies": {}, "modfile_blobs": {"400": "pinned", "500": "unused"},
            "last_update_time": null
        }}});
        let mut archive = json!({"version": "0.0.0", "cache": {}});
        prune(
            &mut active,
            &mut archive,
            &[spec("https://mod.io/g/drg/m/pinned#10/400")],
        )
        .unwrap();
        assert!(
            active["cache"]["modio"]["modfile_blobs"]
                .get("400")
                .is_some()
        );
        assert!(
            archive["cache"]["modio"]["modfile_blobs"]
                .get("500")
                .is_some()
        );
    }
}
