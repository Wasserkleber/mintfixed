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
#[path = "app/frame.rs"]
mod app_frame;
#[path = "mod_list/group_header.rs"]
mod group_header;
#[path = "mod_list/mod_list_style.rs"]
mod mod_list_style;
#[path = "mod_list/mod_tags.rs"]
mod mod_tags;
#[path = "mod_list/profile_ui.rs"]
mod profile_ui;
use group_controls::{Drag, Target};
#[path = "runtime/message.rs"]
mod message;
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

#[cfg(test)]
#[path = "tests/priority_input_tests.rs"]
mod priority_input_tests;
#[cfg(test)]
#[path = "tests/tests.rs"]
mod tests;

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

pub struct App {
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
    timestamp: Instant,
    status: LastActionStatus,
}
impl LastAction {
    fn success(msg: String) -> Self {
        Self {
            details: None,
            timestamp: Instant::now(),
            status: LastActionStatus::Success(msg),
        }
    }
    fn failure(msg: String) -> Self {
        Self {
            details: None,
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

    fn apply_mod_history(&mut self, redo: bool) {
        match self.edit_history.restore(&mut self.state.mod_data, redo) {
            Ok(true) => {
                self.profile_issues.clear();
                self.last_action = Some(LastAction::success(if redo {
                    "Redid mod-list change".into()
                } else {
                    "Undid mod-list change".into()
                }));
            }
            Ok(false) => {}
            Err(error) => self.last_action = Some(LastAction::failure(error)),
        }
    }
    fn show_group_mod_removal_confirmation(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.pending_group_mod_removal else {
            return;
        };

        let mut open = true;
        let mut confirmed = false;
        let mut cancelled = false;
        egui::Window::new("Remove mod from group?")
            .id(egui::Id::new("confirm_group_mod_removal"))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(format!(
                    "Remove '{}' from group '{}'?",
                    pending.mod_name, pending.group_name,
                ));
                ui.add_space(6.0);
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "⚠ Warning: this is a shared group.",
                );
                ui.label("This removes the mod from EVERY profile using this group.");
                ui.label("Other mods in the group will not be affected.");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancelled = true;
                    }
                    if ui
                        .add(Button::new("Remove from group").fill(colors::DARK_RED))
                        .clicked()
                    {
                        confirmed = true;
                    }
                });
            });

        if !open || cancelled || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.pending_group_mod_removal = None;
            return;
        }
        if !confirmed {
            return;
        }

        let pending = self.pending_group_mod_removal.take().unwrap();
        let before = self.state.mod_data.deref().deref().clone();
        let removed = group_controls::remove_group_member(
            self.state.mod_data.deref_mut().deref_mut(),
            &pending.group_name,
            &pending.mod_id,
        );

        if removed.is_err() {
            self.last_action = Some(LastAction::failure(
                "The mod or group changed before removal. Nothing was deleted.".into(),
            ));
            return;
        }
        match self.state.mod_data.save() {
            Ok(()) => {
                self.profile_issues.clear();
                self.last_action = Some(LastAction::success(format!(
                    "Removed '{}' from shared group '{}'",
                    pending.mod_name, pending.group_name,
                )));
            }
            Err(error) => {
                *self.state.mod_data.deref_mut().deref_mut() = before;
                self.last_action = Some(LastAction::failure(format!(
                    "Could not save group removal: {error}",
                )));
            }
        }
    }

    fn parse_mods(&self) -> Vec<ModSpecification> {
        self.resolve_mod
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .map(|l| ModSpecification::new(l.to_string()))
            .collect()
    }

    fn show_update_window(&mut self, ctx: &egui::Context) {
        if let (Some(update), Some(update_time)) =
            (self.available_update.as_ref(), self.show_update_time)
        {
            let now = SystemTime::now();
            let wait_time = Duration::from_secs(10);
            egui::Area::new("available-update-overlay".into())
                .movable(false)
                .fixed_pos(Pos2::ZERO)
                .order(egui::Order::Background)
                .show(ctx, |ui| {
                    egui::Frame::NONE
                        .fill(Color32::from_rgba_unmultiplied(0, 0, 0, 127))
                        .show(ui, |ui| {
                            ui.allocate_space(ui.available_size());
                        })
                });
            if let Some(MessageHandle { state, .. }) = &self.self_update_rid {
                egui::Window::new("Update progress")
                    .collapsible(false)
                    .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                    .resizable(false)
                    .show(ctx, |ui| {
                        ui.with_layout(egui::Layout::top_down_justified(Align::Center), |ui| {
                            match state {
                                SelfUpdateProgress::Pending => {
                                    ui.add(egui::ProgressBar::new(0.0).show_percentage());
                                }
                                SelfUpdateProgress::Progress { progress, size } => {
                                    ui.add(
                                        egui::ProgressBar::new(*progress as f32 / *size as f32)
                                            .show_percentage(),
                                    );
                                }
                                SelfUpdateProgress::Complete => {
                                    ui.add(egui::ProgressBar::new(1.0).show_percentage());
                                    ui.label(
                                        egui::RichText::new("Update successful.")
                                            .color(Color32::LIGHT_GREEN),
                                    );

                                    if ui.button("Restart").clicked() {
                                        self.needs_restart = true;
                                    }
                                }
                            };
                        });
                    });
            } else {
                egui::Window::new(format!("Update available: {}", update.tag_name))
                    .collapsible(false)
                    .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                    .resizable(false)
                    .show(ctx, |ui| {
                        CommonMarkViewer::new().max_image_width(Some(512)).show(
                            ui,
                            &mut self.cache,
                            &update.body,
                        );
                        ui.with_layout(egui::Layout::right_to_left(Align::TOP), |ui| {
                            if ui
                                .add(egui::Button::new("Install update"))
                                .on_hover_text("Download and install the update.")
                                .clicked()
                            {
                                self.self_update_rid = Some(message::SelfUpdate::send(
                                    &mut self.request_counter,
                                    self.tx.clone(),
                                    ctx.clone(),
                                ));
                            }

                            let elapsed = now.duration_since(update_time).unwrap_or_default();
                            if elapsed > wait_time {
                                if ui.button("Close").clicked() {
                                    self.show_update_time = None;
                                }
                            } else {
                                ui.spinner();
                            }
                        });
                    });
            }
        }
    }

    fn show_provider_parameters(&mut self, ctx: &egui::Context) {
        let Some(window) = &mut self.window_provider_parameters else {
            return;
        };

        while let Ok((rid, res)) = window.rx.try_recv() {
            if window.check_rid.as_ref().is_some_and(|r| rid == r.0) {
                match res {
                    Ok(()) => {
                        let window = self.window_provider_parameters.take().unwrap();
                        self.state
                            .config
                            .provider_parameters
                            .insert(window.factory.id.to_string(), window.parameters);
                        self.state.config.save().unwrap();
                        return;
                    }
                    Err(e) => {
                        window.check_error = Some(e.to_string());
                    }
                }
                window.check_rid = None;
            }
        }

        let mut open = true;
        let mut check = false;
        egui::Window::new(format!("Configure {} provider", window.factory.id))
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| {
                ui.add_enabled_ui(window.check_rid.is_none(), |ui| {
                    egui::Grid::new("grid").num_columns(2).show(ui, |ui| {
                        for p in window.factory.parameters {
                            if let Some(link) = p.link {
                                ui.hyperlink_to(p.name, link).on_hover_text(p.description);
                            } else {
                                ui.label(p.name).on_hover_text(p.description);
                            }
                            let res = ui.add(
                                egui::TextEdit::singleline(
                                    window.parameters.entry(p.id.to_string()).or_default(),
                                )
                                .password(true)
                                .desired_width(200.0),
                            );
                            if is_committed(&res) {
                                check = true;
                            }
                            ui.end_row();
                        }
                    });

                    ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
                        if ui.button("Save").clicked() {
                            check = true;
                        }
                        if window.check_rid.is_some() {
                            ui.spinner();
                        }
                        if let Some(error) = &window.check_error {
                            ui.colored_label(ui.visuals().error_fg_color, error);
                        }
                    });
                });
            });
        if !open {
            self.window_provider_parameters = None;
        } else if check {
            window.check_error = None;
            let tx = window.tx.clone();
            let ctx = ctx.clone();
            let rid = self.request_counter.next();
            let store = self.state.store.clone();
            let params = window.parameters.clone();
            let factory = window.factory;
            let handle = tokio::task::spawn(async move {
                let res = store.add_provider_checked(factory, &params).await;
                tx.send((rid, res)).await.unwrap();
                ctx.request_repaint();
            });
            window.check_rid = Some((rid, handle));
        }
    }

    fn show_profile_windows(&mut self, ctx: &egui::Context) {
        let mut to_remove = vec![];
        for profile in &self.open_profiles.clone() {
            let mut open = true;
            egui::Window::new(format!("Profile \"{profile}\""))
                .open(&mut open)
                .show(ctx, |ui| {
                    self.ui_profile(ui, profile);
                });
            if !open {
                to_remove.push(profile.clone());
            }
        }
        for r in to_remove {
            self.open_profiles.remove(&r);
        }
    }

    fn show_settings(&mut self, ctx: &egui::Context) {
        let mut auto_save_local_folder: Option<(String, bool)> = None;
        let mut rescan_folder = false;
        if let Some(window) = &mut self.settings_window {
            let mut open = true;
            let mut try_save = false;
            egui::Window::new("Settings")
                .open(&mut open)
                .resizable(false)
                .show(ctx, |ui| {
                    egui::Grid::new("grid").num_columns(2).striped(true).show(ui, |ui| {
                        let mut job = LayoutJob::default();
                        job.append(
                            "DRG pak",
                            0.0,
                            TextFormat {
                                color: ui.visuals().text_color(),
                                underline: Stroke::new(1.0_f32, ui.visuals().text_color()),
                                ..Default::default()
                            },
                        );
                        ui.label(job).on_hover_cursor(egui::CursorIcon::Help).on_hover_text("Path to FSD-WindowsNoEditor.pak (FSD-WinGDK.pak for Microsoft Store version)\nLocated inside the \"Deep Rock Galactic\" installation directory under FSD/Content/Paks.");
                        ui.horizontal(|ui| {
                            let res = ui.add(
                                egui::TextEdit::singleline(
                                    &mut window.drg_pak_path
                                )
                                .desired_width(200.0),
                            );
                            if res.changed() {
                                window.drg_pak_path_err = None;
                            }
                            if is_committed(&res) {
                                try_save = true;
                            }
                            if ui.button("browse").clicked()
                                && let Some(fsd_pak) = rfd::FileDialog::new()
                                    .add_filter("DRG Pak", &["pak"])
                                    .pick_file()
                                {
                                    window.drg_pak_path = fsd_pak.to_string_lossy().to_string();
                                    window.drg_pak_path_err = None;
                                }
                        });
                        ui.end_row();

                        ui.label("Local mods folder:")
                            .on_hover_text("Choose a local mods folder. Use Import .pak/.zip beside Add mod to scan it; only explicit exclusions stay hidden.");
                        ui.horizontal(|ui| {
                            let res = ui.add(egui::TextEdit::singleline(&mut window.local_mod_folder)
                                .desired_width(200.0)
                                .hint_text("Choose a mods folder..."));
                            if res.changed() {
                                window.local_mod_folder_err = None;
                                auto_save_local_folder = Some((window.local_mod_folder.clone(), false));
                            }
                            if is_committed(&res) {
                                auto_save_local_folder = Some((window.local_mod_folder.clone(), true));
                            }
                            if ui.button("browse").clicked()
                                && let Some(folder) = rfd::FileDialog::new().pick_folder()
                            {
                                window.local_mod_folder = folder.display().to_string();
                                window.local_mod_folder_err = None;
                                auto_save_local_folder = Some((window.local_mod_folder.clone(), true));
                            }
                            if let Some(error) = &window.local_mod_folder_err {
                                ui.colored_label(ui.visuals().error_fg_color, error);
                            }
                        });
                        ui.end_row();
                        let config_dir = &self.state.dirs.config_dir;
                        ui.label("Config directory:");
                        if ui.link(config_dir.display().to_string()).clicked() {
                            opener::open(config_dir).ok();
                        }
                        ui.end_row();

                        let cache_dir = &self.state.dirs.cache_dir;
                        ui.label("Cache directory:");
                        if ui.link(cache_dir.display().to_string()).clicked() {
                            opener::open(cache_dir).ok();
                        }
                        ui.end_row();

                        let data_dir = &self.state.dirs.data_dir;
                        ui.label("Data directory:");
                        if ui.link(data_dir.display().to_string()).clicked() {
                            opener::open(data_dir).ok();
                        }
                        ui.end_row();

                        ui.label("GUI theme:");
                        ui.horizontal(|ui| {
                            ui.horizontal(|ui| {
                                let config = &mut self.state.config;

                                let old_theme = GuiTheme::into_egui_theme(config.gui_theme);
                                let mut theme = old_theme;
                                theme.radio_buttons(ui);
                                if theme != old_theme {
                                    ui.memory_mut(|m| m.options.theme_preference = theme);
                                    config.gui_theme = GuiTheme::from_egui_theme(theme);
                                    config.save().unwrap();
                                }
                            });
                        });
                        ui.end_row();

                        ui.label("Bundled crash reporter:");
                        let config = &mut self.state.config;
                        if ui
                            .add(toggle_switch(&mut config.bundled_crash_reporter))
                            .on_hover_text("Install and autolaunch the pinned BP-Crash-Reporter. Turn off to run your own; applied on the next Install.")
                            .changed()
                        {
                            config.save().unwrap();
                        }
                        ui.end_row();

                        ui.label("Mod providers:");
                        ui.end_row();

                        for provider_factory in ModStore::get_provider_factories() {
                            ui.label(provider_factory.id);
                            if ui.add_enabled(!provider_factory.parameters.is_empty(), egui::Button::new("⚙"))
                                    .on_hover_text(format!("Open \"{}\" settings", provider_factory.id))
                                    .clicked() {
                                self.window_provider_parameters = Some(
                                    WindowProviderParameters::new(provider_factory, &self.state),
                                );
                            }
                            ui.end_row();
                        }
                    });

                    ui.with_layout(egui::Layout::right_to_left(Align::TOP), |ui| {
                        if ui.add_enabled(window.drg_pak_path_err.is_none(), egui::Button::new("save")).clicked() {
                            try_save = true;
                        }
                        if let Some(error) = &window.drg_pak_path_err {
                            ui.colored_label(ui.visuals().error_fg_color, error);
                        }
                    });

                });
            if try_save {
                let folder_text = window.local_mod_folder.trim();
                let folder_result: std::io::Result<Option<PathBuf>> = if folder_text.is_empty() {
                    Ok(None)
                } else {
                    std::fs::canonicalize(folder_text).and_then(|path| {
                        if path.is_dir() {
                            Ok(Some(path))
                        } else {
                            Err(std::io::Error::other("Local mods path is not a folder"))
                        }
                    })
                };
                if let Err(error) = &folder_result {
                    window.local_mod_folder_err = Some(error.to_string());
                } else {
                    if let Err(e) = is_drg_pak(&window.drg_pak_path) {
                        window.drg_pak_path_err = Some(e.to_string());
                    } else {
                        self.state.config.drg_pak_path = Some(PathBuf::from(
                            self.settings_window.take().unwrap().drg_pak_path,
                        ));
                        self.state.config.local_mod_folder = folder_result.ok().flatten();
                        rescan_folder = true;
                        self.state.config.save().unwrap();
                    }
                }
            } else if !open {
                self.settings_window = None;
            }
        }
        // Folder selection is independent of the DRG pak Save button.
        // Save immediately for valid paths (or clearing the field), and on Browse.
        if let Some((folder, committed)) = auto_save_local_folder {
            if folder.trim().is_empty() || PathBuf::from(folder.trim()).is_dir() {
                match folder_import::save_folder_setting(&mut self.state, &folder) {
                    Ok(changed) => {
                        if changed {
                            folder_import::reset_schedule(self);
                        }
                    }
                    Err(error) => {
                        if let Some(window) = &mut self.settings_window {
                            window.local_mod_folder_err = Some(error.clone());
                        }
                        self.last_action = Some(LastAction::failure(error));
                    }
                }
            } else if committed && let Some(window) = &mut self.settings_window {
                window.local_mod_folder_err = Some(format!("Not a folder: {}", folder.trim()));
            }
        }
        if rescan_folder {
            folder_import::reset_schedule(self);
        }
    }

    fn get_sorting_config(&self) -> Option<SortingConfig> {
        self.state.config.sorting_config.clone()
    }

    fn update_sorting_config(&mut self, sort_category: Option<SortBy>, is_ascending: bool) {
        self.state.config.sorting_config = sort_category.map(|sort_category| SortingConfig {
            sort_category,
            is_ascending,
        });
        self.state.config.save().unwrap();
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
