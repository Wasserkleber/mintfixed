//! Windows process enumeration and termination restricted to verified owned PIDs.
use anyhow::{Context, Result, bail};
use fs_err as fs;
use std::path::Path;
use tracing::info;
use windows::{
    Win32::{
        Foundation::{CloseHandle, WAIT_OBJECT_0},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
                TH32CS_SNAPPROCESS,
            },
            Threading::{
                OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
                PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, QueryFullProcessImageNameW,
                TerminateProcess, WaitForSingleObject,
            },
        },
    },
    core::PWSTR,
};

/// A Windows watcher should never be terminated while DRG is running.
#[cfg(target_os = "windows")]
pub(super) fn target_game_is_running(exe_name: &str) -> Result<bool> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
        .context("cannot enumerate running processes")?;
    let result = (|| -> Result<bool> {
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        unsafe { Process32FirstW(snapshot, &mut entry) }
            .context("failed to enumerate Windows processes")?;
        loop {
            let end = entry
                .szExeFile
                .iter()
                .position(|&ch| ch == 0)
                .unwrap_or(entry.szExeFile.len());
            if String::from_utf16_lossy(&entry.szExeFile[..end]).eq_ignore_ascii_case(exe_name) {
                return Ok(true);
            }
            if unsafe { Process32NextW(snapshot, &mut entry) }.is_err() {
                break;
            }
        }
        Ok(false)
    })();
    let _ = unsafe { CloseHandle(snapshot) };
    result
}

/// Stop only Mintfixed's recorded watcher if the PID is still the exact installed
/// executable. Never terminate a manually installed reporter elsewhere.
#[cfg(target_os = "windows")]
pub(crate) fn stop(bin_dir: &Path) -> Result<()> {
    let dir = bin_dir.join("mintfixed-bpcrash");
    let pid_file = dir.join("bpcrash.pid");
    let pid = match fs::read_to_string(&pid_file)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
    {
        Some(pid) => pid,
        None => {
            info!("BP-Crash-Reporter: no Mintfixed-managed process to stop");
            return Ok(());
        }
    };
    let handle = match unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | PROCESS_SYNCHRONIZE,
            false,
            pid,
        )
    } {
        Ok(handle) => handle,
        Err(_) => {
            info!("BP-Crash-Reporter: recorded PID {pid} is no longer available");
            let _ = fs::remove_file(pid_file);
            return Ok(());
        }
    };
    let mut buffer = vec![0u16; 32768];
    let mut len = buffer.len() as u32;
    let path_matches = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut len,
        )
    }
    .is_ok()
        && String::from_utf16_lossy(&buffer[..len as usize])
            .eq_ignore_ascii_case(&dir.join("bpcrash.exe").to_string_lossy());
    if path_matches {
        let terminated = unsafe { TerminateProcess(handle, 0) };
        if let Err(err) = terminated {
            let _ = unsafe { CloseHandle(handle) };
            bail!("failed to stop Mintfixed-managed watcher (pid {pid}): {err}");
        }
        if unsafe { WaitForSingleObject(handle, 5000) } != WAIT_OBJECT_0 {
            let _ = unsafe { CloseHandle(handle) };
            bail!("timed out waiting for Mintfixed-managed watcher to stop (pid {pid})");
        }
        info!("BP-Crash-Reporter: stopped Mintfixed-managed watcher (pid {pid})");
    } else {
        info!("BP-Crash-Reporter: PID {pid} does not match installed reporter; left it running");
    }
    let _ = unsafe { CloseHandle(handle) };
    let _ = fs::remove_file(pid_file);
    Ok(())
}
