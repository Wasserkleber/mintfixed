#[derive(Debug)]
pub struct ResolveMods {
    rid: RequestID,
    profile: String,
    profile_id: String,
    input: String,
    specs: Vec<ModSpecification>,
    result: Result<HashMap<ModSpecification, ModInfo>, ProviderError>,
    is_dependency: bool,
    from_bitfix: bool,
}

impl ResolveMods {
    pub fn send(
        app: &mut App,
        ctx: &egui::Context,
        specs: Vec<ModSpecification>,
        is_dependency: bool,
    ) {
        if !is_dependency && specs.iter().any(|spec| spec.replacement_rule().is_some()) {
            if app.resolve_mod_rid.is_some() || app.pending_replacement_addition.is_some() {
                app.last_action = Some(LastAction::failure(
                    "Finish the pending mod addition first.".into(),
                ));
                return;
            }
            let active = &app.state.mod_data.active_profile;
            app.pending_replacement_addition = Some(crate::gui::PendingReplacementAddition {
                specs,
                is_dependency,
                profile_id: app.state.mod_data.profiles[active].id.clone(),
                decided: std::collections::HashSet::new(),
            });
            ctx.request_repaint();
            return;
        }
        Self::send_inner(app, ctx, specs, is_dependency, false);
    }
    /// Bypass the prompt only after an explicit choice in its confirmation dialog.
    pub(in crate::gui) fn send_confirmed(
        app: &mut App,
        ctx: &egui::Context,
        specs: Vec<ModSpecification>,
        is_dependency: bool,
    ) {
        Self::send_inner(app, ctx, specs, is_dependency, false);
    }
    pub fn send_bitfix(app: &mut App, ctx: &egui::Context, specs: Vec<ModSpecification>) {
        Self::send_inner(app, ctx, specs, false, true);
    }
    fn send_inner(
        app: &mut App,
        ctx: &egui::Context,
        specs: Vec<ModSpecification>,
        is_dependency: bool,
        from_bitfix: bool,
    ) {
        // Reject even mixed lists before inserting anything into the active profile.
        if !from_bitfix && specs.iter().any(crate::gui::bitfix_menu::is_lua_spec) {
            app.last_action = Some(LastAction::failure(
                crate::gui::bitfix_menu::REDIRECT_HINT.into(),
            ));
            return;
        }
        if from_bitfix
            && (specs.is_empty() || specs.iter().any(|s| !crate::gui::bitfix_menu::is_lua_spec(s)))
        {
            app.last_action = Some(LastAction::failure(
                "The Bitfix menu only accepts .lua script paths or URLs.".into(),
            ));
            return;
        }

        if app.resolve_mod_rid.is_some() {
            app.last_action = Some(LastAction::failure(
                "An addition is already running. Wait for it to finish, then try again.".into(),
            ));
            return;
        }
        let profile = app.state.mod_data.active_profile.clone();
        let profile_id = app.state.mod_data.profiles[&profile].id.clone();
        let input = if from_bitfix {
            app.bitfix_menu.input.clone()
        } else {
            app.resolve_mod.clone()
        };
        let rid = app.request_counter.next();
        let store = app.state.store.clone();
        let identity_specs = specs
            .iter()
            .any(|spec| !spec.url.starts_with("https://") && !spec.url.starts_with("http://"))
            .then(|| {
                let mut all = crate::mod_identity::all_specs(&app.state.mod_data);
                all.extend(specs.iter().cloned());
                all
            });
        let ctx = ctx.clone();
        let tx = app.tx.clone();
        let handle = tokio::spawn(async move {
            if let Some(identity_specs) = identity_specs {
                let identity_store = store.clone();
                if let Err(error) = tokio::task::spawn_blocking(move || {
                    identity_store.refresh_local_files(&identity_specs);
                })
                .await
                {
                    tracing::warn!("Local identity worker failed: {error}");
                }
            }
            let result = store.resolve_mods(&specs, false).await;
            tx.send(Message::ResolveMods(Self {
                rid,
                profile,
                profile_id,
                input,
                specs,
                result,
                is_dependency,
                from_bitfix,
            }))
            .await
            .unwrap();
            ctx.request_repaint();
        });
        app.last_action = None;
        app.resolve_mod_rid = Some(MessageHandle {
            rid,
            handle,
            state: (),
        });
    }

