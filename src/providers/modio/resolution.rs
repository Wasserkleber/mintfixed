use super::{
    AmbiguousModNameIdSnafu, DrgModio, HashMap, MODIO_PROVIDER_ID, ModInfo, ModResolution,
    ModResponse, ModSpecification, ModioCache, ModioProvider, NoAssociatedModfileSnafu,
    NoModsForNameIdSnafu, PreviewLinkSnafu, ProviderCache, ProviderError, ensure, format_spec,
    modio, parse_url, process_modio_tags, warn,
};
use snafu::OptionExt;

impl<M: DrgModio + Send + Sync> ModioProvider<M> {
    pub(super) async fn resolve_mod_impl(
        &self,
        spec: &ModSpecification,
        update: bool,
        cache: ProviderCache,
    ) -> Result<ModResponse, ProviderError> {
        ensure!(
            !spec.url.contains("?preview="),
            PreviewLinkSnafu {
                url: spec.url.to_string()
            }
        );

        fn read_cache<F, R>(cache: &ProviderCache, update: bool, f: F) -> Option<R>
        where
            F: Fn(&ModioCache) -> Option<R>,
        {
            (!update)
                .then(|| {
                    cache
                        .read()
                        .unwrap()
                        .get::<ModioCache>(MODIO_PROVIDER_ID)
                        .and_then(f)
                })
                .flatten()
        }

        fn write_cache<F>(cache: &ProviderCache, f: F)
        where
            F: FnOnce(&mut ModioCache),
        {
            f(cache
                .write()
                .unwrap()
                .get_mut::<ModioCache>(MODIO_PROVIDER_ID))
        }

        let url = &spec.url;
        let parsed = parse_url(url)?;

        if let (Some(mod_id), Some(_modfile_id)) = (parsed.mod_id, parsed.modfile_id) {
            // both mod ID and modfile ID specified, but not necessarily name
            let mod_ =
                if let Some(mod_) = read_cache(&cache, update, |c| c.mods.get(&mod_id).cloned()) {
                    mod_
                } else {
                    let mod_ = self.modio.fetch_mod(url.clone(), mod_id).await?;

                    write_cache(&cache, |c| {
                        c.mods.insert(mod_id, mod_.clone());
                        c.mod_id_map.insert(mod_.name_id.to_owned(), mod_id);
                    });

                    mod_
                };

            let dep_ids = match read_cache(&cache, update, |c| c.dependencies.get(&mod_id).cloned())
            {
                Some(deps) => deps,
                None => {
                    let deps = self.modio.fetch_dependencies(url.clone(), mod_id).await?;
                    write_cache(&cache, |c| {
                        c.dependencies.insert(mod_id, deps.clone());
                    });
                    deps
                }
            };

            let deps = {
                // build map of (id -> name) for deps
                let mut name_map = read_cache(&cache, false, |c| {
                    Some(
                        dep_ids
                            .iter()
                            .flat_map(|id| c.mods.get(id).map(|m| (*id, m.name_id.to_string())))
                            .collect::<HashMap<_, _>>(),
                    )
                })
                .unwrap_or_default();

                let filter_ids = dep_ids
                    .iter()
                    .filter(|id| !name_map.contains_key(id))
                    .cloned()
                    .collect::<Vec<_>>();
                if !filter_ids.is_empty() {
                    let mods = self.modio.fetch_mods_by_ids(filter_ids).await?;

                    for m in &mods {
                        name_map.insert(m.id, m.name_id.to_string());
                    }

                    for m in mods {
                        let id = m.id;
                        // TODO avoid fetching mod a second time
                        let m = self
                            .modio
                            .fetch_files(m.profile_url.to_string(), id)
                            .await?;
                        write_cache(&cache, |c| {
                            c.mod_id_map.insert(m.name_id.to_owned(), id);
                            c.mods.insert(id, m);
                        });
                    }
                }

                dep_ids
                    .iter()
                    .filter_map(|id| match name_map.get(id) {
                        Some(name) => Some(format_spec(name, *id, None)),
                        None => {
                            warn!("dependency ID missing from name_map: {id}");
                            None
                        }
                    })
                    .collect()
            };

            Ok(ModResponse::Resolve(ModInfo {
                provider: MODIO_PROVIDER_ID,
                spec: format_spec(&mod_.name_id, mod_id, None),
                name: mod_.name,
                versions: mod_
                    .modfiles
                    .into_iter()
                    .map(|f| format_spec(&mod_.name_id, mod_id, Some(f.id)))
                    .collect(),
                resolution: ModResolution::resolvable(url.as_str().into()),
                suggested_require: mod_.tags.contains("RequiredByAll"),
                suggested_dependencies: deps,
                modio_tags: Some(process_modio_tags(&mod_.tags)),
                modio_id: Some(mod_id),
            }))
        } else if let Some(mod_id) = parsed.mod_id {
            // only mod ID specified, use latest version (either cached local or remote depending)
            let mod_ = match read_cache(&cache, update, |c| c.mods.get(&mod_id).cloned()) {
                Some(mod_) => mod_,
                None => {
                    let mod_ = self.modio.fetch_mod(spec.url.clone(), mod_id).await?;
                    write_cache(&cache, |c| {
                        c.mods.insert(mod_id, mod_.clone());
                        c.mod_id_map.insert(mod_.name_id.to_owned(), mod_id);
                    });
                    mod_
                }
            };

            Ok(ModResponse::Redirect(format_spec(
                &mod_.name_id,
                mod_id,
                Some(
                    mod_.latest_modfile
                        .with_context(|| NoAssociatedModfileSnafu {
                            url: url.to_string(),
                        })?,
                ),
            )))
        } else {
            let name_id = parsed.name_id;

            let cached_id = read_cache(&cache, update, |c| c.mod_id_map.get(name_id).cloned());

            if let Some(id) = cached_id {
                let cached = read_cache(&cache, update, |c| {
                    c.mods.get(&id).and_then(|m| m.latest_modfile)
                });

                let modfile_id = match cached {
                    Some(modfile_id) => modfile_id,
                    None => {
                        let mod_ = self.modio.fetch_mod(spec.url.clone(), id).await?;
                        let modfile_id = mod_.latest_modfile;
                        write_cache(&cache, |c| {
                            c.mods.insert(id, mod_.clone());
                            c.mod_id_map.insert(mod_.name_id, id);
                        });
                        modfile_id.with_context(|| NoAssociatedModfileSnafu {
                            url: url.to_string(),
                        })?
                    }
                };

                Ok(ModResponse::Redirect(format_spec(
                    name_id,
                    id,
                    Some(modfile_id),
                )))
            } else {
                let mut mods = self.modio.fetch_mods_by_name(name_id).await?;
                if mods.len() > 1 {
                    AmbiguousModNameIdSnafu {
                        name_id: name_id.to_string(),
                    }
                    .fail()?
                } else if let Some(mod_) = mods.pop() {
                    let mod_id = mod_.id;
                    let mod_ = self.modio.fetch_mod(spec.url.clone(), mod_id).await?;
                    let modfile_id = mod_.latest_modfile;
                    write_cache(&cache, |c| {
                        c.mods.insert(mod_id, mod_.clone());
                        c.mod_id_map.insert(mod_.name_id, mod_id);
                    });
                    let file = modfile_id.with_context(|| NoAssociatedModfileSnafu {
                        url: url.to_string(),
                    })?;

                    Ok(ModResponse::Redirect(format_spec(
                        name_id,
                        mod_id,
                        Some(file),
                    )))
                } else {
                    NoModsForNameIdSnafu {
                        name_id: name_id.to_string(),
                    }
                    .fail()?
                }
            }
        }
    }
}
