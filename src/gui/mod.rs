#[path = "bitfix/bitfix_menu.rs"]
mod bitfix_menu;
#[path = "crashes/crash_history.rs"]
mod crash_history;
#[path = "crashes/crash_notice.rs"]
mod crash_notice;
#[cfg(all(feature = "hook", target_os = "windows"))]
#[path = "crashes/crash_reporter_update.rs"]
mod crash_reporter_update;
#[path = "mod_list/edit_history.rs"]
mod edit_history;
#[path = "mod_list/find_string.rs"]
mod find_string;
#[path = "imports/folder_import.rs"]
mod folder_import;
mod group_controls;
#[path = "widgets/icon_buttons.rs"]
mod icon_buttons;
mod lint;
use self::lint::LintOptions;
#[path = "app/begin_frame.rs"]
mod app_begin_frame;
#[path = "app/frame.rs"]
mod app_frame;
#[path = "app/install_preflight.rs"]
mod app_install_preflight;
#[path = "app/main_panel.rs"]
mod app_main_panel;
#[path = "app/overlays.rs"]
mod app_overlays;
#[path = "app/status_details.rs"]
mod app_status_details;
#[path = "app/toolbar.rs"]
mod app_toolbar;
#[path = "mod_list/group_drop_target.rs"]
mod group_drop_target;
#[path = "mod_list/group_header.rs"]
mod group_header;
#[path = "mod_list/mod_list_style.rs"]
mod mod_list_style;
#[path = "mod_list/mod_tags.rs"]
mod mod_tags;
#[path = "mod_list/profile_ui.rs"]
mod profile_ui;
#[path = "dialogs/replacement_prompt.rs"]
mod replacement_prompt;
use group_controls::{Drag, Target};
#[path = "runtime/message.rs"]
mod message;
#[path = "runtime/missing_local_files.rs"]
mod missing_local_files;
#[path = "widgets/named_combobox.rs"]
mod named_combobox;
#[path = "profiles/profile_import.rs"]
mod profile_import;
#[path = "profiles/profile_management.rs"]
mod profile_management;
#[path = "runtime/request_counter.rs"]
mod request_counter;
#[path = "widgets/toggle_switch.rs"]
mod toggle_switch;
#[path = "runtime/unavailable_mods.rs"]
mod unavailable_mods;

#[cfg(test)]
#[path = "tests/priority_input_tests.rs"]
mod priority_input_tests;
#[cfg(test)]
#[path = "tests/tests.rs"]
mod tests;
#[cfg(test)]
#[path = "tests/unavailable_mods.rs"]
mod unavailable_mod_tests;

//#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::{Deref, RangeInclusive};
use std::time::{Duration, Instant, SystemTime};
use std::{
    collections::{HashMap, HashSet},
    ops::DerefMut,
    path::PathBuf,
};

use eframe::egui::{Button, CollapsingHeader, RichText};
use eframe::epaint::{Pos2, Vec2};
use eframe::{
    egui::{FontSelection, Layout, TextFormat, Ui},
    emath::{Align, Align2},
    epaint::{Color32, Stroke, text::LayoutJob},
};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use mint_lib::error::ResultExt as _;
use mint_lib::mod_info::{ModioTags, RequiredStatus};
use mint_lib::update::GitHubRelease;
use strum::{EnumIter, IntoEnumIterator};
use tokio::{
    sync::mpsc::{self, Receiver, Sender},
    task::JoinHandle,
};
use tracing::{debug, trace};

use crate::Dirs;
use crate::gui::find_string::searchable_text;
use crate::mod_lints::{LintId, LintReport, SplitAssetPair};
use crate::providers::ProviderError;
use crate::state::{MemberOverrides, SortingConfig};
use crate::{
    MintError,
    integrate::uninstall,
    is_drg_pak,
    providers::{
        ApprovalStatus, FetchProgress, ModInfo, ModSpecification, ModStore, ProviderFactory,
    },
    state::{ModConfig, ModData_v0_1_0 as ModData, ModOrGroup, ModProfile, State},
};
use message::MessageHandle;
use request_counter::{RequestCounter, RequestID};