    fn receive(mut self, app: &mut App) {
        if Some(self.rid) == app.resolve_mod_rid.as_ref().map(|r| r.rid) {
            if self.from_bitfix {
                // Bitfix additions are global; they must not depend on a profile still existing.
                match self.result {
                    Ok(_) => {
                        let before = app.state.mod_data.deref().deref().clone();
                        let mut added = 0;
                        for spec in self.specs {
                            let global = &mut app.state.mod_data.bitfix_scripts;
                            if let Some(existing) = global.iter_mut().find(|mc| mc.spec == spec) {
                                existing.enabled = true;
                            } else {
                                global.push(ModConfig {
                                    id: crate::state::new_id(),
                                    description: None,
                                    member_id: None,
                                    spec,
                                    required: false,
                                    enabled: true,
                                    priority: 0,
                                });
                                added += 1;
                            }
                        }
                        if let Err(error) = app.state.mod_data.save() {
                            *app.state.mod_data.deref_mut().deref_mut() = before;
                            app.last_action = Some(LastAction::failure(format!(
                                "Could not save global Bitfix scripts: {error}"
                            )));
                        } else {
                            if app.bitfix_menu.input == self.input {
                                app.bitfix_menu.input.clear();
                            }
                            app.last_action = Some(LastAction::success(format!(
                                "Added {added} global Bitfix script(s)"
                            )));
                        }
                    }
                    Err(ProviderError::NoProvider { factory, .. }) => {
                        app.window_provider_parameters =
                            Some(WindowProviderParameters::new(factory, &app.state));
                        app.last_action = Some(LastAction::failure("no provider".into()));
                    }
                    Err(error) => {
                        app.problematic_mod_id = error.opt_mod_id();
                        app.last_action = Some(LastAction::failure(error.to_string()));
                    }
                }
                app.resolve_mod_rid = None;
                return;
            }
            let destination = app
                .state
                .mod_data
                .profiles
                .iter()
                .find(|(_, profile)| profile.id == self.profile_id)
                .map(|(name, _)| name.clone());
            let Some(destination) = destination else {
                app.resolve_mod_rid = None;
                app.last_action = Some(LastAction::failure(
                    "Addition cancelled: the original profile no longer exists.".into(),
                ));
                return;
            };
            self.profile = destination;
            match self.result {
                Ok(mut resolved_mods) => {
                    let before = app.state.mod_data.deref().deref().clone();
                    // `resolve_mods` returns a HashMap, so its iteration order cannot
                    // be used to reconstruct the order of pasted mods.
                    // Insert explicit mods in reverse order at index 0.
                    let mut ordered_mods = Vec::with_capacity(resolved_mods.len());
                    ordered_mods.extend(
                        crate::mod_identity::take_explicit(
                            self.specs,
                            &mut resolved_mods,
                            &app.state.store,
                        )
                        .into_iter()
                        .rev()
                        .map(|(spec, info)| (spec, info, false)),
                    );

                    // Keep automatically discovered dependencies, in a stable order.
                    let mut dependencies = resolved_mods.into_iter().collect::<Vec<_>>();
                    dependencies.sort_by(|a, b| a.0.url.cmp(&b.0.url));
                    ordered_mods.extend(
                        dependencies
                            .into_iter()
                            .map(|(spec, info)| (spec, info, true)),
                    );

                    let mut explicit_inserted = 0;
                    let mut dependencies_inserted = 0;
                    for (resolved_spec, info, auto_dependency) in ordered_mods {
                        let is_dep = self.is_dependency || auto_dependency;
                        let add = if is_dep {
                            // if mod is a dependency then check if there is a disabled
                            // mod that satisfies the dependency and enable it.
                            !app.state
                                .mod_data
                                .enable_dependency(&self.profile, &resolved_spec)
                        } else {
                            let data = &mut app.state.mod_data;
                            let profile = data.profiles.get_mut(&self.profile).unwrap();
                            let requested =
                                if crate::providers::modio::pinned_file(&resolved_spec.url)
                                    .is_some()
                                {
                                    &resolved_spec
                                } else {
                                    &info.spec
                                };
                            if crate::mod_identity::promote_standalone(
                                profile,
                                requested,
                                &app.state.store,
                            ) {
                                explicit_inserted += 1;
                                false
                            } else {
                                true
                            }
                        };

                        if add {
                            // Explicit mods stay in clipboard order at the top.
                            // Dependencies follow them without displacing their order.
                            let insertion_index = if auto_dependency {
                                let index = explicit_inserted + dependencies_inserted;
                                dependencies_inserted += 1;
                                index
                            } else {
                                explicit_inserted += 1;
                                0
                            };
                            let ModData { profiles, .. } =
                                app.state.mod_data.deref_mut().deref_mut();

                            profiles.get_mut(&self.profile).unwrap().mods.insert(
                                insertion_index,
                                ModOrGroup::Individual(ModConfig {
                                    id: crate::state::new_id(),
                                    description: None,
                                    member_id: None,
                                    spec: if crate::providers::modio::pinned_file(
                                        &resolved_spec.url,
                                    )
                                    .is_some()
                                    {
                                        resolved_spec.clone()
                                    } else {
                                        info.spec.clone()
                                    },
                                    required: info.suggested_require,
                                    enabled: true,
                                    priority: 0,
                                }),
                            );
                        }
                    }
                    if let Err(error) = app.state.mod_data.save() {
                        *app.state.mod_data.deref_mut().deref_mut() = before;
                        app.resolve_mod_rid = None;
                        app.last_action = Some(LastAction::failure(format!(
                            "Could not save additions: {error}"
                        )));
                        return;
                    }
                    if self.from_bitfix {
                        if app.bitfix_menu.input == self.input {
                            app.bitfix_menu.input.clear();
                        }
                    } else if app.resolve_mod == self.input {
                        app.resolve_mod.clear();
                    }
                    app.last_action = Some(LastAction::success(format!(
                        "Added {} mod{} to {}",
                        explicit_inserted + dependencies_inserted,
                        if explicit_inserted + dependencies_inserted == 1 {
                            ""
                        } else {
                            "s"
                        },
                        self.profile
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
            app.resolve_mod_rid = None;
        }
    }
}

