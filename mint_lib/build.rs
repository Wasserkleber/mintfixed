mod build_version;

fn main() {
    built::write_built_file().expect("Failed to acquire build-time information");
    // Keep built.rs for existing non-display consumers.
    let manifest_dir = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("missing CARGO_MANIFEST_DIR"));
    build_version::emit_rerun_directives(&manifest_dir);
    let release_tag = std::env::var("MINTFIXED_RELEASE_TAG").ok();
    let info = build_version::resolve(&manifest_dir, env!("CARGO_PKG_VERSION"), release_tag.as_deref())
        .unwrap_or_else(|err| panic!("mintfixed version validation failed: {err}"));
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("missing OUT_DIR"));
    build_version::write_generated(&out, &info).expect("failed to write mintfixed version metadata");
}
