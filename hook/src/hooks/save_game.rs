// Mechanical extraction from hook/src/hooks/mod.rs; detour behavior is unchanged.
use super::{DoesSaveGameExist, LoadGameFromSlot, SaveGameToSlot, USaveGame};
use crate::{globals, ue};
use fs_err as fs;
use std::{path::PathBuf, sync::OnceLock};

pub(super) static SAVES_DIR: OnceLock<PathBuf> = OnceLock::new();

fn get_path_for_slot(slot_name: &ue::FString) -> Option<PathBuf> {
    let mut str_path = slot_name.to_string();
    str_path.push_str(".sav");

    let path = std::path::Path::new(&str_path);
    let mut normalized_path = SAVES_DIR.get().unwrap().clone();

    for component in path.components() {
        if let std::path::Component::Normal(c) = component {
            normalized_path.push(c)
        }
    }

    Some(normalized_path)
}

pub(super) fn save_game_to_slot_detour(
    save_game_object: *const USaveGame,
    slot_name: *const ue::FString,
    user_index: i32,
) -> bool {
    unsafe {
        let slot_name = &*slot_name;
        if slot_name.to_string() == "Player" {
            SaveGameToSlot.call(save_game_object, slot_name, user_index)
        } else {
            let mut data: ue::TArray<u8> = Default::default();

            if !(globals().save_game_to_memory())(save_game_object, &mut data) {
                return false;
            }

            let Some(path) = get_path_for_slot(slot_name) else {
                return false;
            };

            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).ok();
            }

            fs::write(path, data.as_slice()).is_ok()
        }
    }
}

pub(super) fn load_game_from_slot_detour(
    slot_name: *const ue::FString,
    user_index: i32,
) -> *const USaveGame {
    unsafe {
        let slot_name = &*slot_name;
        if slot_name.to_string() == "Player" {
            LoadGameFromSlot.call(slot_name, user_index)
        } else if let Some(data) = get_path_for_slot(slot_name).and_then(|path| fs::read(path).ok())
        {
            (globals().load_game_from_memory())(&ue::TArray::from(data.as_slice()))
        } else {
            std::ptr::null()
        }
    }
}

pub(super) fn does_save_game_exist_detour(slot_name: *const ue::FString, user_index: i32) -> bool {
    unsafe {
        let slot_name = &*slot_name;
        if slot_name.to_string() == "Player" {
            DoesSaveGameExist.call(slot_name, user_index)
        } else if let Some(path) = get_path_for_slot(slot_name) {
            path.exists()
        } else {
            false
        }
    }
}
