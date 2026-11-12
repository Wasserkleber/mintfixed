mod bitfix_menu;
mod crash_history;
mod crash_notice;
mod edit_history;
mod find_string;
mod folder_import;
mod group_controls;
mod icon_buttons;
mod mod_list_style;
use group_controls::{Drag, Target};
mod message;
mod named_combobox;
mod profile_import;
mod profile_management;
mod request_counter;
mod toggle_switch;

#[cfg(test)]
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

const MODIO_LOGO_PNG: &[u8] = include_bytes!("../../assets/icons/modio-cog-blue.png");

// A bordered, hover-only help control for the optional preset description.
// Using a real widget makes the tooltip discoverable without triggering actions.
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
    cache: CommonMarkCache,
    needs_restart: bool,
    self_update_rid: Option<MessageHandle<SelfUpdateProgress>>,
    original_exe_path: Option<PathBuf>,
    problematic_mod_id: Option<u32>,
    pending_group_mod_removal: Option<PendingGroupModRemoval>,
    recent_crash: Option<crash_notice::RecentCrash>,
    crash_monitor: Option<crash_notice::CrashMonitor>,
}

#[derive(Default)]
struct LintOptions {
    archive_with_multiple_paks: bool,
    archive_with_only_non_pak_files: bool,
    asset_register_bin: bool,
    conflicting: bool,
    empty_archive: bool,
    outdated_pak_version: bool,
    shader_files: bool,
    non_asset_files: bool,
    split_asset_pairs: bool,
    unmodified_game_assets: bool,
}

