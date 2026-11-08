//! Best-effort BP-Crash-Reporter autolaunch outside DllMain.
//! Native Windows keeps the upstream watcher for post-exit load triage.
//! Proton/Wine uses --once so Steam's Wine session can exit normally.
use anyhow::{Context, Result};
use std::{
    fs::{self, OpenOptions},
    os::windows::process::CommandExt,
    process::{Command, Stdio},
};
use tracing::{info, warn};
use windows::{
    Win32::{
        Foundation::CloseHandle,
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
                TH32CS_SNAPPROCESS,
            },
            LibraryLoader::{GetModuleHandleW, GetProcAddress},
        },
    },
    core::{s, w},
};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
const SESSION_MONITOR_SCRIPT: &str = include_str!("../assets/bpcrash-session.ps1");

/// The one-shot injector cannot perform load triage after DRG dies. Launch a
/// separate, finite-lived process to wait for this game's PID, run --triage,
/// and exit. This helper is deliberately not used under Proton/Wine.
fn launch_native_triage_monitor(
    dir: &std::path::Path,
    reporter_exe: &std::path::Path,
    game_exe: &std::path::Path,
    injector_pid: u32,
    start_unix_seconds: u64,
) -> Result<u32> {
    let script = dir.join("bpcrash-session.ps1");
    if fs::read_to_string(&script).ok().as_deref() != Some(SESSION_MONITOR_SCRIPT) {
        fs::write(&script, SESSION_MONITOR_SCRIPT)?;
    }
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("bpcrash-loader.log"))?;
    let stderr = stdout.try_clone()?;
    let mut cmd = Command::new("powershell.exe");
    cmd.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
    ])
    .arg(&script)
    .arg("-GameProcessId")
    .arg(std::process::id().to_string())
    .arg("-InjectorProcessId")
    .arg(injector_pid.to_string())
    .arg("-GameExe")
    .arg(game_exe)
    .arg("-ReporterExe")
    .arg(reporter_exe)
    .arg("-StartUnixSeconds")
    .arg(start_unix_seconds.to_string())
    .current_dir(dir)
    .stdin(Stdio::null())
    .stdout(Stdio::from(stdout))
    .stderr(Stdio::from(stderr));
    // Where Steam's job permits it, the helper must survive DRG's crash long
    // enough to read FSD.log and finish triage outside the game's job.
    let child = match cmd
        .creation_flags(CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB)
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            warn!(
                "BP-Crash-Reporter: helper job breakaway unavailable ({err}); using normal process"
            );
            cmd.creation_flags(CREATE_NO_WINDOW).spawn()?
        }
    };
    Ok(child.id())
}

/// Running a persistent Windows loader inside Wine/Proton can keep Steam's
/// wineserver alive after DRG quits. Check Wine's ntdll export (not just env
/// variables, which Steam/Proton configurations can change).
fn running_under_wine() -> bool {
    if std::env::var_os("WINEPREFIX").is_some()
        || std::env::var_os("STEAM_COMPAT_DATA_PATH").is_some()
    {
        return true;
    }
    unsafe {
        GetModuleHandleW(w!("ntdll.dll"))
            .ok()
            .and_then(|ntdll| GetProcAddress(ntdll, s!("wine_get_version")))
            .is_some()
    }
}

/// Returns true for *any* running bpcrash.exe, regardless of where it was
/// manually installed. Fail closed if process enumeration fails, so we don't
/// create a duplicate watcher after an access or OS error.
fn reporter_process_running() -> Result<bool> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }?;
    let result = (|| -> Result<bool> {
        let mut entry = PROCESSENTRY32W::default();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        unsafe { Process32FirstW(snapshot, &mut entry) }?;
        loop {
            let end = entry
                .szExeFile
                .iter()
                .position(|&ch| ch == 0)
                .unwrap_or(entry.szExeFile.len());
            if String::from_utf16_lossy(&entry.szExeFile[..end]).eq_ignore_ascii_case("bpcrash.exe")
            {
                return Ok(true);
            }
            if unsafe { Process32NextW(snapshot, &mut entry) }.is_err() {
                return Ok(false);
            }
        }
    })();
    let _ = unsafe { CloseHandle(snapshot) };
    result
}

pub(crate) fn autolaunch() {
    if let Err(err) = try_autolaunch() {
        warn!("BP-Crash-Reporter autolaunch failed (game continues): {err:#}");
    }
}

fn try_autolaunch() -> Result<()> {
    let game_exe = std::env::current_exe()?;
    let bin_dir = game_exe.parent().context("missing DRG binary directory")?;
    let dir = bin_dir.join("mintfixed-bpcrash");
    let exe = dir.join("bpcrash.exe");
    if dir.join("disabled").exists() {
        info!("BP-Crash-Reporter: autolaunch skipped (disabled)");
        return Ok(());
    }
    if !exe.is_file() || !dir.join("bpcrash.cfg").is_file() {
        info!("BP-Crash-Reporter: autolaunch skipped (reporter not installed)");
        return Ok(());
    }

    // A running manual watcher can both inject and perform post-exit triage.
    // Do not spawn another watcher regardless of its installation location.
    if reporter_process_running()? {
        info!("existing Elytras bpcrash.exe detected; skipping Mintfixed autolaunch");
        return Ok(());
    }

    let under_wine = running_under_wine();
    let already_injected = unsafe { GetModuleHandleW(w!("bpcrash_live.dll")) }.is_ok();
    // On Proton, avoid another injection attempt. On native Windows we MUST
    // still start the watcher even when the DLL is already loaded: the watcher
    // recognizes the existing DLL and tracks DRG's exit for load triage.
    if already_injected && under_wine {
        info!("BP-Crash-Reporter is already injected under Wine; skipping duplicate injection");
        return Ok(());
    }
    if already_injected {
        info!(
            "BP-Crash-Reporter is already injected; starting one-shot session for post-crash triage"
        );
    }

    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("bpcrash-loader.log"))?;
    let stderr = stdout.try_clone()?;
    let mut cmd = Command::new(&exe);
    // A native-Windows watcher attaches to the current DRG process (even if
    // already injected), waits for exit, and appends Elytras' load triage.
    // Under Proton the watcher must not remain alive in Wine; --once preserves
    // the working injection, but post-exit triage needs a Linux-native helper.
    cmd.arg("--once");
    let start_unix_seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    cmd.current_dir(&dir)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    let child = cmd.spawn()?;
    if let Err(err) = fs::write(dir.join("bpcrash.pid"), child.id().to_string()) {
        warn!("BP-Crash-Reporter launched, but could not record its pid: {err}");
    }
    if under_wine {
        info!(
            "started BP-Crash-Reporter one-shot injector (Wine/Proton, pid {})",
            child.id()
        );
    } else {
        info!(
            "started BP-Crash-Reporter Windows one-shot injector with post-exit triage (pid {})",
            child.id()
        );
    }
    if !under_wine {
        match launch_native_triage_monitor(&dir, &exe, &game_exe, child.id(), start_unix_seconds) {
            Ok(monitor_pid) => info!(
                "BP-Crash-Reporter: watching game PID {} for post-exit triage (helper PID {})",
                std::process::id(),
                monitor_pid
            ),
            Err(err) => warn!(
                "BP-Crash-Reporter: one-shot injection active, but post-exit triage helper failed: {err:#}"
            ),
        }
    }
    Ok(()) // Never block WinMain while the reporter attaches.
}
