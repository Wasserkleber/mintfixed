// Generated build version information. Public path remains mint_lib::built_info.
include!(concat!(env!("OUT_DIR"), "/built.rs"));
include!(concat!(env!("OUT_DIR"), "/mintfixed_version.rs"));

pub fn version() -> &'static str {
    DISPLAY_VERSION
}

/// CLI shows the command name itself; avoid duplicating the prefix.
pub fn cli_version() -> &'static str {
    version().strip_prefix("mintfixed ").unwrap_or_else(version)
}
