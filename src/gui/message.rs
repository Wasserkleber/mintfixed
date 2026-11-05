use std::collections::BTreeSet;
use std::ops::{Deref, DerefMut};
use std::time::SystemTime;
use std::{collections::HashMap, sync::Arc};

use snafu::prelude::*;
use tokio::{
    sync::mpsc::{self, Sender},
    task::JoinHandle,
};
use tracing::*;

use super::SelfUpdateProgress;
use super::{
    App, SpecFetchProgress, WindowProviderParameters,
    request_counter::{RequestCounter, RequestID},
};
use crate::gui::LastAction;
use crate::integrate::*;
use crate::mod_lints::{LintId, LintReport};
use crate::state::{ModData_v0_1_0 as ModData, ModOrGroup};
use crate::*;
use crate::{
    providers::{FetchProgress, ModInfo, ModStore},
    state::ModConfig,
};
use mint_lib::error::GenericError;
use mint_lib::mod_info::MetaConfig;
use mint_lib::update::GitHubRelease;

#[derive(Debug, Clone, Copy)]
pub(super) enum InstallPhase {
    Resolving,
    Downloading,
    Integrating,
}

#[derive(Debug)]
pub struct MessageHandle<S> {
    pub rid: RequestID,
    pub handle: JoinHandle<()>,
    pub state: S,
}

#[derive(Debug)]
pub enum Message {
    GroupFiles {
        profile: String,
        group: String,
        prepend: bool,
        files: Result<Vec<ModSpecification>, String>,
    },
    ResolveMods(ResolveMods),
    Integrate(Integrate),
    FetchModProgress(FetchModProgress),
    InstallPhase { rid: RequestID, phase: InstallPhase },
    CacheRecoveryProgress { rid: RequestID, completed: usize, total: usize },
    UpdateCache(UpdateCache),
    CheckUpdates(CheckUpdates),
    LintMods(Box<LintMods>),
    SelfUpdate(SelfUpdate),
    FetchSelfUpdateProgress(FetchSelfUpdateProgress),
}

impl Message {
    pub fn handle(self, app: &mut App) {
        match self {
            Self::GroupFiles { profile, group, prepend, files } => {
                app.pending_group_files = app.pending_group_files.saturating_sub(1);
                super::group_controls::add_files(app, profile, group, prepend, files);
            }
            Self::ResolveMods(msg) => msg.receive(app),
            Self::Integrate(msg) => msg.receive(app),
            Self::FetchModProgress(msg) => msg.receive(app),
            Self::InstallPhase { rid, phase } => {
                if app.integrate_rid.as_ref().is_some_and(|h| h.rid == rid) {
                    app.install_phase = phase;
                }
            }
            Self::CacheRecoveryProgress { rid, completed, total } => {
                if app.update_rid.as_ref().is_some_and(|h| h.rid == rid) {
                    // Only successful counter advances reset the inactivity timer.
                    // Retries and duplicate progress messages do not count as progress.
                    if app.cache_recovery_progress.map(|(n, _)| n) != Some(completed) {
                        app.cache_recovery_last_advance = Some(std::time::Instant::now());
                    }
                    app.cache_recovery_progress = Some((completed, total));
                }
            }
            Self::UpdateCache(msg) => msg.receive(app),
            Self::CheckUpdates(msg) => msg.receive(app),
            Self::LintMods(msg) => msg.receive(app),
            Self::SelfUpdate(msg) => msg.receive(app),
            Self::FetchSelfUpdateProgress(msg) => msg.receive(app),
        }
    }
}

#[derive(Debug)]
pub struct ResolveMods {
    rid: RequestID,
    profile: String,
    input: String,
    specs: Vec<ModSpecification>,
    result: Result<HashMap<ModSpecification, ModInfo>, ProviderError>,
    is_dependency: bool,
}