use self::toggle_switch::toggle_switch;

/// A profile's issues, keyed by the revisions they were computed against.
type ProfileIssuesEntry = ((u64, (u64, u64)), crate::mod_identity::ProfileIssues);

pub fn gui(dirs: Dirs, args: Option<Vec<String>>) -> Result<(), MintError> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([900.0, 500.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        mint_lib::built_info::version(),
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc, dirs, args)?))),
    )
    .with_generic(|e| format!("{e}"))?;
    Ok(())
}

pub mod colors {
    use eframe::epaint::Color32;

    pub const DARK_RED: Color32 = Color32::DARK_RED;
    pub const DARKER_RED: Color32 = Color32::from_rgb(110, 0, 0);

    pub const DARK_GREEN: Color32 = Color32::DARK_GREEN;
    pub const DARKER_GREEN: Color32 = Color32::from_rgb(0, 80, 0);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GuiTheme {
    Light,
    Dark,
}

impl GuiTheme {
    fn from_egui_theme(theme: egui::ThemePreference) -> Option<Self> {
        match theme {
            egui::ThemePreference::Dark => Some(GuiTheme::Dark),
            egui::ThemePreference::Light => Some(GuiTheme::Light),
            egui::ThemePreference::System => None,
        }
    }
    fn into_egui_theme(theme: Option<Self>) -> egui::ThemePreference {
        match theme {
            Some(GuiTheme::Dark) => egui::ThemePreference::Dark,
            Some(GuiTheme::Light) => egui::ThemePreference::Light,
            None => egui::ThemePreference::System,
        }
    }
}

#[derive(PartialEq, Debug, EnumIter, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum SortBy {
    Enabled,
    Name,
    Priority,
    Provider,
    RequiredStatus,
    ApprovalCategory,
}

impl SortBy {
    fn as_str(&self) -> &'static str {
        match self {
            SortBy::Enabled => "Enabled",
            SortBy::Name => "Name",
            SortBy::Priority => "Priority",
            SortBy::Provider => "Provider",
            SortBy::RequiredStatus => "Is Required",
            SortBy::ApprovalCategory => "Approval",
        }
    }
}

/// Collect unavailable group member names even when the group or member is disabled.
/// Returning owned names avoids retaining group borrows while the UI edits them.
fn unavailable_group_mod_names(
    mods: &[ModConfig],
    mut is_unavailable: impl FnMut(&ModSpecification) -> bool,
    mut display_name: impl FnMut(&ModSpecification) -> String,
) -> Vec<String> {
    mods.iter()
        .filter(|member| is_unavailable(&member.spec))
        .map(|member| display_name(&member.spec))
        .collect()
}

const MODIO_LOGO_PNG: &[u8] = include_bytes!("../../assets/icons/modio-cog-blue.png");

// A bordered, hover-only help control for the optional preset description.
// Using a real widget makes the tooltip discoverable without triggering actions.
// Keep the displayed priority stable while editing so sort-by-priority cannot
// move a row or interrupt multi-digit keyboard input after the first digit.
// egui 0.31 keeps the text of a focused DragValue in temp data under its widget ID.
// The stored priority must stay unchanged while typing (to avoid resorting rows),
// but the red/green/neutral text tint should follow the pending edit immediately.
fn priority_preview_value(ui: &Ui, committed: i32) -> i32 {
    let id = ui.next_auto_id(); // the ID of the immediately following DragValue
    if !ui.memory(|memory| memory.has_focus(id)) {
        return committed;
    }
    ui.data(|data| data.get_temp::<String>(id))
        .and_then(|text| {
            let text = text.trim();
            if text.is_empty() || text == "-" {
                Some(0)
            } else {
                text.parse::<i32>().ok()
            }
        })
        .unwrap_or(committed)
}

