// ModStore orchestration methods mechanically extracted; bodies unchanged.
use super::*;
use std::collections::HashSet;

impl ModStore {
    pub async fn resolve_mods(
        &self,
        mods: &[ModSpecification],
        update: bool,
    ) -> Result<HashMap<ModSpecification, ModInfo>, ProviderError> {
        use futures::stream::{self, StreamExt, TryStreamExt};

        let mut to_resolve = mods.iter().cloned().collect::<HashSet<ModSpecification>>();
        let mut mods_map = HashMap::new();
        let mut precise_mod_specs = HashSet::new();
        // Track requests (including aliases), not only successfully resolved canonical specs.
        // This prevents cyclical dependencies or repeated requests from being re-enqueued.
        let mut attempted = HashSet::new();
        while !to_resolve.is_empty() {
            let batch = std::mem::take(&mut to_resolve);
            attempted.extend(batch.iter().cloned());
            let results =
                stream::iter(batch.into_iter().map(|spec| self.resolve_mod(spec, update)))
                    .boxed()
                    .buffer_unordered(5)
                    .try_collect::<Vec<_>>()
                    .await?;
            // Finish the batch before computing its next dependency frontier.
            for (request, info) in results {
                precise_mod_specs.insert(info.spec.clone());
                mods_map.insert(request, info);
            }
            to_resolve = crate::providers::dependencies::pending_dependencies(
                mods_map.values(),
                &precise_mod_specs,
                &attempted,
            );
        }
        Ok(mods_map)
    }

    pub async fn resolve_mod(
        &self,
        original_spec: ModSpecification,
        update: bool,
    ) -> Result<(ModSpecification, ModInfo), ProviderError> {
        let mut spec = original_spec.clone();
        loop {
            match self
                .get_provider(&spec.url)?
                .resolve_mod(&spec, update, self.cache.clone())
                .await?
            {
                ModResponse::Resolve(m) => {
                    return Ok((original_spec, m));
                }
                ModResponse::Redirect(redirected_spec) => spec = redirected_spec,
            };
        }
    }

    pub fn get_mod_info(&self, spec: &ModSpecification) -> Option<ModInfo> {
        self.get_provider(&spec.url)
            .ok()?
            .get_mod_info(spec, self.cache.clone())
    }

    pub fn is_pinned(&self, spec: &ModSpecification) -> bool {
        self.get_provider(&spec.url)
            .unwrap()
            .is_pinned(spec, self.cache.clone())
    }

    pub fn get_version_name(&self, spec: &ModSpecification) -> Option<String> {
        self.get_provider(&spec.url)
            .unwrap()
            .get_version_name(spec, self.cache.clone())
    }
}
