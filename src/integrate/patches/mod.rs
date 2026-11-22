// Unreal-specific patching internals. Keep integration orchestration in integrate.rs.
// Each child retains the original implementation and imports the same parent types.
use super::*;

mod imports;
mod modding_ui;
mod player_controller;
mod server_browser;
mod server_rules;
mod spawn_mods;

use self::imports::{Import, get_import};
pub(super) use self::modding_ui::{patch_modding_tab, patch_modding_tab_item};
pub(super) use self::player_controller::hook_pcb;
pub(super) use self::server_browser::patch_server_list_entry;
pub(super) use self::server_rules::patch;
pub(super) use self::spawn_mods::{SPAWN_MODS_PATH, patch_spawn_mods};