fn priority_drag_value(ui: &mut Ui, priority: &mut i32) -> egui::Response {
    let response = ui.add(
        egui::DragValue::new(priority)
            .custom_formatter(|n, _| {
                if n == 0. {
                    "-".to_string()
                } else {
                    format!("{n}")
                }
            })
            .custom_parser(|text| {
                let text = text.trim();
                if text.is_empty() || text == "-" {
                    Some(0.0)
                } else {
                    text.parse::<f64>().ok()
                }
            })
            .update_while_editing(false)
            .speed(0.05)
            .range(RangeInclusive::new(-999, 999)),
    );
    // One extra frame picks up the most recently typed text from egui's
    // DragValue buffer, without committing it or causing the list to resort.
    if response.has_focus() && response.changed() {
        ui.ctx().request_repaint();
    }
    response
}

fn description_help_button(ui: &mut Ui, description: &str) {
    let border = ui.visuals().widgets.inactive.fg_stroke.color;
    ui.add(
        Button::new(RichText::new("?").strong())
            .small()
            .frame(true)
            .stroke(Stroke::new(1.0_f32, border))
            .min_size(egui::vec2(18.0, 18.0))
            .sense(egui::Sense::hover()),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
    .on_hover_ui_at_pointer(|ui| {
        ui.set_max_width(320.0);
        ui.label(RichText::new("Description").strong());
        ui.label(description);
    });
}

// A single-line, width-bounded status label. Never let diagnostics reserve toolbar space.
// Hover exposes the unabridged message; clicking opens the copyable details window.
fn status_message(ui: &mut Ui, message: &str) -> bool {
    let width = ui.available_width();
    if width < 18.0 {
        return false;
    }
    // A frameless Button keeps the text left-aligned and truncates without
    // Label's automatic tooltip (egui 0.31), avoiding duplicate hover popups.
    ui.add(
        Button::new(RichText::new(message).text_style(egui::TextStyle::Body))
            .frame(false)
            .truncate(),
    )
    .on_hover_ui_at_pointer(|ui| {
        ui.set_max_width(500.0);
        ui.label(message);
        ui.small("Click to open and copy the complete message");
    })
    .on_hover_cursor(egui::CursorIcon::PointingHand)
    .clicked()
}

struct PendingReplacementAddition {
    specs: Vec<ModSpecification>,
    is_dependency: bool,
    profile_id: String,
    decided: HashSet<String>,
}

pub struct App {
    pending_replacement_addition: Option<PendingReplacementAddition>,
    crash_history: crash_history::CrashHistory,
    bitfix_menu: bitfix_menu::BitfixMenu,
    args: Option<Vec<String>>,
    tx: Sender<message::Message>,
    rx: Receiver<message::Message>,
    state: State,
    edit_history: edit_history::ModEditHistory,
    profile_issues: HashMap<String, ProfileIssuesEntry>,
    groups_to_open: HashSet<(String, String)>,
    new_group_name: String,
    pending_group_files: usize,
    resolve_mod: String,
    resolve_mod_rid: Option<MessageHandle<()>>,
    integrate_rid: Option<MessageHandle<HashMap<ModSpecification, SpecFetchProgress>>>,
    // Tracks only installations started by Update Cache, not manual installations.
    cache_install_rid: Option<RequestID>,
    cache_install_notice: Option<String>,
    cache_install_details: Option<String>,
    cache_install_locations: Vec<unavailable_mods::Location>,
    install_phase: message::InstallPhase,
    cache_recovery_progress: Option<(usize, usize)>,
    cache_recovery_last_advance: Option<std::time::Instant>,
    update_rid: Option<MessageHandle<()>>,
    check_updates_rid: Option<MessageHandle<()>>,
    has_run_init: bool,
    last_ui_frame_start: Option<Instant>,
    request_counter: RequestCounter,
    window_provider_parameters: Option<WindowProviderParameters>,
    search_string: String,
    scroll_to_match: bool,
    focus_search: bool,
    settings_window: Option<WindowSettings>,
    folder_import: folder_import::FolderImport,
    modio_texture_handle: Option<egui::TextureHandle>,
    last_action: Option<LastAction>,
    status_details: Option<String>,
    status_locations: Vec<unavailable_mods::Location>,
    unavailable_overview: unavailable_mods::Overview,
    live_unavailable_details: bool,
    show_mod_target: Option<unavailable_mods::ResolvedTarget>,
    available_update: Option<GitHubRelease>,
    show_update_time: Option<SystemTime>,
    open_profiles: HashSet<String>,
    lint_rid: Option<MessageHandle<()>>,
    lint_report_window: Option<WindowLintReport>,
    lint_report: Option<LintReport>,
    lints_toggle_window: Option<WindowLintsToggle>,
    lint_options: LintOptions,
    lint_scope: lint::LintScope,
    lint_selected_mods: BTreeSet<ModSpecification>,
    lint_focus: Option<ModSpecification>,
    lint_filter: String,
    lint_focus_filter: String,
    lint_run: Option<lint::LintRun>,
    lint_report_path: Option<PathBuf>,
    lint_report_error: Option<String>,
    cache: CommonMarkCache,
    needs_restart: bool,
    self_update_rid: Option<MessageHandle<SelfUpdateProgress>>,
    original_exe_path: Option<PathBuf>,
    problematic_mod_id: Option<u32>,
    pending_group_mod_removal: Option<PendingGroupModRemoval>,
    recent_crash: Option<crash_notice::RecentCrash>,
    #[cfg(all(feature = "hook", target_os = "windows"))]
    startup_crash_reporter_update: crash_reporter_update::StartupUpdate,
    crash_monitor: Option<crash_notice::CrashMonitor>,
}

struct LastAction {
    details: Option<String>,
    unavailable_locations: Vec<unavailable_mods::Location>,
    timestamp: Instant,
    status: LastActionStatus,
}
impl LastAction {
    fn success(msg: String) -> Self {
        Self {
            details: None,
            unavailable_locations: Vec::new(),
            timestamp: Instant::now(),
            status: LastActionStatus::Success(msg),
        }
    }
    fn failure(msg: String) -> Self {
        Self {
            details: None,
            unavailable_locations: Vec::new(),
            timestamp: Instant::now(),
            status: LastActionStatus::Failure(msg),
        }
    }
    fn timeago(&self) -> String {
        let duration = Instant::now().duration_since(self.timestamp);
        let seconds = duration.as_secs();
        if seconds < 60 {
            format!("{seconds}s ago")
        } else if seconds < 3600 {
            format!("{}m ago", seconds / 60)
        } else {
            ">1h ago".into()
        }
    }
}

enum LastActionStatus {
    Success(String),
    Failure(String),
}

// Identify a group member by its stable entry ID, not its current row position.
struct PendingGroupModRemoval {
    group_name: String,
    mod_id: String,
    mod_name: String,
}

impl App {
    fn new(
        cc: &eframe::CreationContext,
        dirs: Dirs,
        args: Option<Vec<String>>,
    ) -> Result<Self, MintError> {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let (tx, rx) = mpsc::channel(10);
        let state = State::init(dirs)?;
        let recent_crash = crash_notice::find_recent_unseen(&state);
        #[cfg(all(feature = "hook", target_os = "windows"))]
        let startup_crash_reporter_update =
            crash_reporter_update::StartupUpdate::start(&state, &cc.egui_ctx);
        let specs = crate::mod_identity::all_specs(&state.mod_data);
        if !specs.is_empty() {
            let store = state.store.clone();
            let ctx = cc.egui_ctx.clone();
            std::thread::spawn(move || {
                store.refresh_local_files(&specs);
                ctx.request_repaint();
            });
        }

        let edit_history =
            edit_history::ModEditHistory::new(&state.mod_data, state.mod_data.revision());
        Ok(Self {
            pending_replacement_addition: None,
            crash_history: crash_history::CrashHistory::default(),
            bitfix_menu: bitfix_menu::BitfixMenu::default(),
            args,
            tx,
            rx,
            request_counter: Default::default(),
            state,
            edit_history,
            profile_issues: HashMap::new(),
            groups_to_open: HashSet::new(),
            new_group_name: String::new(),
            pending_group_files: 0,
            resolve_mod: Default::default(),
            resolve_mod_rid: None,
            integrate_rid: None,
            cache_install_rid: None,
            cache_install_notice: None,
            cache_install_details: None,
            cache_install_locations: Vec::new(),
            install_phase: message::InstallPhase::Resolving,
            cache_recovery_progress: None,
            cache_recovery_last_advance: None,
            update_rid: None,
            check_updates_rid: None,
            has_run_init: false,
            last_ui_frame_start: None,
            window_provider_parameters: None,
            search_string: Default::default(),
            scroll_to_match: false,
            focus_search: false,
            settings_window: None,
            folder_import: Default::default(),
            modio_texture_handle: None,
            last_action: None,
            status_details: None,
            status_locations: Vec::new(),
            unavailable_overview: Default::default(),
            live_unavailable_details: false,
            show_mod_target: None,
            available_update: None,
            show_update_time: None,
            open_profiles: Default::default(),
            lint_rid: None,
            lint_report_window: None,
            lint_report: None,
            lints_toggle_window: None,
            lint_options: LintOptions::default(),
            lint_scope: lint::LintScope::All,
            lint_selected_mods: BTreeSet::new(),
            lint_focus: None,
            lint_filter: String::new(),
            lint_focus_filter: String::new(),
            lint_run: None,
            lint_report_path: None,
            lint_report_error: None,
            cache: Default::default(),
            needs_restart: false,
            self_update_rid: None,
            original_exe_path: None,
            problematic_mod_id: None,
            pending_group_mod_removal: None,
            recent_crash,
            #[cfg(all(feature = "hook", target_os = "windows"))]
            startup_crash_reporter_update,
            crash_monitor: None,
        })
    }
}

fn sorted_mod_indices<'a>(
    overrides: Option<&MemberOverrides>,
    mods: impl IntoIterator<Item = Option<&'a ModConfig>>,
    config: Option<&SortingConfig>,
    get_info: impl Fn(&ModSpecification) -> Option<ModInfo>,
) -> Vec<usize> {
    let mut entries: Vec<_> = mods
        .into_iter()
        .enumerate()
        .map(|(index, mc)| {
            let info = config.and_then(|_| mc.and_then(|mc| get_info(&mc.spec)));
            (index, mc, info)
        })
        .collect();
    if let Some(config) = config {
        let compare = sort_mods(config, overrides);
        // Group entries separate independently sorted runs and retain their positions.
        for run in entries.split_mut(|(_, mc, _)| mc.is_none()) {
            run.sort_by(|(_, a, info_a), (_, b, info_b)| {
                compare((a.unwrap(), info_a.as_ref()), (b.unwrap(), info_b.as_ref()))
            });
        }
    }
    entries.into_iter().map(|(index, _, _)| index).collect()
}

