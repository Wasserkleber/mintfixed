#![allow(clippy::missing_transmute_annotations)]

mod kismet_callbacks;
mod lifecycle;
mod memory;
mod save_game;
mod server_list;

use std::{ffi::c_void, path::Path};

use anyhow::{Context, Result};
use mint_lib::DRGInstallationType;

use crate::{globals, ue};

retour::static_detour! {
    static HookUFunctionBind: unsafe extern "system" fn(*mut ue::UFunction);
    static SaveGameToSlot: unsafe extern "system" fn(*const USaveGame, *const ue::FString, i32) -> bool;
    static LoadGameFromSlot: unsafe extern "system" fn(*const ue::FString, i32) -> *const USaveGame;
    static DoesSaveGameExist: unsafe extern "system" fn(*const ue::FString, i32) -> bool;
    static UObjectTemperatureComponentTimerCallback: unsafe extern "system" fn(*mut c_void);
    static WinMain: unsafe extern "system" fn(*mut (), *mut (), *mut (), i32, *const ()) -> i32;

}

#[repr(C)]
pub struct USaveGame;

pub type FnSaveGameToMemory =
    unsafe extern "system" fn(*const USaveGame, *mut ue::TArray<u8>) -> bool;
pub type FnLoadGameFromMemory =
    unsafe extern "system" fn(*const ue::TArray<u8>) -> *const USaveGame;

type ExecFn = unsafe extern "system" fn(*mut ue::UObject, *mut ue::kismet::FFrame, *mut c_void);

pub unsafe fn initialize() -> Result<()> {
    let hooks = [
        (
            "/Game/_mint/BPL_MINT.BPL_MINT_C:Get Mod JSON",
            kismet_callbacks::exec_get_mod_json as ExecFn,
        ),
        (
            "/Game/_mint/BPL_MINT.BPL_MINT_C:GetUpdateAvailable",
            kismet_callbacks::exec_get_update_available as ExecFn,
        ),
        (
            "/Script/Engine.KismetSystemLibrary:PrintString",
            kismet_callbacks::exec_print_string as ExecFn,
        ),
    ]
    .iter()
    .chain(server_list::kismet_hooks().iter())
    .cloned()
    .collect::<std::collections::HashMap<_, ExecFn>>();

    unsafe {
        WinMain.initialize(
            std::mem::transmute(globals().resolution.core.as_ref().unwrap().main.0),
            lifecycle::detour_main,
        )?;
        WinMain.enable()?;

        HookUFunctionBind.initialize(
            std::mem::transmute(globals().resolution.core.as_ref().unwrap().ufunction_bind.0),
            move |function| {
                HookUFunctionBind.call(function);
                if let Some(function) = function.as_mut() {
                    let path = function
                        .ustruct
                        .ufield
                        .uobject
                        .uobject_base_utility
                        .uobject_base
                        .get_path_name(None);
                    if let Some(hook) = hooks.get(path.as_str()) {
                        function
                            .function_flags
                            .insert(ue::EFunctionFlags::FUNC_Native);
                        function.func = *hook;
                    }
                }
            },
        )?;
        HookUFunctionBind.enable()?;

        server_list::init_hooks()?;

        let installation_type = DRGInstallationType::from_exe_path()?;

        match installation_type {
            DRGInstallationType::Steam => {
                if let Ok(address) = &globals().resolution.disable {
                    memory::patch_mem(
                        (address.0 as *mut u8).add(29),
                        [0xB8, 0x01, 0x00, 0x00, 0x00],
                    )?;
                }
            }
            DRGInstallationType::Xbox => {
                let saves_dir = std::env::current_exe()
                    .ok()
                    .as_deref()
                    .and_then(Path::parent)
                    .and_then(Path::parent)
                    .and_then(Path::parent)
                    .context("could not determine save location")?
                    .join("Saved")
                    .join("SaveGames");
                save_game::SAVES_DIR.get_or_init(|| saves_dir);

                if let Ok(save_game) = &globals().resolution.save_game {
                    SaveGameToSlot
                        .initialize(
                            std::mem::transmute(save_game.save_game_to_slot.0),
                            save_game::save_game_to_slot_detour,
                        )?
                        .enable()?;
                    LoadGameFromSlot
                        .initialize(
                            std::mem::transmute(save_game.load_game_from_slot.0),
                            save_game::load_game_from_slot_detour,
                        )?
                        .enable()?;

                    DoesSaveGameExist
                        .initialize(
                            std::mem::transmute(save_game.does_save_game_exist.0),
                            save_game::does_save_game_exist_detour,
                        )?
                        .enable()?;
                }
            }
        }
    }
    Ok(())
}
