use super::*;

#[derive(Debug)]
pub struct LintMods {
    rid: RequestID,
    result: Result<LintReport, IntegrationError>,
}

impl LintMods {
    pub fn send(
        rc: &mut RequestCounter,
        store: Arc<ModStore>,
        mods: Vec<ModSpecification>,
        enabled_lints: BTreeSet<LintId>,
        game_pak_path: Option<PathBuf>,
        tx: Sender<Message>,
        ctx: egui::Context,
    ) -> MessageHandle<()> {
        let rid = rc.next();

        let handle = tokio::task::spawn(async move {
            let paths_res =
                resolve_async_ordered(store, ctx.clone(), mods.clone(), rid, tx.clone()).await;
            let mod_path_pairs_res =
                paths_res.map(|paths| mods.into_iter().zip(paths).collect::<Vec<_>>());

            let report_res = match mod_path_pairs_res {
                Ok(pairs) => tokio::task::spawn_blocking(move || {
                    crate::mod_lints::run_lints(
                        &enabled_lints,
                        pairs.into_iter().collect(),
                        game_pak_path,
                    )
                })
                .await
                .unwrap()
                .map_err(Into::into),
                Err(e) => Err(e),
            };

            tx.send(Message::LintMods(
                LintMods {
                    rid,
                    result: report_res,
                }
                .into(),
            ))
            .await
            .unwrap();
            ctx.request_repaint();
        });

        MessageHandle {
            rid,
            handle,
            state: Default::default(),
        }
    }

    pub(super) fn receive(self, app: &mut App) {
        if Some(self.rid) == app.lint_state.lint_rid.as_ref().map(|r| r.rid) {
            match self.result {
                Ok(mut report) => {
                    info!("lint mod report complete");
                    if let Some(run) = &app.lint_state.lint_run {
                        if let Some(anchor) = &run.focus {
                            crate::gui::lint::focus_only(&mut report, anchor);
                        }
                        match crate::gui::lint::save_txt(&app.state.dirs.data_dir, &report, run) {
                            Ok(path) => {
                                app.status_state.last_action = Some(LastAction::success(format!(
                                    "Lint report saved: {}",
                                    path.display()
                                )));
                                app.lint_state.lint_report_path = Some(path);
                                app.lint_state.lint_report_error = None;
                            }
                            Err(error) => {
                                app.lint_state.lint_report_path = None;
                                app.lint_state.lint_report_error =
                                    Some(format!("Could not save lint TXT report: {error}"));
                                app.status_state.last_action = Some(LastAction::failure(format!(
                                    "Lint checks finished, but TXT export failed: {error}"
                                )));
                            }
                        }
                    }
                    app.lint_state.lint_report = Some(report);
                }
                Err(ref e)
                    if let IntegrationError::ProviderError { source } = e
                        && let ProviderError::NoProvider { url: _, factory } = source =>
                {
                    app.window_provider_parameters =
                        Some(WindowProviderParameters::new(factory, &app.state));
                    app.status_state.last_action = Some(LastAction::failure("no provider".to_string()));
                }
                Err(e) => {
                    error!("{}", e);
                    app.status_state.problematic_mod_id = e.opt_mod_id();
                    app.status_state.last_action = Some(LastAction::failure(e.to_string()));
                }
            }
            app.lint_state.lint_rid = None;
        }
    }
}

async fn resolve_async_ordered(
    store: Arc<ModStore>,
    ctx: egui::Context,
    mod_specs: Vec<ModSpecification>,
    rid: RequestID,
    message_tx: Sender<Message>,
) -> Result<Vec<PathBuf>, IntegrationError> {
    let update = false;

    let mods = store.resolve_mods(&mod_specs, update).await?;

    let to_integrate = mod_specs
        .iter()
        .map(|u| mods[u].clone())
        .collect::<Vec<_>>();
    let res_map: HashMap<ModResolution, ModSpecification> = mods
        .iter()
        .map(|(spec, info)| (info.resolution.clone(), spec.clone()))
        .collect();
    let urls = to_integrate
        .iter()
        .map(|m| &m.resolution)
        .collect::<Vec<&ModResolution>>();

    let (tx, mut rx) = mpsc::channel::<FetchProgress>(10);

    tokio::spawn(async move {
        while let Some(progress) = rx.recv().await {
            if let Some(spec) = res_map.get(progress.resolution()) {
                message_tx
                    .send(Message::FetchModProgress(FetchModProgress {
                        rid,
                        spec: spec.clone(),
                        progress: progress.into(),
                    }))
                    .await
                    .unwrap();
                ctx.request_repaint();
            }
        }
    });

    Ok(store.fetch_mods_ordered(&urls, update, Some(tx)).await?)
}