type ModListEntry<'a> = (&'a ModConfig, Option<&'a ModInfo>);
fn sort_mods<'a>(
    config: &'a SortingConfig,
    overrides: Option<&'a MemberOverrides>,
) -> impl Fn(ModListEntry, ModListEntry) -> Ordering + 'a {
    move |(mc_a, info_a), (mc_b, info_b)| {
        fn map_cmp<V, M, F>(a: &V, b: &V, map: F) -> Ordering
        where
            M: Ord,
            F: Fn(&V) -> M,
        {
            map(a).cmp(&map(b))
        }

        let name_order = map_cmp(&(mc_a, info_a), &(mc_b, info_b), |(mc, info)| {
            (info.map(|i| i.name.to_lowercase()), &mc.spec.url)
        });
        let provider_order = map_cmp(&info_a, &info_b, |info| info.map(|i| i.provider));
        let approval_order = map_cmp(&info_a, &info_b, |info| {
            info.and_then(|i| i.modio_tags.as_ref())
                .map(|t| t.approval_status)
        });
        let required_order = map_cmp(&info_a, &info_b, |info| {
            info.and_then(|i| i.modio_tags.as_ref())
                .map(|t| std::cmp::Reverse(t.required_status))
        });
        let mut order = match config.sort_category {
            SortBy::Enabled => {
                let enabled = |mc: &ModConfig| overrides.map_or(mc.enabled, |o| o.enabled(mc));
                enabled(mc_b).cmp(&enabled(mc_a))
            }
            SortBy::Name => name_order,
            SortBy::Priority => mc_a.priority.cmp(&mc_b.priority),
            SortBy::Provider => provider_order,
            SortBy::RequiredStatus => required_order,
            SortBy::ApprovalCategory => approval_order,
        };

        if config.is_ascending {
            order = order.reverse();
        }
        if config.sort_category != SortBy::Name {
            order = order.then(name_order);
        }
        order
    }
}

