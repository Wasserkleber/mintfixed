// ModStore orchestration methods mechanically extracted; bodies unchanged.
use super::*;
use std::path::Path;

impl ModStore {
    pub(crate) fn metadata_revision(&self) -> (u64, u64) {
        (
            self.cache.read().unwrap().revision,
            self.local_files.read().unwrap().0,
        )
    }

    pub(crate) fn cached_local_file(&self, spec: &ModSpecification) -> Option<[u8; 32]> {
        self.local_files.read().unwrap().1.get(&spec.url).copied()
    }

    /// Blocking: callers dispatch this to a worker before admission checks.
    pub(crate) fn refresh_local_files(&self, specs: &[ModSpecification]) {
        // Serialize workers without holding the UI's cache lock during file I/O.
        let _refresh = self.local_refresh.lock().unwrap();
        let mut refreshed = HashMap::new();
        for spec in specs {
            if spec.url.starts_with("https://") || spec.url.starts_with("http://") {
                continue;
            }
            refreshed
                .entry(spec.url.clone())
                .or_insert_with(|| crate::mod_identity::local_digest(Path::new(&spec.url)).ok());
        }
        let mut cache = self.local_files.write().unwrap();
        for (url, digest) in refreshed {
            match digest {
                Some(digest) => {
                    cache.1.insert(url, digest);
                }
                None => {
                    cache.1.remove(&url);
                }
            }
        }
        cache.0 = cache.0.wrapping_add(1);
    }
}
