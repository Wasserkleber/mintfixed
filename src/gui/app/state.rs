//! Runtime-only UI state grouped by responsibility. Persistent data stays in State.
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::time::{Instant, SystemTime};

use mint_lib::update::GitHubRelease;

use crate::gui::{
    LastAction, PendingReplacementAddition, SelfUpdateProgress, SpecFetchProgress,
    WindowLintReport, WindowLintsToggle,
    lint::{self, LintOptions},
    message::{self, MessageHandle},
    request_counter::RequestID,
    unavailable_mods,
};
use crate::mod_lints::LintReport;
use crate::providers::ModSpecification;

#[derive(Default)]
pub(in crate::gui) struct LintUiState {
    pub(in crate::gui) lint_rid: Option<MessageHandle<()>>,
    pub(in crate::gui) lint_report_window: Option<WindowLintReport>,
    pub(in crate::gui) lint_report: Option<LintReport>,
    pub(in crate::gui) lints_toggle_window: Option<WindowLintsToggle>,
    pub(in crate::gui) lint_options: LintOptions,
    pub(in crate::gui) lint_scope: lint::LintScope,
    pub(in crate::gui) lint_selected_mods: BTreeSet<ModSpecification>,
    pub(in crate::gui) lint_focus: Option<ModSpecification>,
    pub(in crate::gui) lint_filter: String,
    pub(in crate::gui) lint_focus_filter: String,
    pub(in crate::gui) lint_run: Option<lint::LintRun>,
    pub(in crate::gui) lint_report_path: Option<PathBuf>,
    pub(in crate::gui) lint_report_error: Option<String>,
}

pub(in crate::gui) struct InstallUiState {
    pub(in crate::gui) integrate_rid:
        Option<MessageHandle<HashMap<ModSpecification, SpecFetchProgress>>>,
    // Tracks only installations started by Update Cache, not manual installations.
    pub(in crate::gui) cache_install_rid: Option<RequestID>,
    pub(in crate::gui) cache_install_notice: Option<String>,
    pub(in crate::gui) cache_install_details: Option<String>,
    pub(in crate::gui) cache_install_locations: Vec<unavailable_mods::Location>,
    pub(in crate::gui) install_phase: message::InstallPhase,
    pub(in crate::gui) cache_recovery_progress: Option<(usize, usize)>,
    pub(in crate::gui) cache_recovery_last_advance: Option<Instant>,
    pub(in crate::gui) update_rid: Option<MessageHandle<()>>,
}

impl Default for InstallUiState {
    fn default() -> Self {
        Self {
            integrate_rid: None,
            cache_install_rid: None,
            cache_install_notice: None,
            cache_install_details: None,
            cache_install_locations: Vec::new(),
            install_phase: message::InstallPhase::Resolving,
            cache_recovery_progress: None,
            cache_recovery_last_advance: None,
            update_rid: None,
        }
    }
}

#[derive(Default)]
pub(in crate::gui) struct SearchUiState {
    pub(in crate::gui) search_string: String,
    pub(in crate::gui) scroll_to_match: bool,
    pub(in crate::gui) focus_search: bool,
}

#[derive(Default)]
pub(in crate::gui) struct UpdateUiState {
    pub(in crate::gui) check_updates_rid: Option<MessageHandle<()>>,
    pub(in crate::gui) available_update: Option<GitHubRelease>,
    pub(in crate::gui) show_update_time: Option<SystemTime>,
    pub(in crate::gui) self_update_rid: Option<MessageHandle<SelfUpdateProgress>>,
    pub(in crate::gui) original_exe_path: Option<PathBuf>,
}

#[derive(Default)]
pub(in crate::gui) struct StatusUiState {
    pub(in crate::gui) last_action: Option<LastAction>,
    pub(in crate::gui) status_details: Option<String>,
    pub(in crate::gui) status_locations: Vec<unavailable_mods::Location>,
    pub(in crate::gui) unavailable_overview: unavailable_mods::Overview,
    pub(in crate::gui) live_unavailable_details: bool,
    pub(in crate::gui) show_mod_target: Option<unavailable_mods::ResolvedTarget>,
    pub(in crate::gui) problematic_mod_id: Option<u32>,
}

#[derive(Default)]
pub(in crate::gui) struct ResolveUiState {
    pub(in crate::gui) resolve_mod: String,
    pub(in crate::gui) resolve_mod_rid: Option<MessageHandle<()>>,
    pub(in crate::gui) pending_replacement_addition: Option<PendingReplacementAddition>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_states_start_idle() {
        let lint = LintUiState::default();
        assert!(lint.lint_rid.is_none());
        assert_eq!(lint.lint_scope, lint::LintScope::All);
        assert!(lint.lint_selected_mods.is_empty());
        assert!(lint.lint_report.is_none());
        assert!(lint.lint_report_window.is_none());

        let install = InstallUiState::default();
        assert!(install.integrate_rid.is_none());
        assert!(install.update_rid.is_none());
        assert!(install.cache_install_rid.is_none());
        assert!(matches!(
            install.install_phase,
            message::InstallPhase::Resolving
        ));

        let search = SearchUiState::default();
        assert!(search.search_string.is_empty());
        assert!(!search.scroll_to_match);
        assert!(!search.focus_search);

        let update = UpdateUiState::default();
        assert!(update.check_updates_rid.is_none());
        assert!(update.self_update_rid.is_none());

        let resolve = ResolveUiState::default();
        assert!(resolve.resolve_mod.is_empty());
        assert!(resolve.resolve_mod_rid.is_none());
        assert!(resolve.pending_replacement_addition.is_none());

        let status = StatusUiState::default();
        assert!(status.last_action.is_none());
        assert!(status.status_locations.is_empty());
        assert!(status.problematic_mod_id.is_none());
    }
}
