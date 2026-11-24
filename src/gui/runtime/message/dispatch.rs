use super::{
    CheckUpdates, FetchModProgress, FetchSelfUpdateProgress, Integrate, LintMods, ResolveMods,
    SelfUpdate, UpdateCache,
};
use crate::gui::{App, LastAction, request_counter::RequestID};
use crate::providers::ModSpecification;
use tokio::task::JoinHandle;

#[derive(Debug, Clone, Copy)]
pub(in crate::gui) enum InstallPhase {
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
        target: crate::gui::group_controls::GroupDestination,
        prepend: bool,
        files: Result<Vec<ModSpecification>, String>,
    },
    ResolveMods(ResolveMods),
    Integrate(Integrate),
    FetchModProgress(FetchModProgress),
    InstallPhase {
        rid: RequestID,
        phase: InstallPhase,
    },
    CacheRecoveryProgress {
        rid: RequestID,
        completed: usize,
        total: usize,
    },
    UpdateCache(UpdateCache),
    CheckUpdates(CheckUpdates),
    LintMods(Box<LintMods>),
    SelfUpdate(SelfUpdate),
    FetchSelfUpdateProgress(FetchSelfUpdateProgress),
}

impl Message {
    pub fn handle(self, app: &mut App) {
        match self {
            Self::GroupFiles {
                target,
                prepend,
                files,
            } => {
                app.pending_group_files = app.pending_group_files.saturating_sub(1);
                match target.resolve(&app.state.mod_data) {
                    Some((profile, group)) => {
                        crate::gui::group_controls::add_files(app, profile, group, prepend, files)
                    }
                    None => {
                        app.status_state.last_action = Some(LastAction::failure(
                            "Addition cancelled: the original group destination no longer exists."
                                .into(),
                        ))
                    }
                }
            }
            Self::ResolveMods(msg) => msg.receive(app),
            Self::Integrate(msg) => msg.receive(app),
            Self::FetchModProgress(msg) => msg.receive(app),
            Self::InstallPhase { rid, phase } => {
                if app
                    .install_state
                    .integrate_rid
                    .as_ref()
                    .is_some_and(|h| h.rid == rid)
                {
                    app.install_state.install_phase = phase;
                }
            }
            Self::CacheRecoveryProgress {
                rid,
                completed,
                total,
            } => {
                if app
                    .install_state
                    .update_rid
                    .as_ref()
                    .is_some_and(|h| h.rid == rid)
                {
                    // Only successful counter advances reset the inactivity timer.
                    // Retries and duplicate progress messages do not count as progress.
                    if app.install_state.cache_recovery_progress.map(|(n, _)| n) != Some(completed)
                    {
                        app.install_state.cache_recovery_last_advance =
                            Some(std::time::Instant::now());
                    }
                    app.install_state.cache_recovery_progress = Some((completed, total));
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
