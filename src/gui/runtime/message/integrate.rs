use super::*;

#[derive(Debug)]
pub struct Integrate {
    rid: RequestID,
    result: Result<crate::installation::report::InstallReport, IntegrationError>,
}

impl Integrate {
    pub fn send(
        rc: &mut RequestCounter,
        store: Arc<ModStore>,
        entries: Vec<crate::state::InstallEntry>,
        fsd_pak: PathBuf,
        config: MetaConfig,
        tx: Sender<Message>,
        ctx: egui::Context,
    ) -> MessageHandle<HashMap<ModSpecification, SpecFetchProgress>> {
        let rid = rc.next();
        MessageHandle {
            rid,
            handle: tokio::task::spawn(async move {
                let res = integrate_async(
                    store,
                    ctx.clone(),
                    entries,
                    fsd_pak,
                    config,
                    rid,
                    tx.clone(),
                )
                .await;
                tx.send(Message::Integrate(Integrate { rid, result: res }))
                    .await
                    .unwrap();
                ctx.request_repaint();
            }),
            state: Default::default(),
        }
    }

    pub(super) fn receive(self, app: &mut App) {
        if Some(self.rid) == app.install_state.integrate_rid.as_ref().map(|r| r.rid) {
            match self.result {
                Ok(report) => {
                    let mut action = LastAction::success(report.summary());
                    action.details = Some(report.details());
                    app.status_state.last_action = Some(action);
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
            app.install_state.integrate_rid = None;
        }
    }
}

#[derive(Debug)]
pub struct FetchModProgress {
    pub(super) rid: RequestID,
    pub(super) spec: ModSpecification,
    pub(super) progress: SpecFetchProgress,
}

impl FetchModProgress {
    pub(super) fn receive(self, app: &mut App) {
        if let Some(MessageHandle { rid, state, .. }) = &mut app.install_state.integrate_rid
            && *rid == self.rid
        {
            state.insert(self.spec, self.progress);
        }
    }
}
