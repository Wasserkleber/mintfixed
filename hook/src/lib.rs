mod crash_reporter;
mod hooks;
mod runtime_context;
mod ue;

use std::{io::BufReader, path::Path};

use anyhow::{Context, Result};
use fs_err as fs;
use mint_lib::mod_info::Meta;
use runtime_context::GLOBALS;
pub use runtime_context::{Globals, globals};
use tracing::{info, warn};

proxy_dll::proxy_dll!([x3daudio1_7, d3d9], init);

fn init() {
    startup_checkpoint(&format!(
        "DLL entry: hook crate version {}",
        env!("CARGO_PKG_VERSION")
    ));
    // Keep the previous best-effort startup behavior, but report normal errors.
    match unsafe { patch() } {
        Ok(()) => startup_checkpoint("DLL initialization completed successfully"),
        Err(err) => startup_checkpoint(&format!("ERROR: hook initialization failed: {err:#}")),
    }
}

thread_local! {
    static LOG_GUARD: std::cell::RefCell<Option<tracing_appender::non_blocking::WorkerGuard>>  = None.into();
}

/// Write crash-resilient startup checkpoints to mint_hook.log.
///
/// The ordinary tracing appender is asynchronous. During early hook startup,
/// a native access violation can terminate the process before that worker
/// flushes. Open, append, flush and sync each diagnostic checkpoint separately.
/// This logger is deliberately best-effort and must not affect game startup.
pub(crate) fn startup_checkpoint(message: &str) {
    use std::io::Write;

    let Some(bin_dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    else {
        eprintln!("[mintfixed-hook] {message}");
        return;
    };

    let timestamp_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let line = format!(
        "[hook-startup {timestamp_ms} pid={}] {message}\n",
        std::process::id()
    );

    let result = (|| -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(bin_dir.join("mint_hook.log"))?;
        file.write_all(line.as_bytes())?;
        file.flush()?;
        file.sync_data()
    })();
    if let Err(err) = result {
        eprintln!("[mintfixed-hook] could not write startup checkpoint: {err}: {message}");
    }
}

unsafe fn patch() -> Result<()> {
    startup_checkpoint("patch(): locating executable");
    let exe_path = std::env::current_exe().ok();
    let bin_dir = exe_path.as_deref().and_then(Path::parent);
    startup_checkpoint(&format!("Executable path: {exe_path:?}"));
    startup_checkpoint("Initializing tracing appender");

    let guard = bin_dir
        .and_then(|bin_dir| mint_lib::setup_logging(bin_dir.join("mint_hook.log"), "hook").ok());
    if guard.is_none() {
        warn!("failed to set up logging");
    }

    startup_checkpoint("Tracing appender setup returned; retaining guard before PAK operations");
    LOG_GUARD.with_borrow_mut(|g| *g = guard);
    startup_checkpoint("Determining mods_P.pak path");
    let pak_path = bin_dir
        .and_then(Path::parent)
        .and_then(Path::parent)
        .map(|p| p.join("Content/Paks/mods_P.pak"))
        .context("could not determine pak path")?;

    startup_checkpoint(&format!("PAK path: {}", pak_path.display()));
    startup_checkpoint("Opening mods_P.pak");
    let mut pak_reader = BufReader::new(fs::File::open(pak_path)?);
    startup_checkpoint("mods_P.pak opened");
    startup_checkpoint("Reading PAK index");
    let pak = repak::PakBuilder::new().reader(&mut pak_reader)?;
    startup_checkpoint("PAK index parsed");

    startup_checkpoint("Reading meta entry from PAK");
    let meta_buf = pak.get("meta", &mut pak_reader)?;
    startup_checkpoint("Meta entry read");
    startup_checkpoint("Decoding mod metadata");
    let meta: Meta = postcard::from_bytes(&meta_buf)?;
    startup_checkpoint("Mod metadata decoded");

    startup_checkpoint("Reading executable image");
    let image = patternsleuth::process::internal::read_image()?;
    startup_checkpoint("Executable image read");
    startup_checkpoint("Resolving hook patterns");
    let resolution = image.resolve(hook_resolvers::HookResolution::resolver())?;
    startup_checkpoint(&format!("Hook patterns resolved: {resolution:#x?}"));
    info!("PS scan: {:#x?}", resolution);

    // Publish once before enabling any native detours that call globals().
    startup_checkpoint("Publishing hook runtime context");
    GLOBALS
        .set(Globals { resolution, meta })
        .map_err(|_| anyhow::anyhow!("hook runtime context initialized twice"))?;

    startup_checkpoint("Runtime context published; initializing native detours");
    unsafe {
        hooks::initialize()?;
    }
    startup_checkpoint("Native detours initialized successfully");

    info!("hook initialized");

    Ok(())
}
