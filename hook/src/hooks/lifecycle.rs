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
    crate::startup_checkpoint("WinMain detour: entered");
    // Start outside DllMain, before the original game WinMain.
    crate::startup_checkpoint("BP-Crash-Reporter autolaunch: BEGIN");
    crate::crash_reporter::autolaunch();
    crate::startup_checkpoint("BP-Crash-Reporter autolaunch: returned");
    crate::startup_checkpoint("Original WinMain call: BEGIN");
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
    crate::startup_checkpoint("Original WinMain call: returned; dropping tracing guard");
    drop(LOG_GUARD.with_borrow_mut(|g| g.take()).unwrap());

    ret
}
