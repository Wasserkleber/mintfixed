//! End-to-end regression tests for Update Cache's metadata archival step.
//! These tests use temporary cache directories and never contact mod.io or alter
//! a player's real cache. They require the cache cleanup patch on mintfixed.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use mint::providers::{ModSpecification, ModStore};
use serde_json::{json, Value};

fn spec(url: &str) -> ModSpecification {
    ModSpecification::new(url.to_owned())
}

fn mod_entry(name_id: &str, file_id: u32) -> Value {
    json!({
        "name": name_id,
        "name_id": name_id,
        "latest_modfile": file_id,
        "modfiles": [{
            "id": file_id,
            "date_added": 1,
            "version": "1.0",
            "changelog": null
        }],
        "tags": []
    })
}

fn sample_cache() -> Value {
    // The numeric object keys deliberately reproduce the Serde problem seen in
    // real cache.json files: "1033779" is a JSON key but a u32 in Rust.
    json!({
        "version": "0.0.0",
        "cache": {
            "modio": {
                "type": "ModioCache",
                "last_update_time": null,
                "mod_id_map": {
                    "kept-mod": 1033779,
                    "dependency": 3096846,
                    "hardhat-dotty": 2016954
                },
                "dependencies": {
                    "1033779": [3096846],
                    "3096846": [],
                    "2016954": []
                },
                "modfile_blobs": {
                    "6100": "kept-blob",
                    "6101": "dependency-blob",
                    "2512163": "hardhat-blob"
                },
                "mods": {
                    "1033779": mod_entry("kept-mod", 6100),
                    "3096846": mod_entry("dependency", 6101),
                    "2016954": mod_entry("hardhat-dotty", 2512163)
                }
            },
            "http": {
                "type": "HttpProviderCache",
                "url_blobs": {
                    "https://example.org/used.pak": "used-http-blob",
                    "https://example.org/unused.pak": "unused-http-blob"
                }
            }
        }
    })
}

fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn make_store(dir: &Path) -> ModStore {
    write_json(&dir.join("cache.json"), &sample_cache());
    ModStore::new(dir, &HashMap::new()).unwrap()
}

fn kept_specs() -> Vec<ModSpecification> {
    // This is the combined set the Update Cache handler receives from every
    // profile and group; disabled members must be included in that set.
    vec![
        spec("https://mod.io/g/drg/m/kept-mod"),
        spec("https://example.org/used.pak"),
    ]
}

#[test]
fn moves_only_unreferenced_mods_and_related_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(dir.path());
    let moved = store.archive_unreferenced(&kept_specs()).unwrap();
    assert_eq!(moved, 2); // one mod.io mod and one HTTP URL

    let active = read_json(&dir.path().join("cache.json"));
    let archived = read_json(&dir.path().join("deprecated_mods.json"));
    let live_modio = &active["cache"]["modio"];
    let old_modio = &archived["cache"]["modio"];

    assert!(live_modio["mods"].get("1033779").is_some());
    assert!(live_modio["mods"].get("3096846").is_some()); // dependency
    assert!(live_modio["mods"].get("2016954").is_none());
    assert!(old_modio["mods"].get("2016954").is_some());
    assert!(live_modio["mod_id_map"].get("hardhat-dotty").is_none());
    assert!(old_modio["mod_id_map"].get("hardhat-dotty").is_some());
    assert!(live_modio["dependencies"].get("2016954").is_none());
    assert!(old_modio["dependencies"].get("2016954").is_some());
    assert!(live_modio["modfile_blobs"].get("2512163").is_none());
    assert!(old_modio["modfile_blobs"].get("2512163").is_some());
    assert!(active["cache"]["http"]["url_blobs"].get("https://example.org/used.pak").is_some());
    assert!(archived["cache"]["http"]["url_blobs"].get("https://example.org/unused.pak").is_some());
}

#[test]
fn archive_with_numeric_mod_ids_can_be_reloaded() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(dir.path());
    store.archive_unreferenced(&kept_specs()).unwrap();
    drop(store);

    // Reopening the real cache uses the same dynamic provider deserializer as
    // a user launching mintfixed after Update Cache.
    let reopened = ModStore::new(dir.path(), &HashMap::new()).unwrap();
    assert_eq!(reopened.archive_unreferenced(&kept_specs()).unwrap(), 0);

    // The archive must be in the same valid versioned cache format as cache.json.
    let archive_dir = tempfile::tempdir().unwrap();
    fs::copy(
        dir.path().join("deprecated_mods.json"),
        archive_dir.path().join("cache.json"),
    ).unwrap();
    ModStore::new(archive_dir.path(), &HashMap::new()).unwrap();
}