struct WindowProviderParameters {
    tx: Sender<(RequestID, Result<(), ProviderError>)>,
    rx: Receiver<(RequestID, Result<(), ProviderError>)>,
    check_rid: Option<(RequestID, JoinHandle<()>)>,
    check_error: Option<String>,
    factory: &'static ProviderFactory,
    parameters: HashMap<String, String>,
}

impl WindowProviderParameters {
    fn new(factory: &'static ProviderFactory, state: &State) -> Self {
        let (tx, rx) = mpsc::channel(10);
        Self {
            tx,
            rx,
            check_rid: None,
            check_error: None,
            parameters: state
                .config
                .provider_parameters
                .get(factory.id)
                .cloned()
                .unwrap_or_default(),
            factory,
        }
    }
}

struct WindowSettings {
    drg_pak_path: String,
    drg_pak_path_err: Option<String>,
    local_mod_folder: String,
    local_mod_folder_err: Option<String>,
}

impl WindowSettings {
    fn new(state: &State) -> Self {
        let path = state
            .config
            .drg_pak_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        Self {
            drg_pak_path: path,
            drg_pak_path_err: None,
            local_mod_folder: state
                .config
                .local_mod_folder
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            local_mod_folder_err: None,
        }
    }
}

struct WindowLintReport;

