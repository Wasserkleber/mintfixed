#[derive(Debug)]
pub struct UpdateCache {
    rid: RequestID,
    // (references still missing metadata, cached mods, unavailable mod.io slugs)
    result: Result<(usize, usize, usize), ProviderError>,
}

impl UpdateCache {
    pub fn send(app: &mut App, ctx: &egui::Context) {
        let rid = app.request_counter.next();
        let tx = app.tx.clone();
        let ctx = ctx.clone();
        let store = app.state.store.clone();
        let referenced = crate::mod_identity::all_specs(&app.state.mod_data);
        let handle = tokio::spawn(async move {
            let res = async {
                store.update_cache().await?;
                let mut missing = referenced
                    .into_iter()
                    .filter(|spec| spec.url.starts_with("https://mod.io/g/drg/m/"))
                    .filter(|spec| store.get_mod_info(spec).is_none())
                    .collect::<Vec<_>>();
                missing.sort_by(|a, b| a.url.cmp(&b.url));
                info!("recovering metadata for {} mod.io entries", missing.len());
                tx.send(Message::CacheRecoveryProgress {
                    rid,
                    completed: 0,
                    total: missing.len(),
                })
                .await
                .ok();
                ctx.request_repaint();
                for (index, spec) in missing.iter().enumerate() {
                    info!(
                        "recovering metadata {}/{}: {}",
                        index + 1,
                        missing.len(),
                        spec.url
                    );
                    match store.resolve_mod(spec.clone(), false).await {
                        Ok(_) => store.clear_modio_unavailable(spec),
                        Err(error) if crate::providers::modio::is_unavailable_error(&error) => {
                            warn!(
                                "mod.io mod unavailable during metadata recovery: {} ({error})",
                                spec.url
                            );
                            store.mark_modio_unavailable(spec);
                        }
                        Err(error) => return Err(error),
                    }
                    tx.send(Message::CacheRecoveryProgress {
                        rid,
                        completed: index + 1,
                        total: missing.len(),
                    })
                    .await
                    .ok();
                    ctx.request_repaint();
                }
                let remaining = missing
                    .iter()
                    .filter(|spec| store.get_mod_info(spec).is_none())
                    .count();
                let (cached, unavailable) = store.modio_cache_counts();
                store.save_cache_metadata()?;
                Ok::<(usize, usize, usize), ProviderError>((remaining, cached, unavailable))
            }
            .await;
            tx.send(Message::UpdateCache(UpdateCache { rid, result: res }))
                .await
                .unwrap();
            ctx.request_repaint();
        });
        app.cache_recovery_progress = None;
        app.cache_recovery_last_advance = None;
        app.last_action = None;
        app.update_rid = Some(MessageHandle {
            rid,
            handle,
            state: (),
        });
    }

    fn receive(self, app: &mut App) {
        if Some(self.rid) == app.update_rid.as_ref().map(|r| r.rid) {
            match self.result {
                Ok((0, cached, 0)) => {
                    info!("cache update complete; referenced mod.io metadata recovered");
                    app.last_action = Some(LastAction::success(format!(
                        "Cache updated: {cached} cached mod.io mods ready"
                    )));
                }
                Ok((0, cached, unavailable)) => {
                    warn!("cache updated with {unavailable} unavailable mod.io mods");
                    app.last_action = Some(LastAction::success(format!(
                        "Cache updated: {cached} cached mod.io mods, {unavailable} unavailable (cached files preserved)"
                    )));
                }
                Ok((remaining, cached, unavailable)) => {
                    warn!(
                        "cache updated with {remaining} references missing metadata and {unavailable} unavailable mods"
                    );
                    // A missing active mod cannot be safely installed after updating.
                    // The failure status prevents the automatic install from starting.
                    app.last_action = Some(LastAction::failure(format!(
                        "Cache updated: {cached} cached mod.io mods, {unavailable} unavailable; {remaining} entries lack metadata"
                    )));
                }
                Err(ProviderError::NoProvider { url: _, factory }) => {
                    app.window_provider_parameters =
                        Some(WindowProviderParameters::new(factory, &app.state));
                    app.last_action = Some(LastAction::failure("no provider".to_string()));
                }
                Err(e) => {
                    error!("{}", e);
                    app.problematic_mod_id = e.opt_mod_id();
                    app.last_action = Some(LastAction::failure(e.to_string()));
                }
            }
            app.update_rid = None;
        }
    }
}