struct LastAction {
    timestamp: Instant,
    status: LastActionStatus,
}
impl LastAction {
    fn success(msg: String) -> Self {
        Self {
            timestamp: Instant::now(),
            status: LastActionStatus::Success(msg),
        }
    }
    fn failure(msg: String) -> Self {
        Self {
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
            cache: Default::default(),
            needs_restart: false,
            self_update_rid: None,
            original_exe_path: None,
            problematic_mod_id: None,
            pending_group_mod_removal: None,
            recent_crash,
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
    fn ui_profile(&mut self, ui: &mut Ui, profile: &str) -> Option<(String, bool)> {
        if egui::DragAndDrop::payload::<Drag>(ui.ctx()).is_some() {
            ui.style_mut().interaction.selectable_labels = false;
        }
        let profile_name = profile;
        let mut sorting_config = self.get_sorting_config();
        if sorting_config.is_some() && ui.button("Switch to Manual to move mods").clicked() {
            self.update_sorting_config(None, false);
            sorting_config = None;
        }

        let revision = (
            self.state.mod_data.revision(),
            self.state.store.metadata_revision(),
        );
        if self.profile_issues.get(profile).map(|entry| entry.0) != Some(revision) {
            let issues = crate::mod_identity::ProfileIssues::collect(
                &self.state.mod_data,
                profile,
                &self.state.store,
            );
            self.profile_issues
                .insert(profile.to_owned(), (revision, issues));
        }
        let issues = &self.profile_issues[profile].1;
        for warning in &issues.versions {
            ui.colored_label(ui.visuals().warn_fg_color, warning);
        }
        if !issues.pending.is_empty() {
            ui.label(format!(
                "{} mod.io entries awaiting version metadata",
                issues.pending.len()
            ))
            .on_hover_text(issues.pending.join("\n"));
        }

        let ModData {
            profiles, groups, ..
        } = self.state.mod_data.deref_mut().deref_mut();

        struct Ctx {
            needs_save: bool,
            scroll_to_match: bool,
            btn_remove: Option<(Option<String>, usize)>,
            add_deps: Option<Vec<ModSpecification>>,
            drag: Option<(Drag, Target)>,
            file_group: Option<(String, bool)>,
        }
        let mut ctx = Ctx {
            needs_save: false,
            scroll_to_match: self.scroll_to_match,
            btn_remove: None,
            add_deps: None,
            drag: None,
            file_group: None,
        };

        let mut ui_profile = |ui: &mut Ui, profile: &mut ModProfile| {
            let enabled_specs = profile
                .mods
                .iter()
                .enumerate()
                .flat_map(|(i, m)| -> Box<dyn Iterator<Item = _>> {
                    match m {
                        ModOrGroup::Individual(mc) => Box::new(
                            mc.enabled
                                .then_some(((None, i), mc.spec.clone()))
                                .into_iter(),
                        ),
                        ModOrGroup::Group {
                            group_name,
                            enabled,
                            member_overrides,
                            ..
                        } => Box::new(
                            enabled
                                .then(|| groups.get(group_name))
                                .flatten()
                                .into_iter()
                                .flat_map(move |g| {
                                    g.mods.iter().enumerate().filter_map(move |(index, mc)| {
                                        member_overrides.enabled(mc).then_some((
                                            (Some(group_name.clone()), index),
                                            mc.spec.clone(),
                                        ))
                                    })
                                }),
                        ),
                    }
                })
                .collect::<Vec<_>>();

            let ui_mod_tags = |ctx: &mut Ctx, ui: &mut Ui, info: &ModInfo| {
                if let Some(ModioTags {
                    qol,
                    gameplay,
                    audio,
                    visual,
                    framework,
                    required_status,
                    approval_status,
                    versions: _,
                }) = info.modio_tags.as_ref()
                {
                    let mut mk_searchable_modio_tag =
                        |tag_str: &str,
                         ui: &mut Ui,
                         color: Option<egui::Color32>,
                         hover_str: Option<&str>| {
                            let search = searchable_text(tag_str, &self.search_string, {
                                TextFormat {
                                    color: if color.is_some() {
                                        Color32::BLACK
                                    } else {
                                        Color32::GRAY
                                    },

                                    ..Default::default()
                                }
                            });

                            let button = if let Some(color) = color {
                                egui::Button::new(search.job)
                                    .small()
                                    .fill(color)
                                    .stroke(egui::Stroke::NONE)
                            } else {
                                egui::Button::new(search.job)
                                    .small()
                                    .stroke(egui::Stroke::NONE)
                            };

                            let res = if let Some(hover_str) = hover_str {
                                ui.add_enabled(false, button)
                                    .on_disabled_hover_text(hover_str)
                            } else {
                                ui.add_enabled(false, button)
                            };

                            if search.is_match && ctx.scroll_to_match {
                                res.scroll_to_me(None);
                                ctx.scroll_to_match = false;
                            }
                        };

                    match approval_status {
                        ApprovalStatus::Verified => {
                            mk_searchable_modio_tag(
                                "Verified",
                                ui,
                                Some(egui::Color32::LIGHT_GREEN),
                                Some("Does not contain any gameplay affecting features or changes"),
                            );
                        }
                        ApprovalStatus::Approved => {
                            mk_searchable_modio_tag(
                                "Approved",
                                ui,
                                Some(egui::Color32::LIGHT_BLUE),
                                Some("Contains gameplay affecting features or changes"),
                            );
                        }
                        ApprovalStatus::Sandbox => {
                            mk_searchable_modio_tag(
                                "Sandbox",
                                ui,
                                Some(egui::Color32::LIGHT_YELLOW),
                                Some(
                                    "Contains significant, possibly progression breaking, changes to gameplay",
                                ),
                            );
                        }
                    }

                    match required_status {
                        RequiredStatus::RequiredByAll => {
                            mk_searchable_modio_tag(
                                "RequiredByAll",
                                ui,
                                Some(egui::Color32::LIGHT_RED),
                                Some(
                                    "All lobby members must use this mod for it to work correctly!",
                                ),
                            );
                        }
                        RequiredStatus::Optional => {
                            mk_searchable_modio_tag(
                                "Optional",
                                ui,
                                None,
                                Some("Clients are not required to install this mod to function"),
                            );
                        }
                    }

                    if *qol {
                        mk_searchable_modio_tag("QoL", ui, None, None);
                    }
                    if *gameplay {
                        mk_searchable_modio_tag("Gameplay", ui, None, None);
                    }
                    if *audio {
                        mk_searchable_modio_tag("Audio", ui, None, None);
                    }
                    if *visual {
                        mk_searchable_modio_tag("Visual", ui, None, None);
                    }
                    if *framework {
                        mk_searchable_modio_tag("Framework", ui, None, None);
                    }
                }
            };

            let group_tint_snapshot: std::collections::BTreeMap<String, Option<String>> = groups
                .iter()
                .map(|(name, group)| (name.clone(), group.tint_color.clone()))
                .collect();

            let mut ui_mod =
                |ctx: &mut Ctx,
                 ui: &mut Ui,
                 group: Option<&str>,
                 row_index: usize,
                 mc: &mut ModConfig,
                 overrides: Option<&mut MemberOverrides>| {
                    let mut enabled = overrides.as_ref().map_or(mc.enabled, |o| o.enabled(mc));
                    let group_control_tint = group
                        .and_then(|name| group_tint_snapshot.get(name))
                        .and_then(|color| color.as_deref())
                        .map(|color| {
                            mod_list_style::swatch_color(Some(color), ui.visuals().dark_mode)
                        });
                    if !enabled {
                        let vis = ui.visuals_mut();
                        vis.override_text_color = Some(vis.text_color());
                        vis.hyperlink_color = vis.text_color();
                    }

                    if ui
                        .add(toggle_switch(&mut enabled))
                        .on_hover_text_at_pointer("Enabled?")
                        .changed()
                    {
                        if let Some(overrides) = overrides {
                            overrides.set(mc, enabled);
                        } else {
                            mc.enabled = enabled;
                        }
                        ctx.needs_save = true;
                    }

                    /*
                    if ui
                        .add(egui::Checkbox::without_text(&mut mc.required))
                        .changed()
                    {
                        needs_save = true;
                    }
                    */

                    if issues.duplicate(group, row_index)
                        && icon_buttons::warning_button(ui, group_control_tint)
                            .on_hover_text("Remove this duplicate entry")
                            .clicked()
                    {
                        ctx.btn_remove = Some((group.map(str::to_owned), row_index));
                    }

                    let info = self.state.store.get_mod_info(&mc.spec);
                    if self.state.store.is_modio_unavailable(&mc.spec) {
                        ui.label(RichText::new("⚠").color(ui.visuals().warn_fg_color))
                            .on_hover_text("Unavailable on mod.io. Cached metadata and downloaded mod files were preserved; other mod updates can continue.");
                    }
                    if let Some(ref info) = info
                        && let Some(modio_id) = info.modio_id
                        && self.problematic_mod_id.is_some_and(|id| id == modio_id)
                    {
                        let icon = egui::Button::new(RichText::new("❌").color(Color32::WHITE))
                            .fill(Color32::RED);
                        ui.add_enabled(false, icon);
                    }

                    if enabled && let Some(req) = &self.integrate_rid {
                        match req.state.get(&mc.spec) {
                            Some(SpecFetchProgress::Progress { progress, size }) => {
                                ui.add(
                                    egui::ProgressBar::new(*progress as f32 / *size as f32)
                                        .show_percentage()
                                        .desired_width(100.0),
                                );
                            }
                            Some(SpecFetchProgress::Complete) => {
                                ui.add(egui::ProgressBar::new(1.0).desired_width(100.0));
                            }
                            None => {
                                ui.spinner();
                            }
                        }
                    }

                    if let Some(info) = &info {
                        let previous_spec = mc.spec.clone();
                        ui.scope(|ui| {
                            mod_list_style::tint_group_member_controls(ui, group_control_tint);
                            egui::ComboBox::from_id_salt(row_index)
                                .selected_text(
                                    self.state
                                        .store
                                        .get_version_name(&mc.spec)
                                        .unwrap_or_default(),
                                )
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut mc.spec.url,
                                        info.spec.url.to_string(),
                                        self.state
                                            .store
                                            .get_version_name(&info.spec)
                                            .unwrap_or_default(),
                                    );
                                    for version in info.versions.iter().rev() {
                                        ui.selectable_value(
                                            &mut mc.spec.url,
                                            version.url.to_string(),
                                            self.state
                                                .store
                                                .get_version_name(version)
                                                .unwrap_or_default(),
                                        );
                                    }
                                });
                        });

                        ctx.needs_save |= mc.spec != previous_spec;

                        ui.scope(|ui| {
                            mod_list_style::tint_group_member_controls(ui, group_control_tint);
                        ui.style_mut().spacing.interact_size.x = 30.;
                        let dark = ui.visuals().dark_mode;
                        match mc.priority.cmp(&0) {
                            std::cmp::Ordering::Less => {
                                ui.visuals_mut().override_text_color = Some(if dark {
                                    Color32::LIGHT_RED
                                } else {
                                    Color32::DARK_RED
                                });
                            }
                            std::cmp::Ordering::Greater => {
                                ui.visuals_mut().override_text_color = Some(if dark {
                                    Color32::LIGHT_GREEN
                                } else {
                                    Color32::DARK_GREEN
                                });
                            }
                            _ => {}
                        }
                        if ui.add(
                            egui::DragValue::new(&mut mc.priority)
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
                                .speed(0.05)
                                .range(RangeInclusive::new(-999, 999)),
                        )
                        .on_hover_text_at_pointer(
                            "Load Priority\nIn case of asset conflict, mods with higher priority take precedent.\nCan have duplicate values.",
                        ).changed() {
                            ctx.needs_save = true;
                        }
                    });

                        if icon_buttons::copy_url_button(ui, group_control_tint)
                            .on_hover_text_at_pointer("copy URL")
                            .clicked()
                        {
                            ui.ctx().copy_text(mc.spec.url.to_string());
                        }

                        if enabled {
                            let missing_deps = info
                                .suggested_dependencies
                                .iter()
                                .filter(|d| {
                                    !enabled_specs.iter().any(|(_, s)| s.satisfies_dependency(d))
                                })
                                .collect::<Vec<_>>();

                            if !missing_deps.is_empty() {
                                let mut msg = "Add missing dependencies:".to_string();
                                for dep in &missing_deps {
                                    msg.push('\n');
                                    msg.push_str(&dep.url);
                                }
                                if icon_buttons::warning_button(ui, group_control_tint)
                                    .on_hover_text(msg)
                                    .clicked()
                                {
                                    ctx.add_deps =
                                        Some(missing_deps.into_iter().cloned().collect());
                                }
                            }
                        }

                        match info.provider {
                            "modio" => {
                                let texture: &egui::TextureHandle =
                                    self.modio_texture_handle.get_or_insert_with(|| {
                                        let image =
                                            image::load_from_memory(MODIO_LOGO_PNG).unwrap();
                                        let size = [image.width() as _, image.height() as _];
                                        let image_buffer = image.to_rgba8();
                                        let pixels = image_buffer.as_flat_samples();
                                        let image = egui::ColorImage::from_rgba_unmultiplied(
                                            size,
                                            pixels.as_slice(),
                                        );

                                        ui.ctx().load_texture(
                                            "modio-logo",
                                            image,
                                            Default::default(),
                                        )
                                    });
                                let mut img = egui::Image::new(texture)
                                    .fit_to_exact_size([16.0, 16.0].into());
                                if !enabled {
                                    img = img.tint(Color32::LIGHT_RED);
                                }
                                ui.add(img);
                            }
                            "http" => {
                                ui.label("🌐");
                            }
                            "file" => {
                                icon_buttons::folder_badge(ui, group_control_tint);
                            }
                            _ => unimplemented!("unimplemented provider kind"),
                        }

                        let search = searchable_text(&info.name, &self.search_string, {
                            TextFormat {
                                color: ui.visuals().hyperlink_color,
                                ..Default::default()
                            }
                        });

                        let res = ui.hyperlink_to(search.job, &mc.spec.url);
                        if search.is_match && ctx.scroll_to_match {
                            res.scroll_to_me(None);
                            ctx.scroll_to_match = false;
                        }

                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if let Some(description) = mc
                                .description
                                .as_deref()
                                .filter(|text| !text.trim().is_empty())
                            {
                                description_help_button(ui, description);
                            }
                            ui_mod_tags(ctx, ui, info);
                        });
                    } else {
                        if icon_buttons::copy_url_button(ui, group_control_tint)
                            .on_hover_text_at_pointer("Copy URL")
                            .clicked()
                        {
                            ui.ctx().copy_text(mc.spec.url.to_string());
                        }

                        let search = searchable_text(&mc.spec.url, &self.search_string, {
                            TextFormat {
                                color: ui.visuals().hyperlink_color,
                                ..Default::default()
                            }
                        });

                        let res = ui.hyperlink_to(search.job, &mc.spec.url);
                        if search.is_match && ctx.scroll_to_match {
                            res.scroll_to_me(None);
                            ctx.scroll_to_match = false;
                        }
                        if let Some(description) = mc
                            .description
                            .as_deref()
                            .filter(|text| !text.trim().is_empty())
                        {
                            description_help_button(ui, description);
                        }
                    }
                };

            let mut ui_item = |ctx: &mut Ctx,
                               ui: &mut Ui,
                               mc: &mut ModOrGroup,
                               row_index: usize| {
                ui.scope(|ui| {
                    // Color group rows just like the group header, but keep standalone mods neutral.
                    let delete_accent = match &*mc {
                        ModOrGroup::Group { group_name, .. } => group_tint_snapshot
                            .get(group_name.as_str())
                            .and_then(|color| color.as_deref())
                            .map(|color| {
                                mod_list_style::swatch_color(Some(color), ui.visuals().dark_mode)
                            }),
                        _ => None,
                    };

                    ui.visuals_mut().widgets.hovered.weak_bg_fill = colors::DARK_RED;
                    ui.visuals_mut().widgets.active.weak_bg_fill = colors::DARKER_RED;
                    if icon_buttons::delete_button(ui, delete_accent)
                        .on_hover_text_at_pointer("Delete mod")
                        .clicked()
                    {
                        ctx.btn_remove = Some((None, row_index));
                    };
                });

                match mc {
                    ModOrGroup::Individual(mc) => {
                        ui.push_id(("mod", row_index), |ui| {
                            ui_mod(ctx, ui, None, row_index, mc, None);
                        });
                    }
                    ModOrGroup::Group {
                        group_name,
                        enabled,
                        member_overrides,
                        ..
                    } => {
                        if ui
                            .add(toggle_switch(enabled))
                            .on_hover_text_at_pointer("Enabled?")
                            .changed()
                        {
                            ctx.needs_save = true;
                        }
                        let open = !self.groups_to_open.is_empty()
                            && self
                                .groups_to_open
                                .remove(&(profile_name.to_owned(), group_name.clone()));
                        let selected_group_tint = groups
                            .get(group_name)
                            .and_then(|group| group.tint_color.clone());
                        let group_description = groups
                            .get(group_name)
                            .and_then(|group| group.description.clone())
                            .filter(|text| !text.trim().is_empty());
                        let group_member_count =
                            groups.get(group_name).map_or(0, |group| group.mods.len());
                        // Match children even when the collapsing group does not render their rows.
                        let search_needle = self.search_string.to_lowercase();
                        let matching_mod_in_group = !search_needle.trim().is_empty()
                            && groups.get(group_name).is_some_and(|group| {
                                group.mods.iter().any(|mc| {
                                    let display_name = self
                                        .state
                                        .store
                                        .get_mod_info(&mc.spec)
                                        .map(|info| info.name)
                                        .unwrap_or_else(|| mc.spec.url.clone());
                                    display_name.to_lowercase().contains(&search_needle)
                                })
                            });
                        // egui::CollapsingState::body requires a vertical layout for indentation.
                        let (header_response, body_response, fully_closed) = ui.vertical(|ui| {
                            // Keep the group title clickable and render its help icon
                            // independently, on the same header row.
                            let mut collapse = egui::collapsing_header::CollapsingState::load_with_default_open(
                                ui.ctx(),
                                ui.make_persistent_id((profile_name, group_name.as_str())),
                                false,
                            );
                            if open {
                                collapse.set_open(true);
                            }
                            let fully_closed = collapse.openness(ui.ctx()) <= 0.0;
                            // Tinted names are slightly desaturated for legibility on
                            // the existing colored header background in both themes.
                            let group_header_color = mod_list_style::group_header_text_color(
                                ui,
                                selected_group_tint.as_deref(),
                            );
                            // Highlight the group while its matching members are hidden.
                            let group_title = if fully_closed && matching_mod_in_group {
                                RichText::new(group_name.as_str())
                                    .strong()
                                    .background_color(Color32::YELLOW)
                                    .color(Color32::BLACK)
                            } else {
                                let mut title = RichText::new(group_name.as_str()).strong();
                                if let Some(color) = group_header_color {
                                    title = title.color(color);
                                }
                                title
                            };
                            // mintfixed: soft neutral text for uncolored group headers
                            let group_title = if selected_group_tint.is_none()
                                && !(fully_closed && matching_mod_in_group)
                            {
                                group_title.color(if ui.visuals().dark_mode {
                                    Color32::from_gray(164)
                                } else {
                                    Color32::from_gray(92)
                                })
                            } else {
                                group_title
                            };
                            let mut title_clicked = false;
                            // egui draws its built-in collapse arrow before the header
                            // callback, so tint its foreground only while painting this
                            // group header. Save/restore widget styling for the mod rows.
                            let previous_arrow_widgets = group_header_color.map(|color| {
                                let widgets = &mut ui.visuals_mut().widgets;
                                let previous = widgets.clone();
                                for state in [
                                    &mut widgets.inactive,
                                    &mut widgets.hovered,
                                    &mut widgets.active,
                                    &mut widgets.open,
                                ] {
                                    state.fg_stroke.color = color;
                                }
                                previous
                            });
                            let mut header = collapse.show_header(ui, |ui| {
                                title_clicked = ui
                                    .add(egui::Label::new(group_title).sense(egui::Sense::click()))
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .clicked();
                                ui.label(
                                    RichText::new(format!(
                                        "{} mod{}",
                                        group_member_count,
                                        if group_member_count == 1 { "" } else { "s" }
                                    ))
                                    .small()
                                    .weak(),
                                );
                                let selected_color = groups
                                    .get(group_name)
                                    .and_then(|group| group.tint_color.clone());
                                let mut requested_color: Option<Option<String>> = None;
                                // group color control — right aligned in the remaining header width
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.scope(|ui| {
                                        ui.spacing_mut().button_padding = egui::vec2(2.0, 0.0);
                                        // Invisible at rest; highlight only when hovered or selected.
                                        ui.visuals_mut().widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
                                        ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;
                                        ui.menu_image_button(
                                            egui::Image::new(egui::include_image!("../../assets/icons/group_palette.svg"))
                                                .fit_to_exact_size(egui::vec2(15.0, 15.0))
                                                .tint(mod_list_style::swatch_color(
                                                    selected_color.as_deref(),
                                                    ui.visuals().dark_mode,
                                                )),
                                            |ui| {
                                                if mod_list_style::color_menu_option(ui, None, "None", selected_color.is_none()) {
                                                    requested_color = Some(None);
                                                    ui.close_menu();
                                                }
                                                for &(key, label) in mod_list_style::COLOR_OPTIONS {
                                                    if mod_list_style::color_menu_option(
                                                        ui,
                                                        Some(key),
                                                        label,
                                                        selected_color.as_deref() == Some(key),
                                                    ) {
                                                        requested_color = Some(Some(key.to_owned()));
                                                        ui.close_menu();
                                                    }
                                                }
                                            },
                                        )
                                        .response
                                        .on_hover_text("Group color");
                                    });
                                    if let Some(description) = group_description.as_deref() {
                                        description_help_button(ui, description);
                                    }
                                });
                                if let Some(color) = requested_color {
                                    if let Some(group) = groups.get_mut(group_name) {
                                        group.tint_color = color;
                                        ctx.needs_save = true;
                                        ui.ctx().request_repaint();
                                    }
                                }
                            });
                            if title_clicked {
                                header.toggle();
                            }
                            let (_toggle_response, header_contents, body_response) = header.body(|ui| {
                                // The tinted arrow must not recolor the group's mods.
                                if let Some(previous) = &previous_arrow_widgets {
                                    ui.visuals_mut().widgets = previous.clone();
                                }
                                let group = groups.get_mut(group_name).unwrap();
                                let order = sorted_mod_indices(
                                    Some(member_overrides),
                                    group.mods.iter().map(Some),
                                    sorting_config.as_ref(),
                                    |spec| self.state.store.get_mod_info(spec),
                                );
                                for (visual_index, index) in order.into_iter().enumerate() {
                                    if bitfix_menu::is_lua_spec(&group.mods[index].spec) {
                                        continue;
                                    }
                                    let member = Drag::paint_row(ui, profile_name, Some(group_name), index, |ui| {
                                        mod_list_style::row_frame(ui, visual_index, false, selected_group_tint.as_deref(), true, false)
                                            .show(ui, |ui| {
                                                ui.set_min_width(ui.available_width());
ui.push_id(("group", group_name.as_str(), index), |ui| {
                                        ui.horizontal(|ui| {
                                            if sorting_config.is_none() {
                                                Drag::handle(ui, profile_name, Some(group_name), index, &group.mods[index].id, false);
                                            }
                                            ui.scope(|ui| {
                                                // Match the shared group's selected color.
                                                let delete_accent = selected_group_tint.as_deref()
                                                    .map(|color| mod_list_style::swatch_color(Some(color), ui.visuals().dark_mode));

                                                ui.visuals_mut().widgets.hovered.weak_bg_fill = colors::DARK_RED;
                                                ui.visuals_mut().widgets.active.weak_bg_fill = colors::DARKER_RED;
                                                if icon_buttons::delete_button(ui, delete_accent)
                                                    .on_hover_text_at_pointer("Remove mod from shared group (confirmation required)")
                                                    .clicked()
                                                {
                                                    ctx.btn_remove = Some((Some(group_name.clone()), index));
                                                }
                                            });
                                            ui_mod(
                                                ctx,
                                                ui,
                                                Some(group_name),
                                                index,
                                                &mut group.mods[index],
                                                Some(member_overrides),
                                            );
                                        });
                                    })
                                            })
                                            .inner
                                    });
                                    if sorting_config.is_none() {
                                        group_controls::row_target(ui, &member.response, &mut ctx.drag, Some(group_name), index);
                                    }
                                }
                            });
                            // Closed groups have no body, so restore the header UI too.
                            if let Some(previous) = previous_arrow_widgets {
                                ui.visuals_mut().widgets = previous;
                            }
                            let header_response = header_contents.response;
                            if fully_closed && matching_mod_in_group && ctx.scroll_to_match {
                                header_response.scroll_to_me(None);
                                ctx.scroll_to_match = false;
                            }
                            let body_response = body_response.map(|body| body.response);
                                (header_response, body_response, fully_closed)
                            }).inner;
                        // Give the group header a full-width drop target, but leave
                        // narrow bands for inserting a mod *outside* the group.
                        let mut header_row = header_response.rect;
                        header_row.min.x = ui.max_rect().left();
                        header_row.max.x = ui.max_rect().right();
                        let edge = 5.0_f32.min(header_row.height() * 0.2);
                        // An expanded group's members immediately follow the header,
                        // so the bottom edge should drop into the group, not after it.
                        let bottom_edge = if fully_closed { edge } else { 0.0 };
                        let mut header_drop_area = header_row;
                        header_drop_area.min.y += edge;
                        header_drop_area.max.y -= bottom_edge;
                        let above_group = egui::Rect::from_min_max(
                            header_row.min,
                            egui::pos2(header_row.max.x, header_drop_area.min.y),
                        );
                        let below_group = egui::Rect::from_min_max(
                            egui::pos2(header_row.min.x, header_drop_area.max.y),
                            header_row.max,
                        );
                        if group_controls::hover_open(
                            ui,
                            &header_response,
                            header_drop_area,
                            fully_closed,
                            profile_name,
                            sorting_config.is_none(),
                        ) {
                            self.groups_to_open
                                .insert((profile_name.to_owned(), group_name.clone()));
                            ui.ctx().request_repaint();
                        }
                        let mut area = header_drop_area;
                        if let Some(body) = &body_response {
                            area = area.union(body.rect);
                        }
                        if ui.rect_contains_pointer(area) {
                            ctx.file_group = Some((group_name.clone(), fully_closed));
                        }
                        let over_group_header = ui.rect_contains_pointer(header_drop_area);
                        let over_above_group = ui.rect_contains_pointer(above_group);
                        let over_below_group =
                            fully_closed && ui.rect_contains_pointer(below_group);
                        let dragging_mod = egui::DragAndDrop::payload::<Drag>(ui.ctx())
                            .is_some_and(|d| !d.is_group);
                        // Highlight the destination without drawing lines over the
                        // group name. A slim rounded bar indicates a profile-level
                        // insertion; a soft tint indicates a drop into the group.
                        let accent = ui.visuals().selection.stroke.color;
                        let group_fill =
                            Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 42);
                        if sorting_config.is_none() && dragging_mod {
                            if over_above_group || over_below_group {
                                let y = if over_above_group {
                                    above_group.center().y
                                } else {
                                    below_group.center().y
                                };
                                let marker = egui::Rect::from_center_size(
                                    egui::pos2(header_row.center().x, y),
                                    egui::vec2((header_row.width() - 16.0).max(0.0), 3.0),
                                );
                                ui.painter().rect_filled(
                                    marker,
                                    egui::CornerRadius::same(2),
                                    accent,
                                );
                            } else if over_group_header {
                                ui.painter().rect_filled(
                                    header_drop_area.shrink2(egui::vec2(3.0, 0.0)),
                                    egui::CornerRadius::same(4),
                                    group_fill,
                                );
                            }
                        } else if ctx
                            .file_group
                            .as_ref()
                            .is_some_and(|(name, _)| name == group_name)
                            && ui.input(|i| !i.raw.hovered_files.is_empty())
                        {
                            // Use the same drop-into-group feedback for local .pak files.
                            ui.painter().rect_filled(
                                header_drop_area.shrink2(egui::vec2(3.0, 0.0)),
                                egui::CornerRadius::same(4),
                                group_fill,
                            );
                        }
                        if sorting_config.is_none()
                            && dragging_mod
                            && (over_above_group || over_group_header || over_below_group)
                            && ui.input(|i| i.pointer.any_released())
                            && ctx.drag.is_none()
                            && let Some(source) = egui::DragAndDrop::payload::<Drag>(ui.ctx())
                        {
                            let target = if over_above_group {
                                Target {
                                    group: None,
                                    slot: row_index,
                                }
                            } else if over_below_group {
                                Target {
                                    group: None,
                                    slot: row_index + 1,
                                }
                            } else {
                                // Dropping on the header always prepends to the group,
                                // whether it was already open or expanded on hover.
                                Target {
                                    group: Some(group_name.clone()),
                                    slot: 0,
                                }
                            };
                            ctx.drag = Some(((*source).clone(), target));
                        }
                    }
                }
            };

            if sorting_config.is_some() {
                let order = sorted_mod_indices(
                    None,
                    profile.mods.iter().map(|item| match item {
                        ModOrGroup::Individual(mc) => Some(mc),
                        ModOrGroup::Group { .. } => None,
                    }),
                    sorting_config.as_ref(),
                    |spec| self.state.store.get_mod_info(spec),
                );
                let mut previous_group_row_tint: Option<&str> = None;
                let mut matching_color_run_offset: usize = 0;
                for (visual_index, store_index) in order.into_iter().enumerate() {
                    // Compare only consecutive visible rows, not unrelated groups.
                    let current_group_row_tint = match &profile.mods[store_index] {
                        ModOrGroup::Group { group_name, .. } => group_tint_snapshot
                            .get(group_name)
                            .and_then(|tint| tint.as_deref()),
                        _ => None,
                    };
                    let same_color_as_previous = mod_list_style::same_selected_color(
                        previous_group_row_tint,
                        current_group_row_tint,
                    );
                    matching_color_run_offset = if same_color_as_previous {
                        matching_color_run_offset + 1
                    } else {
                        0
                    };
                    let stripe_matching_color =
                        current_group_row_tint.is_some() && matching_color_run_offset % 2 == 1;
                    previous_group_row_tint = current_group_row_tint;

                    if matches!(&profile.mods[store_index], ModOrGroup::Individual(mc) if bitfix_menu::is_lua_spec(&mc.spec))
                    {
                        continue;
                    }
                    let frame = match &profile.mods[store_index] {
                        ModOrGroup::Group { group_name, .. } => mod_list_style::row_frame(
                            ui,
                            visual_index,
                            true,
                            group_tint_snapshot
                                .get(group_name)
                                .and_then(|tint| tint.as_deref()),
                            false,
                            stripe_matching_color,
                        ),
                        _ => mod_list_style::row_frame(ui, visual_index, false, None, false, false),
                    };
                    frame.show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui_item(&mut ctx, ui, &mut profile.mods[store_index], store_index);
                        });
                    });
                }
            } else {
                let mut previous_group_row_tint: Option<&str> = None;
                let mut matching_color_run_offset: usize = 0;
                for (index, item) in profile.mods.iter_mut().enumerate() {
                    // Compare only consecutive visible rows, not unrelated groups.
                    let current_group_row_tint = match &*item {
                        ModOrGroup::Group { group_name, .. } => group_tint_snapshot
                            .get(group_name)
                            .and_then(|tint| tint.as_deref()),
                        _ => None,
                    };
                    let same_color_as_previous = mod_list_style::same_selected_color(
                        previous_group_row_tint,
                        current_group_row_tint,
                    );
                    matching_color_run_offset = if same_color_as_previous {
                        matching_color_run_offset + 1
                    } else {
                        0
                    };
                    let stripe_matching_color =
                        current_group_row_tint.is_some() && matching_color_run_offset % 2 == 1;
                    previous_group_row_tint = current_group_row_tint;

                    if matches!(&*item, ModOrGroup::Individual(mc) if bitfix_menu::is_lua_spec(&mc.spec))
                    {
                        continue;
                    }
                    let frame = match item {
                        ModOrGroup::Group { group_name, .. } => mod_list_style::row_frame(
                            ui,
                            index,
                            true,
                            group_tint_snapshot
                                .get(group_name)
                                .and_then(|tint| tint.as_deref()),
                            false,
                            stripe_matching_color,
                        ),
                        _ => mod_list_style::row_frame(ui, index, false, None, false, false),
                    };
                    let row = Drag::paint_row(ui, profile_name, None, index, |ui| {
                        frame.show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.horizontal(|ui| {
                                let (key, is_group) = match item {
                                    ModOrGroup::Individual(mc) => (mc.id.as_str(), false),
                                    ModOrGroup::Group { id, .. } => (id.as_str(), true),
                                };
                                Drag::handle(ui, profile_name, None, index, key, is_group);
                                ui_item(&mut ctx, ui, item, index);
                            });
                        })
                    });
                    // The group header already handles its three drop zones.
                    // Do not paint a second, competing profile-row marker on top
                    // of its highlight. Still handle gaps between rows and group drags.
                    let mut group_row_area = row.response.rect;
                    group_row_area.min.x = ui.max_rect().left();
                    group_row_area.max.x = ui.max_rect().right();
                    let group_handles_mod_drop = matches!(item, ModOrGroup::Group { .. })
                        && egui::DragAndDrop::payload::<Drag>(ui.ctx())
                            .is_some_and(|drag| !drag.is_group)
                        && ui.rect_contains_pointer(group_row_area);
                    if !group_handles_mod_drop {
                        group_controls::row_target(ui, &row.response, &mut ctx.drag, None, index);
                    }
                }
                // Treat all visible empty space below the last row as a top-level
                // insertion target. A single label only provided one row of hit area,
                // making it difficult to drag a mod out of the last expanded group.
                if sorting_config.is_none()
                    && ctx.drag.is_none()
                    && let Some(source) = egui::DragAndDrop::payload::<Drag>(ui.ctx())
                {
                    let viewport = ui.clip_rect();
                    let top = ui.cursor().top().max(viewport.top());
                    if top < viewport.bottom() {
                        let end_area = egui::Rect::from_min_max(
                            egui::pos2(viewport.left(), top),
                            viewport.right_bottom(),
                        );
                        if ui.rect_contains_pointer(end_area) {
                            // Keep the insertion indicator at the end of the list,
                            // regardless of how far into the blank area we hover.
                            let marker = egui::Rect::from_center_size(
                                egui::pos2(end_area.center().x, top),
                                egui::vec2((end_area.width() - 16.0).max(0.0), 3.0),
                            );
                            ui.painter().rect_filled(
                                marker,
                                egui::CornerRadius::same(2),
                                ui.visuals().selection.stroke.color,
                            );
                            if ui.input(|input| input.pointer.any_released()) {
                                ctx.drag = Some((
                                    (*source).clone(),
                                    Target {
                                        group: None,
                                        slot: profile.mods.len(),
                                    },
                                ));
                            }
                        }
                    }
                }
            }
        };

        egui::ScrollArea::vertical().show(ui, |ui| {
            group_controls::auto_scroll(ui, profile_name, sorting_config.is_none());
            if let Some(profile) = profiles.get_mut(profile) {
                ui_profile(ui, profile);
            } else {
                ui.label("no such profile");
            }
        });

        if let Some((target, index)) = ctx.btn_remove {
            if let Some(name) = target {
                // The duplicate-entry warning uses this path too: always confirm.
                if let Some(mc) = groups.get(&name).and_then(|group| group.mods.get(index)) {
                    self.pending_group_mod_removal = Some(PendingGroupModRemoval {
                        group_name: name,
                        mod_id: mc.id.clone(),
                        mod_name: self
                            .state
                            .store
                            .get_mod_info(&mc.spec)
                            .map(|info| info.name)
                            .unwrap_or_else(|| mc.spec.url.clone()),
                    });
                }
            } else {
                profiles.get_mut(profile).unwrap().mods.remove(index);
                ctx.needs_save = true;
            }
        }

        if let Some((source, dest)) = ctx.drag {
            let before = self.state.mod_data.deref().deref().clone();
            let destination_group = dest.group.clone();
            match source.apply(&mut self.state.mod_data, profile, dest, &self.state.store) {
                Ok(()) => {
                    if let Some(name) = destination_group {
                        self.groups_to_open.insert((profile.to_owned(), name));
                        ui.ctx().request_repaint();
                    }
                    match self.state.mod_data.save() {
                        Ok(()) => {
                            ctx.needs_save = false;
                        }
                        Err(error) => {
                            *self.state.mod_data.deref_mut().deref_mut() = before;
                            self.last_action =
                                Some(LastAction::failure(format!("Could not save move: {error}")));
                        }
                    }
                }
                Err(error) => self.last_action = Some(LastAction::failure(error.into())),
            }
        }

        if let Some(add_deps) = ctx.add_deps {
            message::ResolveMods::send(self, ui.ctx(), add_deps, true);
            self.problematic_mod_id = None;
        }

        self.scroll_to_match = ctx.scroll_to_match;

        if ctx.needs_save {
            self.state.mod_data.save().unwrap();
        }
        ctx.file_group
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

    fn show_lints_toggle(&mut self, ctx: &egui::Context) {
        if let Some(_lints_toggle) = &self.lints_toggle_window {
            let mut open = true;

            egui::Window::new("Toggle lints")
                .open(&mut open)
                .resizable(false)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        egui::Grid::new("lints-toggle-grid").show(ui, |ui| {
                            ui.heading("Lint");
                            ui.heading("Enabled?");
                            ui.end_row();

                            ui.label("Archive with multiple paks");
                            ui.add(toggle_switch(
                                &mut self.lint_options.archive_with_multiple_paks,
                            ));
                            ui.end_row();

                            ui.label("Archive with only non-pak files");
                            ui.add(toggle_switch(
                                &mut self.lint_options.archive_with_only_non_pak_files,
                            ));
                            ui.end_row();

                            ui.label("Mods containing AssetRegister.bin");
                            ui.add(toggle_switch(&mut self.lint_options.asset_register_bin));
                            ui.end_row();

                            ui.label("Mods containing conflicting files");
                            ui.add(toggle_switch(&mut self.lint_options.conflicting));
                            ui.end_row();

                            ui.label("Mods containing empty archives");
                            ui.add(toggle_switch(&mut self.lint_options.empty_archive));
                            ui.end_row();

                            ui.label("Mods containing oudated pak version");
                            ui.add(toggle_switch(&mut self.lint_options.outdated_pak_version));
                            ui.end_row();

                            ui.label("Mods containing shader files");
                            ui.add(toggle_switch(&mut self.lint_options.shader_files));
                            ui.end_row();

                            ui.label("Mods containing non-asset files");
                            ui.add(toggle_switch(&mut self.lint_options.non_asset_files));
                            ui.end_row();

                            ui.label("Mods containing split {uexp, uasset} pairs");
                            ui.add(toggle_switch(&mut self.lint_options.split_asset_pairs));
                            ui.end_row();

                            ui.label("Mods containing unmodified game assets");
                            ui.add_enabled(
                                self.state.config.drg_pak_path.is_some(),
                                toggle_switch(&mut self.lint_options.unmodified_game_assets),
                            )
                            .on_disabled_hover_text(
                                "This lint requires DRG pak path to be specified",
                            );
                            ui.end_row();
                        });
                    });

                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            self.lints_toggle_window = None;
                        }

                        if ui
                            .add_enabled(
                                self.check_updates_rid.is_none()
                                    && self.integrate_rid.is_none()
                                    && self.lint_rid.is_none(),
                                egui::Button::new("Generate report"),
                            )
                            .clicked()
                        {
                            let lint_options = BTreeMap::from([
                                (
                                    LintId::ARCHIVE_WITH_MULTIPLE_PAKS,
                                    self.lint_options.archive_with_multiple_paks,
                                ),
                                (
                                    LintId::ARCHIVE_WITH_ONLY_NON_PAK_FILES,
                                    self.lint_options.archive_with_only_non_pak_files,
                                ),
                                (
                                    LintId::ASSET_REGISTRY_BIN,
                                    self.lint_options.asset_register_bin,
                                ),
                                (LintId::CONFLICTING, self.lint_options.conflicting),
                                (LintId::EMPTY_ARCHIVE, self.lint_options.empty_archive),
                                (
                                    LintId::OUTDATED_PAK_VERSION,
                                    self.lint_options.outdated_pak_version,
                                ),
                                (LintId::SHADER_FILES, self.lint_options.shader_files),
                                (LintId::NON_ASSET_FILES, self.lint_options.non_asset_files),
                                (
                                    LintId::SPLIT_ASSET_PAIRS,
                                    self.lint_options.split_asset_pairs,
                                ),
                                (
                                    LintId::UNMODIFIED_GAME_ASSETS,
                                    self.lint_options.unmodified_game_assets,
                                ),
                            ]);

                            trace!(?lint_options);

                            let mut mods = Vec::new();
                            self.state.mod_data.for_each_enabled_mod(
                                &self.state.mod_data.active_profile,
                                |mc| {
                                    mods.push(mc.spec.clone());
                                },
                            );

                            self.lint_report = None;
                            self.lint_rid = Some(message::LintMods::send(
                                &mut self.request_counter,
                                self.state.store.clone(),
                                mods,
                                BTreeSet::from_iter(
                                    lint_options
                                        .into_iter()
                                        .filter_map(|(lint, enabled)| enabled.then_some(lint)),
                                ),
                                self.state.config.drg_pak_path.clone(),
                                self.tx.clone(),
                                ctx.clone(),
                            ));
                            self.problematic_mod_id = None;
                            self.lint_report_window = Some(WindowLintReport);
                        }
                    });
                });

            if !open {
                self.lints_toggle_window = None;
            }
        }
    }

    fn show_lint_report(&mut self, ctx: &egui::Context) {
        if self.lint_report_window.is_some() {
            let mut open = true;

            egui::Window::new("Lint results")
                .open(&mut open)
                .resizable(true)
                .show(ctx, |ui| {
                    if let Some(report) = &self.lint_report {
                        let scroll_height =
                            (ui.available_height() - 30.0).clamp(0.0, f32::INFINITY);
                        egui::ScrollArea::vertical()
                            .max_height(scroll_height)
                            .show(ui, |ui| {
                                const AMBER: Color32 = Color32::from_rgb(255, 191, 0);

                                if let Some(conflicting_mods) = &report.conflicting_mods
                                    && !conflicting_mods.is_empty() {
                                        CollapsingHeader::new(
                                            RichText::new("⚠ Mods(s) with conflicting asset modifications detected")
                                                .color(AMBER),
                                        )
                                        .default_open(true)
                                        .show(ui, |ui| {
                                            conflicting_mods.iter().for_each(|(path, mods)| {
                                                CollapsingHeader::new(
                                                    RichText::new(format!(
                                                        "⚠ Conflicting modification of asset `{path}`"
                                                    ))
                                                    .color(AMBER),
                                                )
                                                .show(
                                                    ui,
                                                    |ui| {
                                                        mods.iter().for_each(|mod_spec| {
                                                            ui.label(&mod_spec.url);
                                                        });
                                                    },
                                                );
                                            });
                                        });
                                    }

                                if let Some(asset_register_bin_mods) = &report.asset_register_bin_mods
                                    && !asset_register_bin_mods.is_empty() {
                                        CollapsingHeader::new(
                                            RichText::new("ℹ Mod(s) with `AssetRegistry.bin` included detected")
                                                .color(Color32::LIGHT_BLUE),
                                        )
                                        .default_open(true)
                                        .show(ui, |ui| {
                                            asset_register_bin_mods.iter().for_each(
                                                |(r#mod, paths)| {
                                                    CollapsingHeader::new(
                                                        RichText::new(format!(
                                                        "ℹ {} includes one or more `AssetRegistry.bin`",
                                                        r#mod.url
                                                    ))
                                                        .color(Color32::LIGHT_BLUE),
                                                    )
                                                    .show(ui, |ui| {
                                                        paths.iter().for_each(|path| {
                                                            ui.label(path);
                                                        });
                                                    });
                                                },
                                            );
                                        });
                                    }

                                if let Some(shader_file_mods) = &report.shader_file_mods
                                    && !shader_file_mods.is_empty() {
                                        CollapsingHeader::new(
                                            RichText::new(
                                                "⚠ Mods(s) with shader files included detected",
                                            )
                                            .color(AMBER),
                                        )
                                        .default_open(true)
                                        .show(ui, |ui| {
                                            shader_file_mods.iter().for_each(
                                                |(r#mod, shader_files)| {
                                                    CollapsingHeader::new(
                                                        RichText::new(format!(
                                                            "⚠ {} includes one or more shader files",
                                                            r#mod.url
                                                        ))
                                                        .color(AMBER),
                                                    )
                                                    .show(ui, |ui| {
                                                        shader_files.iter().for_each(|shader_file| {
                                                            ui.label(shader_file);
                                                        });
                                                    });
                                                },
                                            );
                                        });
                                    }

                                if let Some(outdated_pak_version_mods) = &report.outdated_pak_version_mods
                                    && !outdated_pak_version_mods.is_empty() {
                                        CollapsingHeader::new(
                                            RichText::new(
                                                "⚠ Mod(s) with outdated pak version detected",
                                            )
                                            .color(AMBER),
                                        )
                                        .default_open(true)
                                        .show(ui, |ui| {
                                            outdated_pak_version_mods.iter().for_each(
                                                |(r#mod, version)| {
                                                    ui.label(
                                                        RichText::new(format!(
                                                            "⚠ {} includes outdated pak version {}",
                                                            r#mod.url, version
                                                        ))
                                                        .color(AMBER),
                                                    );
                                                },
                                            );
                                        });
                                    }

                                if let Some(empty_archive_mods) = &report.empty_archive_mods
                                    && !empty_archive_mods.is_empty() {
                                        CollapsingHeader::new(
                                            RichText::new(
                                                "⚠ Mod(s) with empty archives detected",
                                            )
                                            .color(AMBER),
                                        )
                                        .default_open(true)
                                        .show(ui, |ui| {
                                            empty_archive_mods.iter().for_each(|r#mod| {
                                                ui.label(
                                                    RichText::new(format!(
                                                        "⚠ {} contains an empty archive",
                                                        r#mod.url
                                                    ))
                                                    .color(AMBER),
                                                );
                                            });
                                        });
                                    }

                                if let Some(archive_with_only_non_pak_files_mods) = &report.archive_with_only_non_pak_files_mods
                                    && !archive_with_only_non_pak_files_mods.is_empty() {
                                        CollapsingHeader::new(
                                            RichText::new(
                                                "⚠ Mod(s) with only non-`.pak` files detected",
                                            )
                                            .color(AMBER),
                                        )
                                        .default_open(true)
                                        .show(ui, |ui| {
                                            archive_with_only_non_pak_files_mods.iter().for_each(|r#mod| {
                                                ui.label(
                                                    RichText::new(format!(
                                                        "⚠ {} contains only non-`.pak` files, perhaps the author forgot to pack it?",
                                                        r#mod.url
                                                    ))
                                                    .color(AMBER),
                                                );
                                            });
                                        });
                                    }

                                if let Some(archive_with_multiple_paks_mods) = &report.archive_with_multiple_paks_mods
                                    && !archive_with_multiple_paks_mods.is_empty() {
                                        CollapsingHeader::new(
                                            RichText::new(
                                                "⚠ Mod(s) with multiple `.pak`s detected",
                                            )
                                            .color(AMBER),
                                        )
                                        .default_open(true)
                                        .show(ui, |ui| {
                                            archive_with_multiple_paks_mods.iter().for_each(|r#mod| {
                                                ui.label(RichText::new(format!(
                                                    "⚠ {} contains multiple `.pak`s, only the first encountered `.pak` will be loaded",
                                                    r#mod.url
                                                ))
                                                .color(AMBER));
                                            });
                                        });
                                    }

                                if let Some(non_asset_file_mods) = &report.non_asset_file_mods
                                    && !non_asset_file_mods.is_empty() {
                                        CollapsingHeader::new(
                                            RichText::new(
                                                "⚠ Mod(s) with non-asset files detected",
                                            )
                                            .color(AMBER),
                                        )
                                        .default_open(true)
                                        .show(ui, |ui| {
                                            non_asset_file_mods.iter().for_each(|(r#mod, files)| {
                                                CollapsingHeader::new(
                                                    RichText::new(format!(
                                                        "⚠ {} includes non-asset files",
                                                        r#mod.url
                                                    ))
                                                    .color(AMBER),
                                                )
                                                .show(ui, |ui| {
                                                    files.iter().for_each(|file| {
                                                        ui.label(file);
                                                    });
                                                });
                                            });
                                        });
                                    }

                                if let Some(split_asset_pairs_mods) = &report.split_asset_pairs_mods
                                    && !split_asset_pairs_mods.is_empty() {
                                        CollapsingHeader::new(
                                            RichText::new(
                                                "⚠ Mod(s) with split {uexp, uasset} pairs detected",
                                            )
                                            .color(AMBER),
                                        )
                                        .default_open(true)
                                        .show(ui, |ui| {
                                            split_asset_pairs_mods.iter().for_each(|(r#mod, files)| {
                                                CollapsingHeader::new(
                                                    RichText::new(format!(
                                                        "⚠ {} includes split {{uexp, uasset}} pairs",
                                                        r#mod.url
                                                    ))
                                                    .color(AMBER),
                                                )
                                                .show(ui, |ui| {
                                                    files.iter().for_each(|(file, kind)| {
                                                        match kind {
                                                            SplitAssetPair::MissingUasset => {
                                                                ui.label(format!("`{file}` missing matching .uasset file"));
                                                            },
                                                            SplitAssetPair::MissingUexp => {
                                                                ui.label(format!("`{file}` missing matching .uexp file"));
                                                            }
                                                        }
                                                    });
                                                });
                                            });
                                        });
                                    }

                                if let Some(unmodified_game_assets_mods) = &report.unmodified_game_assets_mods
                                    && !unmodified_game_assets_mods.is_empty() {
                                        CollapsingHeader::new(
                                            RichText::new(
                                                "⚠ Mod(s) with unmodified game assets detected",
                                            )
                                            .color(AMBER),
                                        )
                                        .default_open(true)
                                        .show(ui, |ui| {
                                            unmodified_game_assets_mods.iter().for_each(|(r#mod, files)| {
                                                CollapsingHeader::new(
                                                    RichText::new(format!(
                                                        "⚠ {} includes unmodified game assets",
                                                        r#mod.url
                                                    ))
                                                    .color(AMBER),
                                                )
                                                .show(ui, |ui| {
                                                    files.iter().for_each(|file| {
                                                        ui.label(file);
                                                    });
                                                });
                                            });
                                        });
                                    }
                            });
                    } else {
                        ui.spinner();
                        ui.label("Lint report generating...");
                    }
                });

            if !open {
                self.lint_report_window = None;
                self.lint_rid = None;
            }
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

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // VSync limits frames to the monitor's refresh rate, which may exceed 60 Hz.
        // Pace updates during rapid input without requesting any frames when idle.
        const MIN_FRAME_INTERVAL: Duration = Duration::from_nanos(16_666_667);
        if let Some(previous) = self.last_ui_frame_start
            && let Some(remaining) = MIN_FRAME_INTERVAL.checked_sub(previous.elapsed())
        {
            std::thread::sleep(remaining);
        }
        self.last_ui_frame_start = Some(Instant::now());

        if self.needs_restart
            && let Some(original_exe_path) = &self.original_exe_path
        {
            debug!("needs restart");
            self.needs_restart = false;

            debug!("restarting...");
            let _child = std::process::Command::new(original_exe_path)
                .spawn()
                .unwrap();
            debug!("created child process");

            std::process::exit(0);
        }

        // do some init things that depend on ctx so cannot be done earlier
        if !self.has_run_init {
            self.has_run_init = true;

            let theme = GuiTheme::into_egui_theme(self.state.config.gui_theme);
            ctx.memory_mut(|m| m.options.theme_preference = theme);

            message::CheckUpdates::send(self, ctx);
        }

        // message handling
        while let Ok(msg) = self.rx.try_recv() {
            // Only act on the completion of the currently running cache update.
            let cache_was_running = self.update_rid.is_some();
            let is_cache_result = matches!(&msg, message::Message::UpdateCache(_));
            // Match the exact automatic installation that is completing.
            let finishing_cache_install = matches!(&msg, message::Message::Integrate(_))
                && self.cache_install_rid.is_some()
                && self.cache_install_rid == self.integrate_rid.as_ref().map(|r| r.rid);
            msg.handle(self);
            // The shared store already contains the recovered metadata; invalidate
            // cached profile warnings and redraw names/tags immediately.
            if is_cache_result && cache_was_running && self.update_rid.is_none() {
                self.profile_issues.clear();
                self.cache_recovery_progress = None;
                self.cache_recovery_last_advance = None;
                ctx.request_repaint();
            }
            if finishing_cache_install && self.integrate_rid.is_none() {
                self.cache_install_rid = None;
                if let Some(action) = self.last_action.take() {
                    self.last_action = Some(match action.status {
                        LastActionStatus::Success(_) => {
                            LastAction::success("Cache updated; installation complete".to_string())
                        }
                        LastActionStatus::Failure(reason) => LastAction::failure(format!(
                            "Cache updated; installation failed: {reason}"
                        )),
                    });
                }
            }
            if is_cache_result
                && cache_was_running
                && self.update_rid.is_none()
                && self.integrate_rid.is_none()
                && self
                    .last_action
                    .as_ref()
                    .is_some_and(|action| matches!(&action.status, LastActionStatus::Success(_)))
            {
                // Use the same profile ordering and integration as the Install Mods button.
                if let Some(fsd_pak) = self.state.config.drg_pak_path.clone() {
                    if !self.preflight_bitfix_proxy() {
                        continue;
                    }
                    let data = &self.state.mod_data;
                    let mods = data.ordered_install_entries(&data.active_profile);
                    self.integrate_rid = Some(message::Integrate::send(
                        &mut self.request_counter,
                        self.state.store.clone(),
                        mods,
                        fsd_pak,
                        self.state.config.deref().into(),
                        self.tx.clone(),
                        ctx.clone(),
                    ));
                    self.cache_install_rid = self.integrate_rid.as_ref().map(|r| r.rid);
                    self.install_phase = message::InstallPhase::Resolving;
                    self.problematic_mod_id = None;
                    // The install is not yet successful; the live phase label is shown below.
                    self.last_action = None;
                } else {
                    self.last_action = Some(LastAction::failure(
                        "cache updated, but DRG install not found".to_string(),
                    ));
                }
            }
        }

        // Record finished edits, including asynchronously resolved mods.
        if !ctx.input(|i| i.pointer.primary_down()) {
            self.edit_history
                .sync(&self.state.mod_data, self.state.mod_data.revision());
        }
        // Preserve native text-edit undo while the user is typing.
        if !ctx.wants_keyboard_input()
            && self.integrate_rid.is_none()
            && self.update_rid.is_none()
            && self.lint_rid.is_none()
        {
            let shortcut = ctx.input(|i| {
                if !i.modifiers.command {
                    return None;
                }
                if i.key_pressed(egui::Key::Y) || (i.modifiers.shift && i.key_pressed(egui::Key::Z))
                {
                    Some(true)
                } else if !i.modifiers.shift && i.key_pressed(egui::Key::Z) {
                    Some(false)
                } else {
                    None
                }
            });
            if let Some(redo) = shortcut {
                self.apply_mod_history(redo);
            }
        }
        // begin draw

        self.show_update_window(ctx);
        self.show_provider_parameters(ctx);
        self.show_profile_windows(ctx);
        self.show_settings(ctx);
        folder_import::tick(self, ctx);
        folder_import::show_preview(self, ctx);
        self.show_lints_toggle(ctx);
        self.show_lint_report(ctx);
        // Check the configured installation without accessing the disk every frame.
        if self
            .crash_monitor
            .as_ref()
            .is_none_or(|monitor| !monitor.matches_installation(&self.state))
        {
            self.crash_monitor = Some(crash_notice::CrashMonitor::start(&self.state, ctx));
        }
        if let Some(monitor) = &mut self.crash_monitor {
            monitor.poll(&mut self.recent_crash);
        }
        crash_notice::show(ctx, &mut self.recent_crash);

        self.crash_history.show(ctx, &self.state);
        self.show_bitfix_menu(ctx);
        self.show_bitfix_conflict(ctx);
        egui::TopBottomPanel::bottom("bottom_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if crash_history::toolbar_button(ui) {
                    self.crash_history.open(&self.state);
                }
                if bitfix_menu::toolbar_button(ui) {
                    self.bitfix_menu.open = true;
                }
            ui.with_layout(egui::Layout::right_to_left(Align::TOP), |ui| {
                ui.add_enabled_ui(
                    self.integrate_rid.is_none()
                        && self.update_rid.is_none()
                        && self.lint_rid.is_none()
                        && self.self_update_rid.is_none()
                        && self.state.config.drg_pak_path.is_some(),
                    |ui| {
                        // Right-to-left toolbar: this button appears to the RIGHT of Install mods.
                        if ui
                            .button("Start DRG")
                            .on_hover_text(if self.args.is_some() {
                                "Launch using Mintfixed's custom launch command"
                            } else {
                                "Launch Deep Rock Galactic through Steam"
                            })
                            .clicked()
                        {
                            let result = if let Some(args) = &self.args {
                                match args.split_first() {
                                    Some((program, command_args)) => std::process::Command::new(program)
                                        .args(command_args)
                                        .spawn()
                                        .map(|_| ())
                                        .map_err(|err| err.to_string()),
                                    None => Err("No launch command was provided".to_owned()),
                                }
                            } else {
                                opener::open("steam://rungameid/548430")
                                    .map_err(|err| err.to_string())
                            };
                            if let Err(err) = result {
                                self.last_action = Some(LastAction::failure(format!(
                                    "Failed to launch DRG: {err}"
                                )));
                            }
                        }
                        ui.add_enabled_ui(self.state.config.drg_pak_path.is_some(), |ui| {
                            let mut button = ui.button("Install mods");
                            if self.state.config.drg_pak_path.is_none() {
                                button = button.on_disabled_hover_text(
                                    "DRG install not found. Configure it in the settings menu.",
                                );
                            }

                            if button.clicked() {
                                if !self.preflight_bitfix_proxy() {
                                    return;
                                }
                                let data = &self.state.mod_data;
                                let mods = data.ordered_install_entries(&data.active_profile);

                                self.last_action = None;
                                self.cache_install_rid = None;
                                self.install_phase = message::InstallPhase::Resolving;
                                self.integrate_rid = Some(message::Integrate::send(
                                    &mut self.request_counter,
                                    self.state.store.clone(),
                                    mods,
                                    self.state.config.drg_pak_path.as_ref().unwrap().clone(),
                                    self.state.config.deref().into(),
                                    self.tx.clone(),
                                    ctx.clone(),
                                ));
                                self.problematic_mod_id = None;
                            }
                        });

                        ui.add_enabled_ui(self.state.config.drg_pak_path.is_some(), |ui| {
                            let mut button = ui.button("Uninstall mods");
                            if self.state.config.drg_pak_path.is_none() {
                                button = button.on_disabled_hover_text(
                                    "DRG install not found. Configure it in the settings menu.",
                                );
                            }
                            if button.clicked() {
                                self.last_action = None;
                                if let Some(pak_path) = &self.state.config.drg_pak_path {
                                    let mut mods = HashSet::default();
                                    let active_profile = self.state.mod_data.active_profile.clone();
                                    self.state.mod_data.for_each_enabled_mod(
                                        &active_profile,
                                        |mc| {
                                            if let Some(modio_id) = self
                                                .state
                                                .store
                                                .get_mod_info(&mc.spec)
                                                .and_then(|i| i.modio_id)
                                            {
                                                mods.insert(modio_id);
                                            }
                                        },
                                    );

                                    debug!("uninstalling mods: pak_path = {}", pak_path.display());
                                    self.last_action = Some(match uninstall(pak_path, mods) {
                                        Ok(()) => LastAction::success(
                                            "Successfully uninstalled mods".to_string(),
                                        ),
                                        Err(e) => LastAction::failure(format!(
                                            "Failed to uninstall mods: {e}"
                                        )),
                                    })
                                }
                            }
                        });

                        if ui
                            .button("Update cache")
                            .on_hover_text(
                                "Archives unused cache entries, checks for mod updates, then installs the active profile",
                            )
                            .clicked()
                        {
                            let referenced = crate::mod_identity::all_specs(&self.state.mod_data);
                            match self.state.store.archive_unreferenced(&referenced) {
                                Ok(archived) => {
                                    message::UpdateCache::send(self, ctx);
                                    if archived > 0 {
                                        self.last_action = Some(LastAction::success(format!(
                                            "Archived {archived} unused cached mods; updating cache"
                                        )));
                                    }
                                    self.problematic_mod_id = None;
                                }
                                Err(error) => {
                                    self.last_action = Some(LastAction::failure(format!(
                                        "Cache cleanup failed: {error}"
                                    )));
                                }
                            }
                        }
                    },
                );
                if self.integrate_rid.is_some() {
                    if ui.button("Cancel").clicked() {
                        self.integrate_rid.take().unwrap().handle.abort();
                        self.install_phase = message::InstallPhase::Resolving;
                        if self.cache_install_rid.take().is_some() {
                            self.last_action = Some(LastAction::failure(
                                "Cache updated; installation cancelled".to_string(),
                            ));
                        }
                    }
                    ui.spinner();
                }
                if self.update_rid.is_some() {
                    if ui.button("Cancel").clicked() {
                        self.update_rid.take().unwrap().handle.abort();
                        self.cache_recovery_progress = None;
                        self.cache_recovery_last_advance = None;
                        self.last_action = Some(LastAction::failure("Cache update cancelled".to_string()));
                    }
                    ui.spinner();
                }
                if ui
                    .button("Lint mods")
                    .on_hover_text("Lint mods in the current profile")
                    .clicked()
                {
                    self.lints_toggle_window = Some(WindowLintsToggle);
                }
                let settings_icon = egui::Image::new(egui::include_image!("../../assets/icons/settings.svg"))
                    .fit_to_exact_size(egui::vec2(14.0, 14.0))
                    .tint(ui.visuals().text_color());
                if ui
                    .add(egui::Button::image(settings_icon))
                    .on_hover_text("Open settings")
                    .clicked()
                {
                    self.settings_window = Some(WindowSettings::new(&self.state));
                }
                if let Some(available_update) = &self.available_update
                    && ui
                        .button(egui::RichText::new("\u{26A0}").color(ui.visuals().warn_fg_color))
                        .on_hover_text(format!(
                            "Update available: {}\n{}",
                            available_update.tag_name, available_update.html_url
                        ))
                        .clicked()
                {
                    ui.ctx()
                        .open_url(egui::OpenUrl::new_tab(&available_update.html_url));
                }
                // Render fixed action buttons first (right-to-left), then confine
                // status text to the actual remainder of the toolbar row.
                let status_width = ui.available_width().max(0.0);
                if status_width >= 24.0 {
                    ui.allocate_ui_with_layout(
                        egui::vec2(status_width, ui.spacing().interact_size.y),
                        egui::Layout::left_to_right(Align::Center),
                        |ui| {
                            if self.update_rid.is_some() {
                                let progress = if let Some((completed, total)) = self.cache_recovery_progress {
                                    // Repaint when the inactivity threshold expires, even if
                                    // mod.io hasn't sent another progress event.
                                    let waiting = completed < total
                                        && self.cache_recovery_last_advance.is_some_and(|last| {
                                            let elapsed = last.elapsed();
                                            let threshold = Duration::from_secs(10);
                                            if elapsed < threshold {
                                                ui.ctx().request_repaint_after(threshold - elapsed);
                                                false
                                            } else {
                                                true
                                            }
                                        });
                                    format!(
                                        "Recovering mod.io metadata: ({completed}/{total}){}",
                                        if waiting { " (waiting for mod.io)" } else { "" }
                                    )
                                } else {
                                    "Updating provider caches...".to_owned()
                                };
                                if status_message(ui, &progress) {
                                    self.status_details = Some(progress);
                                }
                            } else if self.integrate_rid.is_some() {
                                let progress = match self.install_phase {
                                    message::InstallPhase::Resolving => "Resolving mod versions and dependencies...",
                                    message::InstallPhase::Downloading => "Downloading / checking cached mods...",
                                    message::InstallPhase::Integrating => "Writing mods_P.pak...",
                                };
                                if status_message(ui, progress) {
                                    self.status_details = Some(progress.to_owned());
                                }
                            } else if let Some(last_action) = &self.last_action {
                                // Hide the badge as well if no room remains for it.
                                if ui.available_width() >= 64.0 {
                                    let (color, message, outcome) = match &last_action.status {
                                        LastActionStatus::Success(message) => (Color32::LIGHT_GREEN, message, "Success"),
                                        LastActionStatus::Failure(message) => (Color32::LIGHT_RED, message, "Error"),
                                    };
                                    ui.label(
                                        egui::RichText::new("STATUS")
                                            .color(Color32::BLACK)
                                            .background_color(color),
                                    );
                                    ui.ctx().request_repaint_after(Duration::from_secs(1));
                                    let summary = format!("({}): {}", last_action.timeago(), message);
                                    if status_message(ui, &summary) {
                                        self.status_details = Some(format!("{outcome} {summary}"));
                                    }
                                }
                            }
                        },
                    );
                }
            });
            });
        });
        if let Some(details) = self.status_details.as_ref() {
            let mut open = true;
            egui::Window::new("Status details")
                .open(&mut open)
                .resizable(true)
                .default_width(540.0)
                .show(ctx, |ui| {
                    if ui.button("Copy message").clicked() {
                        ui.ctx().copy_text(details.clone());
                    }
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .max_height(400.0)
                        .show(ui, |ui| {
                            ui.add(egui::Label::new(details.as_str()).wrap());
                        });
                });
            if !open {
                self.status_details = None;
            }
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.integrate_rid.is_some() || self.update_rid.is_some() || self.lint_rid.is_some()
            {
                ui.disable();
            }
            // profile selection

            let mut group_changed = false;
            let mut group_success = None;
            let buttons = |ui: &mut Ui, mod_data: &mut ModData| {
                (group_changed, group_success) = group_controls::controls(
                    ui,
                    mod_data,
                    &mut self.new_group_name,
                    self.pending_group_files > 0,
                );

                // TODO find better icon, flesh out multiple-view usage, fix GUI locking
                /*
                if ui
                    .button("pop out")
                    .on_hover_text_at_pointer("pop out")
                    .clicked()
                {
                    self.open_profiles.insert(mod_data.active_profile.clone());
                }
                */
            };

            let mut imported_profile = None;
            let profile_import_button = |ui: &mut Ui, mod_data: &mut ModData, modified: &mut bool| {
                imported_profile = profile_import::ui_inline(ui, mod_data, modified);
            };
            let mut copy_feedback: Option<Result<String, String>> = None;
            let profile_copy_button = |ui: &mut Ui, mod_data: &mut ModData| {
                copy_feedback = profile_management::copy_options(ui, mod_data);
                copy_feedback.clone()
            };
            let profile_toolbar_changed = named_combobox::ui(
                ui,
                "profile",
                self.state.mod_data.deref_mut().deref_mut(),
                Some(buttons),
                Some(profile_import_button),
                Some(profile_copy_button),
                self.resolve_mod_rid.is_none() && self.pending_group_files == 0,
            );
            if profile_toolbar_changed || group_changed {
                self.state.mod_data.save().unwrap();
                self.edit_history.reset(&self.state.mod_data, self.state.mod_data.revision());
            }
            if let Some(message) = group_success {
                self.last_action = Some(LastAction::success(message));
            }
            if let Some(message) = imported_profile {
                self.last_action = Some(LastAction::success(message));
            }
            if let Some(result) = copy_feedback {
                self.last_action = Some(match result {
                    Ok(message) => LastAction::success(message),
                    Err(message) => LastAction::failure(message),
                });
            }

            ui.separator();

            let mut open_local_folder_import = false;
            ui.with_layout(egui::Layout::right_to_left(Align::TOP), |ui| {
                if self.resolve_mod_rid.is_some() {
                    ui.spinner();
                }
                if ui
                    .add_enabled(
                        self.state.config.local_mod_folder.is_some() && self.resolve_mod_rid.is_none(),
                        egui::Button::new("Import .pak/.zip"),
                    )
                    .on_hover_text("Scan the local mods folder configured in Settings; manage exclusions in the import dialog")
                    .clicked()
                {
                    open_local_folder_import = true;
                }
                ui.with_layout(ui.layout().with_main_justify(true), |ui| {
                    // define multiline layouter to be able to show multiple lines in a single line widget
                    let font_id = FontSelection::default().resolve(ui.style());
                    let text_color = ui.visuals().widgets.inactive.text_color();
                    let mut multiline_layouter = move |ui: &Ui, text: &str, wrap_width: f32| {
                        let layout_job = LayoutJob::simple(
                            text.to_string(),
                            font_id.clone(),
                            text_color,
                            wrap_width,
                        );
                        ui.fonts(|f| f.layout_job(layout_job))
                    };

                    let resolve = ui.add_enabled(
                        self.resolve_mod_rid.is_none(),
                        egui::TextEdit::singleline(&mut self.resolve_mod)
                            .layouter(&mut multiline_layouter)
                            .hint_text("Add mod..."),
                    );
                    if is_committed(&resolve) {
                        if looks_like_structured_json(&self.resolve_mod) {
                            self.last_action = Some(LastAction::failure(
                                "JSON cannot be added as mod URLs. Use Profiles > Import Profile (JSON).".into(),
                            ));
                        } else {
                            message::ResolveMods::send(self, ctx, self.parse_mods(), false);
                            self.problematic_mod_id = None;
                        }
                    }
                });
            });

            if open_local_folder_import
                && let Some(folder) = self.state.config.local_mod_folder.clone() {
                    folder_import::open_preview(self, ctx, folder);
                }
            let profile = self.state.mod_data.active_profile.clone();

            ui.horizontal(|ui| {
                let (mut sort_category, mut is_ascending) = self
                    .get_sorting_config()
                    .map(|c| (Some(c.sort_category), c.is_ascending))
                    .unwrap_or_default();

                let selected_text = match sort_category {
                    None => "Manual".to_owned(),
                    Some(category) => format!(
                        "{} {}",
                        category.as_str(),
                        if is_ascending { "⏶" } else { "⏷" },
                    ),
                };

                let mut clicked = false;
                egui::ComboBox::from_id_salt("mod_sort_by")
                    .selected_text(format!("Sort by: {selected_text}"))
                    .width(180.0)
                    .show_ui(ui, |ui| {
                        if ui.radio_value(&mut sort_category, None, "Manual").clicked() {
                            clicked = true;
                        }
                        for category in SortBy::iter() {
                            let mut radio_label = category.as_str().to_owned();
                            if sort_category == Some(category) {
                                radio_label.push_str(if is_ascending { " ⏶" } else { " ⏷" });
                            }
                            let resp = ui.radio_value(&mut sort_category, Some(category), radio_label);
                            if resp.clicked() {
                                clicked = true;
                                if resp.changed() {
                                    is_ascending = true;
                                } else {
                                    // Clicking the active category reverses its direction.
                                    is_ascending = !is_ascending;
                                }
                            }
                        }
                    });

                if clicked {
                    self.update_sorting_config(sort_category, is_ascending);
                }

                // Keep history controls beside sorting while preserving their shortcuts.
                let history_enabled = self.resolve_mod_rid.is_none()
                    && self.integrate_rid.is_none()
                    && self.update_rid.is_none()
                    && self.lint_rid.is_none();
                let undo_image = egui::Image::new(egui::include_image!("../../assets/icons/undo.svg"))
                    .fit_to_exact_size(egui::vec2(17.0, 17.0))
                    .tint(ui.visuals().text_color());
                if ui.add_enabled(
                    history_enabled && self.edit_history.can_undo(),
                    egui::Button::image(undo_image),
                )
                .on_hover_text("Undo mod-list change (Ctrl+Z)")
                .clicked() {
                    self.apply_mod_history(false);
                }
                let redo_image = egui::Image::new(egui::include_image!("../../assets/icons/redo.svg"))
                    .fit_to_exact_size(egui::vec2(17.0, 17.0))
                    .tint(ui.visuals().text_color());
                if ui.add_enabled(
                    history_enabled && self.edit_history.can_redo(),
                    egui::Button::image(redo_image),
                )
                .on_hover_text("Redo mod-list change (Ctrl+Y / Ctrl+Shift+Z)")
                .clicked() {
                    self.apply_mod_history(true);
                }

                ui.add_space(16.);
                // TODO: actually implement mod groups.
                let search_string = &mut self.search_string;
                let lower = search_string.to_lowercase();
                let any_matches = self.state.mod_data.any_mod(&profile, |mc, _| {
                    self.state
                        .store
                        .get_mod_info(&mc.spec)
                        .map(|i| i.name.to_lowercase().contains(&lower))
                        .unwrap_or(false)
                });

                let mut text_edit = egui::TextEdit::singleline(search_string).hint_text("Search");
                if !any_matches {
                    text_edit = text_edit.text_color(ui.visuals().error_fg_color);
                }
                let res = ui
                    .scope_builder(
                        egui::UiBuilder::new().layout(egui::Layout::bottom_up(Align::RIGHT)),
                        |ui| ui.add(text_edit),
                    )
                    .inner;
                if res.changed() {
                    self.scroll_to_match = true;
                }
                if res.lost_focus()
                    && ui.input(|i| {
                        i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Escape)
                    })
                {
                    search_string.clear();
                    self.scroll_to_match = false;
                } else if self.focus_search {
                    res.request_focus();
                    self.focus_search = false;
                }
            });
            ui.add_space(4.);

            Drag::cancel_if_invalid(ctx, &profile, self.get_sorting_config().is_none());
            let file_group = self.ui_profile(ui, &profile);

            // must access memory outside of input lock to prevent deadlock
            let is_anything_focused = ctx.memory(|m| m.focused().is_some());
            ctx.input(|i| {
                if !i.raw.dropped_files.is_empty()
                    && self.integrate_rid.is_none()
                    && self.update_rid.is_none()
                {
                    if let Some((group, prepend)) = &file_group {
                        let paths = i
                            .raw
                            .dropped_files
                            .iter()
                            .filter_map(|f| f.path.clone())
                            .collect();
                        group_controls::drop_files(
                            self,
                            ctx,
                            profile.clone(),
                            group.clone(),
                            *prepend,
                            paths,
                        );
                    } else {
                        let mut mods = String::new();
                        for f in i
                            .raw
                            .dropped_files
                            .iter()
                            .filter_map(|f| f.path.as_ref().map(|p| p.to_string_lossy()))
                        {
                            mods.push_str(&f);
                            mods.push('\n');
                        }

                        self.resolve_mod = mods.trim().to_string();
                        message::ResolveMods::send(self, ctx, self.parse_mods(), false);
                        self.problematic_mod_id = None;
                    }
                }
                for e in &i.events {
                    match e {
                        egui::Event::Paste(s) => {
                            if self.integrate_rid.is_none()
                                && self.update_rid.is_none()
                                && self.lint_rid.is_none()
                                && !is_anything_focused
                            {
                                // A global paste with no focused text field is a mod-list shortcut.
                                // Never feed profile/group JSON keys (such as `id`) to mod providers.
                                if looks_like_structured_json(s) {
                                    self.last_action = Some(LastAction::failure(
                                        "Profile JSON detected. Open Profiles > Import Profile (JSON), click its text field, then paste.".into(),
                                    ));
                                } else {
                                    self.resolve_mod = s.trim().to_string();
                                    message::ResolveMods::send(self, ctx, self.parse_mods(), false);
                                }
                            }
                        }
                        egui::Event::Text(text)
                            if !is_anything_focused && !text.trim().is_empty() => {
                                self.search_string = text.to_string();
                                self.scroll_to_match = true;
                                self.focus_search = true;
                            }
                        _ => {}
                    }
                }
            });
        });
        self.show_group_mod_removal_confirmation(ctx);
        if !ctx.input(|i| i.pointer.primary_down()) {
            self.edit_history
                .sync(&self.state.mod_data, self.state.mod_data.revision());
        }
    }
}

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
