// Mechanical extraction from hook/src/hooks/mod.rs; detour behavior is unchanged.
use crate::{
    globals,
    ue::{self, FLinearColor, UObject},
};
use std::{ffi::c_void, ptr::NonNull};

pub(super) unsafe extern "system" fn exec_get_mod_json(
    _context: *mut ue::UObject,
    stack: *mut ue::kismet::FFrame,
    _result: *mut c_void,
) {
    unsafe {
        let stack = stack.as_mut().unwrap();

        let _ctx: Option<&ue::UObject> = stack.arg();

        stack.most_recent_property_address = std::ptr::null();
        let ret: Option<ue::FString> = stack.arg();
        let ret_address = (stack.most_recent_property_address as *mut ue::FString)
            .as_mut()
            .unwrap();

        let json = serde_json::to_string(&globals().meta).unwrap();

        ret_address.clear();
        ret_address.extend_from_slice(&json.encode_utf16().chain([0]).collect::<Vec<_>>());

        std::mem::forget(ret);

        if !stack.code.is_null() {
            stack.code = stack.code.add(1);
        }
    }
}

pub(super) unsafe extern "system" fn exec_get_update_available(
    _context: *mut ue::UObject,
    stack: *mut ue::kismet::FFrame,
    _result: *mut c_void,
) {
    unsafe {
        let stack = stack.as_mut().unwrap();

        let _ctx: Option<&ue::UObject> = stack.arg();

        stack.arg::<bool>();
        let done_checking = &mut *(stack.most_recent_property_address as *mut bool);
        *done_checking = true;

        stack.arg::<bool>();
        let available = &mut *(stack.most_recent_property_address as *mut bool);
        *available = false;

        std::mem::forget(stack.arg::<ue::FString>());
        let version = &mut *(stack.most_recent_property_address as *mut ue::FString);
        *version = ue::FString::new();

        if !stack.code.is_null() {
            stack.code = stack.code.add(1);
        }
    }
}

pub(super) unsafe extern "system" fn exec_print_string(
    _context: *mut ue::UObject,
    stack: *mut ue::kismet::FFrame,
    _result: *mut c_void,
) {
    unsafe {
        let stack = stack.as_mut().unwrap();

        let _ctx: Option<NonNull<UObject>> = stack.arg();
        let string: ue::FString = stack.arg();
        let _print_to_screen: bool = stack.arg();
        let _print_to_log: bool = stack.arg();
        let _color: FLinearColor = stack.arg();
        let _duration: f32 = stack.arg();

        println!("PrintString({string})");

        if !stack.code.is_null() {
            stack.code = stack.code.add(1);
        }
    }
}
