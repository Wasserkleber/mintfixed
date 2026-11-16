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

        // used to deduplicate dependencies from mods already present in the mod list
        let mut precise_mod_specs = HashSet::new();

        while !to_resolve.is_empty() {
            for (u, m) in stream::iter(
                to_resolve
                    .iter()
                    .map(|u| self.resolve_mod(u.to_owned(), update)),
            )
            .boxed()
            .buffer_unordered(5)
            .try_collect::<Vec<_>>()
            .await?
            {
                precise_mod_specs.insert(m.spec.clone());
                mods_map.insert(u, m);
                to_resolve.clear();
                for m in mods_map.values() {
                    for d in &m.suggested_dependencies {
                        if !precise_mod_specs.contains(d) {
                            to_resolve.insert(d.clone());
                        }
                    }
                }
            }
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
