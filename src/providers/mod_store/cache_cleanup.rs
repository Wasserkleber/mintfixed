// ModStore orchestration methods mechanically extracted; bodies unchanged.
use super::*;
use std::io::Write;
use tracing::{error, info};

impl ModStore {
    /// Run before Update Cache; all profile and group references, including disabled mods,
    /// must be passed in. Does not touch blob files or the actual mod downloads.
    pub fn archive_unreferenced(
        &self,
        live_specs: &[ModSpecification],
    ) -> Result<usize, ProviderError> {
        info!("Cache cleanup started: checking referenced mods across profiles and groups");
        let fail = |reason: String| {
            error!("Cache cleanup failed: {reason}");
            ProviderError::CacheCleanupFailed { reason }
        };
        let archive_path = self.cache_path.join("deprecated_mods.json");
        let mut current_lock = self.cache.write().unwrap();
        let mut current_json =
            serde_json::to_value(&**current_lock).map_err(|e| fail(e.to_string()))?;
        // Read the archive as plain JSON. Passing its tagged cache through Serde's
        // versioned wrapper can reject valid numeric mod.io map keys as strings.
        let mut archived_json: serde_json::Value = match std::fs::read(&archive_path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| fail(format!("invalid deprecated_mods.json: {e}")))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                serde_json::json!({ "version": "0.0.0", "cache": {} })
            }
            Err(e) => return Err(fail(format!("could not read deprecated_mods.json: {e}"))),
        };
        let before = current_json.clone();
        // Count cached mod.io entries and HTTP URLs before pruning.
        let checked = ["/cache/modio/mods", "/cache/http/url_blobs"]
            .iter()
            .filter_map(|path| before.pointer(path).and_then(|value| value.as_object()))
            .map(|entries| entries.len())
            .sum::<usize>();
        info!("Cache cleanup: {checked} cached mods checked");
        let moved = crate::providers::cache::cleanup::prune(
            &mut current_json,
            &mut archived_json,
            live_specs,
        )
        .map_err(fail)?;
        if current_json == before {
            info!("Cache cleanup complete: 0 mods archived");
            return Ok(0);
        }

        // Decode concrete providers directly from JSON bytes. This avoids
        // Serde's internally-tagged enum buffering bug for HashMap<u32, _> keys.
        // Do not use serde_json::from_value here: it triggers that same bug.
        fn parse_provider<T: serde::de::DeserializeOwned>(
            document: &serde_json::Value,
            name: &str,
        ) -> Result<Option<T>, String> {
            let Some(value) = document.get("cache").and_then(|cache| cache.get(name)) else {
                return Ok(None);
            };
            let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
            serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| format!("invalid {name} provider metadata: {e}"))
        }
        let active_modio: Option<crate::providers::modio::ModioCache> =
            parse_provider(&current_json, "modio").map_err(fail)?;
        let active_http: Option<crate::providers::http::HttpProviderCache> =
            parse_provider(&current_json, "http").map_err(fail)?;
        // Validate the archive before persisting it too.
        let _: Option<crate::providers::modio::ModioCache> =
            parse_provider(&archived_json, "modio").map_err(fail)?;
        let _: Option<crate::providers::http::HttpProviderCache> =
            parse_provider(&archived_json, "http").map_err(fail)?;

        // Archive first: if interrupted, duplicate archive records are safer than data loss.
        let mut temp =
            tempfile::NamedTempFile::new_in(&self.cache_path).map_err(|e| fail(e.to_string()))?;
        temp.write_all(
            &serde_json::to_vec_pretty(&archived_json).map_err(|e| fail(e.to_string()))?,
        )
        .map_err(|e| fail(e.to_string()))?;
        temp.persist(&archive_path)
            .map_err(|e| fail(e.to_string()))?;

        // Keep the original in-memory cache structure. Replace only provider
        // entries changed by the cleanup, not the version-tagged cache wrapper.
        if let Some(modio) = active_modio {
            current_lock
                .cache
                .insert("modio".to_owned(), Box::new(modio));
        }
        if let Some(http) = active_http {
            current_lock.cache.insert("http".to_owned(), Box::new(http));
        }
        current_lock.revision = current_lock.revision.wrapping_add(1);
        current_lock.save().map_err(|e| fail(e.to_string()))?;

        // Only report entries that were actually removed from the active cache.
        // Emit logs after the save succeeds, so failed cleanup is never reported
        // as completed. Both profile and group references were handled by prune.
        if let (Some(previous), Some(remaining)) = (
            before
                .pointer("/cache/modio/mods")
                .and_then(|value| value.as_object()),
            current_json
                .pointer("/cache/modio/mods")
                .and_then(|value| value.as_object()),
        ) {
            for (id, mod_info) in previous {
                if remaining.contains_key(id) {
                    continue;
                }
                let name = mod_info
                    .get("name")
                    .and_then(|value| value.as_str())
                    .or_else(|| mod_info.get("name_id").and_then(|value| value.as_str()))
                    .unwrap_or("Unknown mod");
                info!("Cache cleanup: deprecated {name} (Mod.io ID: {id})");
            }
        }
        if let (Some(previous), Some(remaining)) = (
            before
                .pointer("/cache/http/url_blobs")
                .and_then(|value| value.as_object()),
            current_json
                .pointer("/cache/http/url_blobs")
                .and_then(|value| value.as_object()),
        ) {
            for url in previous.keys() {
                if !remaining.contains_key(url) {
                    info!("Cache cleanup: deprecated HTTP mod {url}");
                }
            }
        }
        info!(
            "Cache cleanup complete: {moved} {} archived to {}",
            if moved == 1 { "mod" } else { "mods" },
            archive_path.display()
        );
        Ok(moved)
    }
}
