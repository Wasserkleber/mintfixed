// Mechanical extraction from hook/src/hooks/mod.rs; detour behavior is unchanged.
use super::WinMain;
use crate::LOG_GUARD;

pub(super) fn detour_main(
    h_instance: *mut (),
    h_prev_instance: *mut (),
    lp_cmd_line: *mut (),
    n_cmd_show: i32,
    cmd_line: *const (),
) -> i32 {
    // Start outside DllMain, before the original game WinMain.
    crate::crash_reporter::autolaunch();
    let ret = unsafe {
        WinMain.call(
            h_instance,
            h_prev_instance,
            lp_cmd_line,
            n_cmd_show,
            cmd_line,
        )
    };

    // about to exit, drop log guard
    drop(LOG_GUARD.with_borrow_mut(|g| g.take()).unwrap());

    ret
}
