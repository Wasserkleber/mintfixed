// ModStore orchestration methods mechanically extracted; bodies unchanged.
use super::*;

impl ModStore {
    /// Status metadata is stored in cache.json, not in profiles or mod blobs.
    pub fn is_modio_unavailable(&self, spec: &ModSpecification) -> bool {
        crate::providers::modio::is_unavailable(&self.cache, &spec.url)
    }

    pub fn mark_modio_unavailable(&self, spec: &ModSpecification) {
        crate::providers::modio::mark_unavailable(&self.cache, &spec.url);
    }

    pub fn clear_modio_unavailable(&self, spec: &ModSpecification) {
        crate::providers::modio::clear_unavailable(&self.cache, &spec.url);
    }

    pub fn modio_cache_counts(&self) -> (usize, usize) {
        crate::providers::modio::cache_counts(&self.cache)
    }

    pub(crate) fn cached_modio_file(&self, spec: &ModSpecification) -> Option<(u32, u32)> {
        crate::providers::modio::cached_file(&spec.url, &self.cache)
    }

    pub(crate) fn cached_modio_name(&self, id: u32) -> Option<String> {
        crate::providers::modio::cached_name(id, &self.cache)
    }
}
