// Exercise the actual std-only build-script implementation with temporary Git repos.
// Git/Cargo rerun helpers are used by build.rs, but not by this test target.
#[allow(dead_code)]
#[path = "../build_version.rs"]
mod build_version;