impl ResolveMods {
    pub fn send(
        app: &mut App,
        ctx: &egui::Context,
        specs: Vec<ModSpecification>,
        is_dependency: bool,
    ) {
        if app.resolve_mod_rid.is_some() {
            app.last_action = Some(LastAction::failure("An addition is already running. Wait for it to finish, then try again.".into()));
            return;
        }
        let profile = app.state.mod_data.active_profile.clone();
        let input = app.resolve_mod.clone();
        let rid = app.request_counter.next();
        let store = app.state.store.clone();
        let identity_specs = specs.iter().any(|spec| {
            !spec.url.starts_with("https://") && !spec.url.starts_with("http://")
        }).then(|| {
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
                }).await {
                    tracing::warn!("Local identity worker failed: {error}");
                }
            }
            let result = store.resolve_mods(&specs, false).await;
            tx.send(Message::ResolveMods(Self {
                rid,
                profile,
                input,
                specs,
                result,
                is_dependency,
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

    fn receive(self, app: &mut App) {
        if Some(self.rid) == app.resolve_mod_rid.as_ref().map(|r| r.rid) {
            if !app.state.mod_data.profiles.contains_key(&self.profile) {
                app.resolve_mod_rid = None;
                app.last_action = Some(LastAction::failure("Addition cancelled: the original profile no longer exists.".into()));
                return;
            }
            match self.result {
                Ok(mut resolved_mods) => {
                    let before = app.state.mod_data.deref().deref().clone();
                    // `resolve_mods` returns a HashMap, so its iteration order cannot
                    // be used to reconstruct the order of pasted mods.
                    // Insert explicit mods in reverse order at index 0.
                    let mut ordered_mods = Vec::with_capacity(resolved_mods.len());
                    ordered_mods.extend(crate::mod_identity::take_explicit(
                        self.specs, &mut resolved_mods, &app.state.store,
                    ).into_iter().rev().map(|(spec, info)| (spec, info, false)));

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
                            !app.state.mod_data.enable_dependency(&self.profile, &resolved_spec)
                        } else {
                            let data = &mut app.state.mod_data;
                            let profile = data.profiles.get_mut(&self.profile).unwrap();
                            let requested = if crate::providers::modio::pinned_file(&resolved_spec.url).is_some() {
                                &resolved_spec
                            } else {
                                &info.spec
                            };
                            if crate::mod_identity::promote_standalone(profile, requested, &app.state.store) {
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
                            let ModData {
                                profiles,
                                ..
                            } = app.state.mod_data.deref_mut().deref_mut();

                            profiles.get_mut(&self.profile).unwrap().mods.insert(
                                insertion_index,
                                ModOrGroup::Individual(ModConfig {
                                    description: None,
                                    member_id: None,
                                    spec: if crate::providers::modio::pinned_file(&resolved_spec.url).is_some() {
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
                        app.last_action = Some(LastAction::failure(format!("Could not save additions: {error}")));
                        return;
                    }
                    if app.resolve_mod == self.input { app.resolve_mod.clear(); }
                    app.last_action = Some(LastAction::success(
                        format!("Added {} mod{} to {}", explicit_inserted + dependencies_inserted,
                            if explicit_inserted + dependencies_inserted == 1 { "" } else { "s" }, self.profile),
                    ));
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

#[derive(Debug)]
pub struct Integrate {
    rid: RequestID,
    result: Result<(), IntegrationError>,
}

impl Integrate {
    pub fn send(
        rc: &mut RequestCounter,
        store: Arc<ModStore>,
        mods: Vec<ModSpecification>,
        fsd_pak: PathBuf,
        config: MetaConfig,
        tx: Sender<Message>,
        ctx: egui::Context,
    ) -> MessageHandle<HashMap<ModSpecification, SpecFetchProgress>> {
        let rid = rc.next();
        MessageHandle {
            rid,
            handle: tokio::task::spawn(async move {
                let res =
                    integrate_async(store, ctx.clone(), mods, fsd_pak, config, rid, tx.clone())
                        .await;
                tx.send(Message::Integrate(Integrate { rid, result: res }))
                    .await
                    .unwrap();
                ctx.request_repaint();
            }),
            state: Default::default(),
        }
    }

    fn receive(self, app: &mut App) {
        if Some(self.rid) == app.integrate_rid.as_ref().map(|r| r.rid) {
            match self.result {
                Ok(()) => {
                    info!("integration complete");
                    app.last_action = Some(LastAction::success("integration complete".to_string()));
                }
                Err(ref e)
                    if let IntegrationError::ProviderError { source } = e
                        && let ProviderError::NoProvider { url: _, factory } = source =>
                {
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
            app.integrate_rid = None;
        }
    }
}

#[derive(Debug)]
pub struct FetchModProgress {
    rid: RequestID,
    spec: ModSpecification,
    progress: SpecFetchProgress,
}

impl FetchModProgress {
    fn receive(self, app: &mut App) {
        if let Some(MessageHandle { rid, state, .. }) = &mut app.integrate_rid
            && *rid == self.rid
        {
            state.insert(self.spec, self.progress);
        }
    }
}

#[derive(Debug)]
pub struct UpdateCache {
    rid: RequestID,
    result: Result<usize, ProviderError>,
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
                }).await.ok();
                ctx.request_repaint();
                for (index, spec) in missing.iter().enumerate() {
                    info!("recovering metadata {}/{}: {}", index + 1, missing.len(), spec.url);
                    store.resolve_mod(spec.clone(), false).await?;
                    tx.send(Message::CacheRecoveryProgress {
                        rid,
                        completed: index + 1,
                        total: missing.len(),
                    }).await.ok();
                    ctx.request_repaint();
                }
                Ok::<usize, ProviderError>(missing
                    .iter()
                    .filter(|spec| store.get_mod_info(spec).is_none())
                    .count())
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
                Ok(0) => {
                    info!("cache update complete; referenced mod.io metadata recovered");
                    app.last_action = Some(LastAction::success(
                        "Cache updated; referenced mod.io metadata ready".to_string(),
                    ));
                }
                Ok(remaining) => {
                    error!("cache recovery incomplete: {remaining} referenced mod.io entries missing metadata");
                    app.last_action = Some(LastAction::failure(format!(
                        "Cache recovery incomplete: {remaining} mod.io entries still lack metadata"
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

#[derive(Debug)]
pub struct CheckUpdates {
    rid: RequestID,
    result: Result<GitHubRelease, GenericError>,
}

impl CheckUpdates {
    pub fn send(app: &mut App, ctx: &egui::Context) {
        let rid = app.request_counter.next();
        let tx = app.tx.clone();
        let ctx = ctx.clone();

        let handle = tokio::spawn(async move {
            tx.send(Message::CheckUpdates(Self {
                rid,
                result: mint_lib::update::get_latest_release().await,
            }))
            .await
            .unwrap();
            ctx.request_repaint();
        });
        app.check_updates_rid = Some(MessageHandle {
            rid,
            handle,
            state: (),
        });
    }

    fn receive(self, app: &mut App) {
        if Some(self.rid) == app.check_updates_rid.as_ref().map(|r| r.rid) {
            app.check_updates_rid = None;
            match self.result {
                Ok(release) => {
                    if let (Ok(version), Some(Ok(release_version))) = (
                        semver::Version::parse(env!("CARGO_PKG_VERSION")),
                        release
                            .tag_name
                            .strip_prefix('v')
                            .map(semver::Version::parse),
                    ) && release_version > version
                    {
                        app.available_update = Some(release);
                        app.show_update_time = Some(SystemTime::now());
                    }
                }
                Err(e) => tracing::warn!("failed to fetch update {e}"),
            }
        }
    }
}

async fn integrate_async(
    store: Arc<ModStore>,
    ctx: egui::Context,
    mod_specs: Vec<ModSpecification>,
    fsd_pak: PathBuf,
    config: MetaConfig,
    rid: RequestID,
    message_tx: Sender<Message>,
) -> Result<(), IntegrationError> {
    let update = false;

    let mods = store.resolve_mods(&mod_specs, update).await?;

    let to_integrate = crate::mod_identity::installation_inputs(&mod_specs, &mods)?;
    // Report phases from the actual async operation, not from guessed download progress.
    message_tx.send(Message::InstallPhase { rid, phase: InstallPhase::Downloading }).await.ok();
    ctx.request_repaint();
    let stage_tx = message_tx.clone();
    let stage_ctx = ctx.clone();
    // Index aliases once so every represented row receives fetch progress.
    let identity = |info: &ModInfo| {
        use crate::mod_identity::Artifact;
        match crate::providers::modio::pinned_file(&info.resolution.url.0) {
            Some((id, file)) if info.provider == "modio" => Artifact::Modio(id, file),
            _ => Artifact::Specification(ModSpecification { url: info.resolution.url.0.clone() }),
        }
    };
    let mut aliases = HashMap::<_, Vec<ModSpecification>>::new();
    for spec in &mod_specs {
        let info = &mods[spec];
        aliases.entry((info.provider, identity(info))).or_default().push(spec.clone());
    }
    let res_map: HashMap<ModResolution, Vec<ModSpecification>> = to_integrate.iter().map(|info| {
        (info.resolution.clone(), aliases.remove(&(info.provider, identity(info))).unwrap_or_default())
    }).collect();
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
    stage_tx.send(Message::InstallPhase { rid, phase: InstallPhase::Integrating }).await.ok();
    stage_ctx.request_repaint();

    tokio::task::spawn_blocking(|| {
        crate::integrate::integrate(
            fsd_pak,
            config,
            to_integrate.into_iter().zip(paths).collect(),
        )
    })
    .await??;

    Ok(())
}

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

    fn receive(self, app: &mut App) {
        if Some(self.rid) == app.lint_rid.as_ref().map(|r| r.rid) {
            match self.result {
                Ok(report) => {
                    info!("lint mod report complete");
                    app.lint_report = Some(report);
                    app.last_action =
                        Some(LastAction::success("lint mod report complete".to_string()));
                }
                Err(ref e)
                    if let IntegrationError::ProviderError { source } = e
                        && let ProviderError::NoProvider { url: _, factory } = source =>
                {
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
            app.integrate_rid = None;
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

#[derive(Debug)]
pub struct SelfUpdate {
    rid: RequestID,
    result: Result<PathBuf, IntegrationError>,
}

impl SelfUpdate {
    pub fn send(
        rc: &mut RequestCounter,
        tx: Sender<Message>,
        ctx: egui::Context,
    ) -> MessageHandle<SelfUpdateProgress> {
        let rid = rc.next();
        MessageHandle {
            rid,
            handle: tokio::task::spawn(async move {
                let result = self_update_async(ctx.clone(), rid, tx.clone()).await;
                tx.send(Message::SelfUpdate(SelfUpdate { rid, result }))
                    .await
                    .unwrap();
                ctx.request_repaint();
            }),
            state: SelfUpdateProgress::Pending,
        }
    }

    fn receive(self, app: &mut App) {
        if Some(self.rid) == app.self_update_rid.as_ref().map(|r| r.rid) {
            match self.result {
                Ok(original_exe_path) => {
                    info!("self update complete");
                    app.original_exe_path = Some(original_exe_path);
                    app.last_action = Some(LastAction::success("self update complete".to_string()));
                }
                Err(e) => {
                    error!("self update failed");
                    error!("{:#?}", e);
                    app.self_update_rid = None;
                    app.last_action = Some(LastAction::failure("self update failed".to_string()));
                }
            }
            app.integrate_rid = None;
        }
    }
}

#[derive(Debug)]
pub struct FetchSelfUpdateProgress {
    rid: RequestID,
    progress: SelfUpdateProgress,
}

impl FetchSelfUpdateProgress {
    fn receive(self, app: &mut App) {
        if let Some(MessageHandle { rid, state, .. }) = &mut app.self_update_rid
            && *rid == self.rid
        {
            *state = self.progress;
        }
    }
}

async fn self_update_async(
    ctx: egui::Context,
    rid: RequestID,
    message_tx: Sender<Message>,
) -> Result<PathBuf, IntegrationError> {
    use futures::stream::TryStreamExt;
    use tokio::io::AsyncWriteExt;

    let (tx, mut rx) = mpsc::channel::<SelfUpdateProgress>(1);

    tokio::spawn(async move {
        while let Some(progress) = rx.recv().await {
            message_tx
                .send(Message::FetchSelfUpdateProgress(FetchSelfUpdateProgress {
                    rid,
                    progress,
                }))
                .await
                .unwrap();
            ctx.request_repaint();
        }
    });

    let client = reqwest::Client::new();

    let asset_name = if cfg!(target_os = "windows") {
        "mint-x86_64-pc-windows-msvc.zip"
    } else if cfg!(target_os = "linux") {
        "mint-x86_64-unknown-linux-gnu.zip"
    } else {
        unimplemented!("unsupported platform");
    };

    info!("downloading update");

    let response = client
        .get(format!(
            "https://github.com/trumank/mint/releases/latest/download/{asset_name}"
        ))
        .send()
        .await
        .map_err(Into::into)
        .with_context(|_| SelfUpdateFailedSnafu)?
        .error_for_status()
        .map_err(Into::into)
        .with_context(|_| SelfUpdateFailedSnafu)?;
    let size = response.content_length();
    debug!(?response);
    debug!(?size);

    let tmp_dir = tempfile::Builder::new()
        .prefix("self_update")
        .tempdir_in(std::env::current_dir()?)?;
    let tmp_archive_path = tmp_dir.path().join(asset_name);
    let mut tmp_archive = tokio::fs::File::create(&tmp_archive_path)
        .await
        .map_err(Into::into)
        .with_context(|_| SelfUpdateFailedSnafu)?;
    let mut stream = response.bytes_stream();

    let mut total_bytes_written = 0;
    while let Some(bytes) = stream
        .try_next()
        .await
        .map_err(Into::into)
        .with_context(|_| SelfUpdateFailedSnafu)?
    {
        let bytes_written = tmp_archive.write(&bytes).await?;
        total_bytes_written += bytes_written;
        if let Some(size) = size {
            tx.send(SelfUpdateProgress::Progress {
                progress: total_bytes_written as u64,
                size,
            })
            .await
            .unwrap();
        }
    }

    debug!(?tmp_dir);
    debug!(?tmp_archive_path);
    debug!(?tmp_archive);

    let original_exe_path =
        tokio::task::spawn_blocking(move || -> Result<PathBuf, IntegrationError> {
            let bin_name = if cfg!(target_os = "windows") {
                "mint.exe"
            } else if cfg!(target_os = "linux") {
                "mint"
            } else {
                unimplemented!("unsupported platform");
            };

            info!("extracting downloaded update archive");
            self_update::Extract::from_source(&tmp_archive_path)
                .archive(self_update::ArchiveKind::Zip)
                .extract_file(tmp_dir.path(), bin_name)
                .map_err(Into::into)
                .with_context(|_| SelfUpdateFailedSnafu)?;

            info!("replacing old executable with new executable");
            let tmp_file = tmp_dir.path().join("replacement_tmp");
            let bin_path = tmp_dir.path().join(bin_name);

            let original_exe_path = std::env::current_exe()?;

            self_update::Move::from_source(&bin_path)
                .replace_using_temp(&tmp_file)
                .to_dest(&original_exe_path)
                .map_err(Into::into)
                .with_context(|_| SelfUpdateFailedSnafu)?;

            #[cfg(target_os = "linux")]
            {
                info!("setting executable permission on new executable");
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&original_exe_path, std::fs::Permissions::from_mode(0o755))
                    .unwrap();
            }

            Ok(original_exe_path)
        })
        .await??;

    tx.send(SelfUpdateProgress::Complete).await.unwrap();

    info!("update successful");

    Ok(original_exe_path)
}

#[cfg(test)]
mod addition_destination_tests {
    use super::*;

    fn app() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        let app = App::new(&eframe::CreationContext::_new_kittest(egui::Context::default()),
            Dirs::from_path(dir.path()).unwrap(), None).unwrap();
        (dir, app)
    }

    fn result(app: &mut App) -> ResolveMods {
        let rid = app.request_counter.next();
        app.resolve_mod_rid = Some(MessageHandle { rid, handle: tokio::spawn(async {}), state: () });
        let spec = ModSpecification::new("test-mod".to_owned());
        let info = ModInfo { provider: "file", name: "Test".into(), spec: spec.clone(), versions: vec![],
            resolution: ModResolution::resolvable("test-mod".to_owned().into()),
            suggested_require: false, suggested_dependencies: vec![], modio_id: None, modio_tags: None };
        ResolveMods { rid, profile: "default".into(), input: "submitted input".into(),
            specs: vec![spec.clone()], result: Ok(HashMap::from([(spec, info)])), is_dependency: false }
    }

    #[tokio::test]
    async fn profile_switch_does_not_redirect_result_or_clear_new_input() {
        let (_dir, mut app) = app();
        let result = result(&mut app);
        app.state.mod_data.profiles.insert("Other".into(), Default::default());
        app.state.mod_data.active_profile = "Other".into();
        app.resolve_mod = "new input".into();
        result.receive(&mut app);
        assert_eq!(app.state.mod_data.profiles["default"].mods.len(), 1);
        assert!(app.state.mod_data.profiles["Other"].mods.is_empty());
        assert_eq!(app.resolve_mod, "new input");
        assert!(app.resolve_mod_rid.is_none());
    }

    #[tokio::test]
    async fn second_request_does_not_replace_pending_result() {
        let (_dir, mut app) = app();
        let result = result(&mut app);
        ResolveMods::send(&mut app, &egui::Context::default(), vec![], false);
        assert_eq!(app.resolve_mod_rid.as_ref().unwrap().rid, result.rid);
        result.receive(&mut app);
        assert_eq!(app.state.mod_data.profiles["default"].mods.len(), 1);
    }

    #[tokio::test]
    async fn missing_original_profile_cancels_without_fallback() {
        let (_dir, mut app) = app();
        let result = result(&mut app);
        app.state.mod_data.profiles.insert("Other".into(), Default::default());
        app.state.mod_data.active_profile = "Other".into();
        app.state.mod_data.profiles.remove("default");
        result.receive(&mut app);
        assert!(app.state.mod_data.profiles["Other"].mods.is_empty());
        assert!(app.resolve_mod_rid.is_none());
    }
}
