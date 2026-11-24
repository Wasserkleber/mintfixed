use super::{InstallPhase, Message, MessageHandle};
use crate::gui::{
    App, LastAction, SpecFetchProgress, WindowProviderParameters,
    request_counter::{RequestCounter, RequestID},
};
use crate::integrate::IntegrationError;
use crate::providers::{
    FetchProgress, ModInfo, ModResolution, ModSpecification, ModStore, ProviderError,
};
use mint_lib::mod_info::MetaConfig;
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tokio::sync::mpsc::{self, Sender};
use tracing::error;

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
                    app.status_state.last_action =
                        Some(LastAction::failure("no provider".to_string()));
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

async fn integrate_async(
    store: Arc<ModStore>,
    ctx: egui::Context,
    entries: Vec<crate::state::InstallEntry>,
    fsd_pak: PathBuf,
    config: MetaConfig,
    rid: RequestID,
    message_tx: Sender<Message>,
) -> Result<crate::installation::report::InstallReport, IntegrationError> {
    let update = false;
    let mod_specs: Vec<_> = entries.iter().map(|entry| entry.spec.clone()).collect();

    let mods = store.resolve_mods(&mod_specs, update).await?;

    let selection = crate::installation::validation::select_installation_inputs(&entries, &mods)?;
    let to_integrate = selection.mods;
    // Report phases from the actual async operation, not from guessed download progress.
    message_tx
        .send(Message::InstallPhase {
            rid,
            phase: InstallPhase::Downloading,
        })
        .await
        .ok();
    ctx.request_repaint();
    let stage_tx = message_tx.clone();
    let stage_ctx = ctx.clone();
    // Index aliases once so every represented row receives fetch progress.
    let identity = |info: &ModInfo| {
        use crate::mod_identity::Artifact;
        match crate::providers::modio::pinned_file(&info.resolution.url.0) {
            Some((id, file)) if info.provider == "modio" => Artifact::Modio(id, file),
            _ => Artifact::Specification(ModSpecification {
                url: info.resolution.url.0.clone(),
            }),
        }
    };
    let mut aliases = HashMap::<_, Vec<ModSpecification>>::new();
    for spec in &mod_specs {
        let info = &mods[spec];
        aliases
            .entry((info.provider, identity(info)))
            .or_default()
            .push(spec.clone());
    }
    let res_map: HashMap<ModResolution, Vec<ModSpecification>> = to_integrate
        .iter()
        .map(|info| {
            (
                info.resolution.clone(),
                aliases
                    .remove(&(info.provider, identity(info)))
                    .unwrap_or_default(),
            )
        })
        .collect();
    let urls = to_integrate
        .iter()
        .map(|m| &m.resolution)
        .collect::<Vec<_>>();

    let (tx, mut rx) = mpsc::channel::<FetchProgress>(10);

    tokio::spawn(async move {
        while let Some(progress) = rx.recv().await {
            if let Some(specs) = res_map.get(progress.resolution()) {
                let progress: SpecFetchProgress = progress.into();
                for spec in specs {
                    message_tx
                        .send(Message::FetchModProgress(FetchModProgress {
                            rid,
                            spec: spec.clone(),
                            progress: progress.clone(),
                        }))
                        .await
                        .unwrap();
                }
                ctx.request_repaint();
            }
        }
    });

    let paths = store.fetch_mods_ordered(&urls, update, Some(tx)).await?;
    stage_tx
        .send(Message::InstallPhase {
            rid,
            phase: InstallPhase::Integrating,
        })
        .await
        .ok();
    stage_ctx.request_repaint();

    tokio::task::spawn_blocking(|| {
        crate::integrate::integrate_with_report(
            fsd_pak,
            config,
            to_integrate.into_iter().zip(paths).collect(),
            selection.locations,
            selection.skipped,
        )
    })
    .await?
}