struct WindowLintsToggle;

// Clipboard JSON is structured data, not a newline-delimited list of mod URLs.
// Catch incomplete JSON too, so an accidental paste doesn't spam provider errors.
fn looks_like_structured_json(text: &str) -> bool {
    let text = text.trim_start();
    text.starts_with('{') || text.starts_with('[')
}

fn is_committed(res: &egui::Response) -> bool {
    res.lost_focus() && res.ctx.input(|i| i.key_pressed(egui::Key::Enter))
}

#[derive(Debug, Clone)]
pub enum SpecFetchProgress {
    Progress { progress: u64, size: u64 },
    Complete,
}

impl From<FetchProgress> for SpecFetchProgress {
    fn from(value: FetchProgress) -> Self {
        match value {
            FetchProgress::Progress { progress, size, .. } => Self::Progress { progress, size },
            FetchProgress::Complete { .. } => Self::Complete,
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum SelfUpdateProgress {
    Pending,
    Progress { progress: u64, size: u64 },
    Complete,
}

#[cfg(test)]
mod structured_json_paste_tests {
    use super::looks_like_structured_json;

    #[test]
    fn rejects_profile_json_before_mod_resolution() {
        assert!(looks_like_structured_json(
            "{\n  \"format\": \"mintfixed-profile-mod-list\",\n  \"id\": \"c91ff9fb9d8bfb6192777bdcccc6be60\"\n}"
        ));
        assert!(looks_like_structured_json(
            "  [\n  \"https://mod.io/g/drg/m/example\"\n]"
        ));
    }

    #[test]
    fn accepts_regular_mod_lists() {
        assert!(!looks_like_structured_json(
            "https://mod.io/g/drg/m/example#123"
        ));
        assert!(!looks_like_structured_json("C:\\mods\\example.pak"));
        assert!(!looks_like_structured_json(
            "https://mod.io/g/drg/m/example\nC:\\mods\\example.pak"
        ));
    }
}

// Stage 2F: separated GUI windows and list actions.
#[path = "dialogs/group_removal.rs"]
mod gui_group_removal_dialog;
#[path = "mod_list/actions.rs"]
mod gui_mod_list_actions;
#[path = "profiles/windows.rs"]
mod gui_profile_windows;
#[path = "settings/windows.rs"]
mod gui_settings_windows;
#[path = "dialogs/update.rs"]
mod gui_update_dialog;