#[test]
fn later_cleanup_merges_archive_instead_of_overwriting_it() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(dir.path());
    assert_eq!(store.archive_unreferenced(&kept_specs()).unwrap(), 2);
    assert_eq!(store.archive_unreferenced(&[]).unwrap(), 3);

    let path = dir.path().join("deprecated_mods.json");
    let archived = read_json(&path);
    for id in ["1033779", "3096846", "2016954"] {
        assert!(archived["cache"]["modio"]["mods"].get(id).is_some(), "missing {id}");
    }
    for url in ["https://example.org/used.pak", "https://example.org/unused.pak"] {
        assert!(archived["cache"]["http"]["url_blobs"].get(url).is_some());
    }

    // Update Cache again without changing any profiles/groups: no new archive.
    let archive_before = fs::read(&path).unwrap();
    assert_eq!(store.archive_unreferenced(&[]).unwrap(), 0);
    assert_eq!(archive_before, fs::read(&path).unwrap());
}

#[test]
fn an_exact_pin_protects_mod_and_modfile_even_with_a_different_slug() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(dir.path());
    let pinned = spec("https://mod.io/g/drg/m/renamed-hardhat#2016954/2512163");
    assert_eq!(store.archive_unreferenced(&[pinned]).unwrap(), 4);

    let active = read_json(&dir.path().join("cache.json"));
    assert!(active["cache"]["modio"]["mods"].get("2016954").is_some());
    assert!(active["cache"]["modio"]["modfile_blobs"].get("2512163").is_some());
    assert!(active["cache"]["modio"]["mod_id_map"].get("hardhat-dotty").is_some());
}

#[test]
fn a_disabled_or_inactive_reference_must_still_be_passed_to_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(dir.path());
    // The caller supplies all_specs(), NOT just enabled/active-profile specs.
    // These simulate an inactive profile and a disabled group's member.
    let inactive_profile = spec("https://mod.io/g/drg/m/hardhat-dotty");
    let disabled_group = spec("https://mod.io/g/drg/m/kept-mod");
    assert_eq!(store.archive_unreferenced(&[
        inactive_profile, disabled_group,
        spec("https://example.org/used.pak"),
        spec("https://example.org/unused.pak"),
    ]).unwrap(), 0);
    assert!(!dir.path().join("deprecated_mods.json").exists());
}

#[test]
fn rejects_unknown_modio_reference_without_touching_files() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(dir.path());
    let before = fs::read(dir.path().join("cache.json")).unwrap();

    let result = store.archive_unreferenced(&[
        spec("https://mod.io/g/drg/m/kept-mod#invalid_fragment"),
    ]);
    assert!(result.is_err());
    assert_eq!(before, fs::read(dir.path().join("cache.json")).unwrap());
    assert!(!dir.path().join("deprecated_mods.json").exists());
}

#[test]
fn malformed_existing_archive_is_an_error_and_does_not_change_active_cache() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(dir.path());
    let archive_path = dir.path().join("deprecated_mods.json");
    fs::write(&archive_path, "{not valid json").unwrap();
    let before = fs::read(dir.path().join("cache.json")).unwrap();

    assert!(store.archive_unreferenced(&kept_specs()).is_err());
    assert_eq!(before, fs::read(dir.path().join("cache.json")).unwrap());
    assert_eq!(fs::read(&archive_path).unwrap(), b"{not valid json");
}

#[test]
fn cleanup_keeps_downloaded_blob_files_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let store = make_store(dir.path());
    let blob_path = dir.path().join("blobs").join("hardhat-blob");
    fs::create_dir_all(blob_path.parent().unwrap()).unwrap();
    fs::write(&blob_path, b"existing pak data").unwrap();

    assert_eq!(store.archive_unreferenced(&kept_specs()).unwrap(), 2);
    assert_eq!(fs::read(blob_path).unwrap(), b"existing pak data");
}
