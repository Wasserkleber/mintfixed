use rayon::prelude::*;

use super::*;

#[tracing::instrument(skip_all)]
pub fn integrate<P: AsRef<Path>>(
    path_pak: P,
    config: MetaConfig,
    mods: Vec<(ModInfo, PathBuf)>,
) -> Result<(), IntegrationError> {
    integrate_with_report(path_pak, config, mods, Vec::new(), Vec::new()).map(|_| ())
}

#[tracing::instrument(skip_all)]
pub(crate) fn integrate_with_report<P: AsRef<Path>>(
    path_pak: P,
    config: MetaConfig,
    mods: Vec<(ModInfo, PathBuf)>,
    locations: Vec<String>,
    skipped: Vec<crate::installation::report::SkippedMod>,
) -> Result<crate::installation::report::InstallReport, IntegrationError> {
    let mintfixed_crash_vanilla_pak = path_pak.as_ref().to_path_buf();
    let mut report = crate::installation::report::InstallReport {
        installed: 0,
        skipped,
    };
    let mods = crate::installation::validation::prepare_packer_inputs_with_report(
        mods,
        &locations,
        &mut report.skipped,
    )?;
    report.installed = mods.len();
    // Read now: `config` is moved into the pak's meta further down.
    #[cfg(feature = "hook")]
    let bundled_crash_reporter = config.bundled_crash_reporter;
    let (bitfix_patches, mods): (Vec<_>, Vec<_>) = mods
        .into_iter()
        .partition(|(info, _)| crate::bitfix::is_patch(info));
    let Ok(installation) = DRGInstallation::from_pak_path(&path_pak) else {
        return Err(IntegrationError::DrgInstallationNotFound {
            path: path_pak.as_ref().to_path_buf(),
        });
    };
    let path_mod_pak = installation.paks_path().join("mods_P.pak");

    let mut fsd_pak_reader = BufReader::new(fs::File::open(path_pak.as_ref())?);
    let fsd_pak = repak::PakBuilder::new().reader(&mut fsd_pak_reader)?;

    #[derive(Debug, Default)]
    struct RawAsset {
        uasset: Option<Vec<u8>>,
        uexp: Option<Vec<u8>>,
    }

    impl RawAsset {
        fn parse(&self) -> Result<Asset<Cursor<&Vec<u8>>>, IntegrationError> {
            Ok(AssetBuilder::new(
                Cursor::new(self.uasset.as_ref().unwrap()),
                EngineVersion::VER_UE4_27,
            )
            .bulk(Cursor::new(self.uexp.as_ref().unwrap()))
            .build()?)
        }
    }

    let ar_path = "FSD/AssetRegistry.bin";
    let mut asset_registry =
        AssetRegistry::read(&mut Cursor::new(fsd_pak.get(ar_path, &mut fsd_pak_reader)?))
            .map_err(|e| IntegrationError::GenericError { msg: e.to_string() })?;

    let mut other_deferred = vec![];
    let mut deferred = |path| {
        other_deferred.push(path);
        path
    };

    let pcb_path = "FSD/Content/Game/BP_PlayerControllerBase";
    let patch_paths = [
        "FSD/Content/Game/BP_GameInstance",
        "FSD/Content/Game/SpaceRig/BP_PlayerController_SpaceRig",
        "FSD/Content/Game/StartMenu/Bp_StartMenu_PlayerController",
        "FSD/Content/UI/Menu_DeepDives/ITM_DeepDives_Join",
        "FSD/Content/UI/Menu_ServerList/_MENU_ServerList",
        "FSD/Content/UI/Menu_ServerList/WND_JoiningModded",
    ];
    let escape_menu_path = deferred("FSD/Content/UI/Menu_EscapeMenu/MENU_EscapeMenu");
    let modding_tab_path = deferred("FSD/Content/UI/Menu_EscapeMenu/Modding/MENU_Modding");
    let server_list_entry_path = deferred("FSD/Content/UI/Menu_ServerList/ITM_ServerList_Entry");

    let mut deferred_assets: HashMap<&str, RawAsset> = HashMap::from_iter(
        [pcb_path]
            .iter()
            .chain(patch_paths.iter())
            .chain(other_deferred.iter())
            .map(|path| (*path, RawAsset::default())),
    );

    // collect assets from game pak file
    for (path, asset) in &mut deferred_assets {
        // TODO repak should return an option...
        asset.uasset = match fsd_pak.get(&format!("{path}.uasset"), &mut fsd_pak_reader) {
            Ok(file) => Ok(Some(file)),
            Err(repak::Error::MissingEntry(_)) => Ok(None),
            Err(e) => Err(e),
        }?;
        asset.uexp = match fsd_pak.get(&format!("{path}.uexp"), &mut fsd_pak_reader) {
            Ok(file) => Ok(Some(file)),
            Err(repak::Error::MissingEntry(_)) => Ok(None),
            Err(e) => Err(e),
        }?;
    }

    // Resolve bitfix installation before building the replacement mod archive.
    let bitfix_patches: Vec<_> = bitfix_patches
        .iter()
        .map(|(info, path)| (info.name.as_str(), path.as_path()))
        .collect();
    crate::bitfix::sync(&installation, &bitfix_patches)?;

    #[cfg(feature = "hook")]
    {
        let path_hook_dll = installation
            .binaries_directory()
            .join(installation.installation_type.hook_dll_name());
        let hook_dll = include_bytes!(env!("CARGO_CDYLIB_FILE_HOOK_hook"));
        // Replace the hook whenever its contents change, even at the same size.
        let needs_update = match fs::read(&path_hook_dll) {
            Ok(installed) => installed.as_slice() != hook_dll.as_slice(),
            Err(err) if err.kind() == ErrorKind::NotFound => true,
            Err(err) => return Err(err.into()),
        };
        if needs_update {
            info!(
                "Mintfixed: updating hook DLL (contents changed): {}",
                path_hook_dll.display()
            );
            fs::write(&path_hook_dll, hook_dll)?;
        }
    }

    // Keep the installed mods_P.pak when nothing that goes into it changed. Every mod file
    // is still hashed; a failure to hash only means the pak is rebuilt.
    let meta = bundle_writer::meta_bytes(config, &mods);
    let key = match unchanged::key(&meta, path_pak.as_ref(), &mods) {
        Ok(key) => Some(key),
        Err(err) => {
            warn!("cannot fingerprint the mod set ({err}); rebuilding mods_P.pak");
            None
        }
    };
    if key.is_some_and(|key| unchanged::installed_key(&path_mod_pak).as_deref() == Some(&key[..])) {
        info!("mods_P.pak is up to date (same MINT, game pak, settings and mod files): kept");
        return finish_install(
            &installation,
            #[cfg(feature = "hook")]
            bundled_crash_reporter,
            &mods,
            &path_mod_pak,
            report,
        );
    }

    let mut output = crate::installation::output::InstallOutput::new(&path_mod_pak)?;
    let mut writer = BufWriter::new(output.file());
    let mut bundle = ModBundleWriter::new(&mut writer, &fsd_pak.files())?;

    let mut init_spacerig_assets = HashSet::new();
    let mut init_cave_assets = HashSet::new();

    // Open every mod and parse each of its packages once, in parallel. The guard and the
    // registry rows below both read that one parse.
    let paks = mod_inputs::open_all(&mods)?;

    // Blueprint dependency preflight. Resolve first-wins assets before emitting
    // anything into mods_P.pak, then inspect actual exported SuperIndex edges.
    // Do not use the old "all vanilla exports must survive" ABI heuristic:
    // intentionally simplified UI/level Blueprints legitimately remove functions.
    let vanilla_files: HashMap<String, String> = fsd_pak
        .files()
        .into_iter()
        .map(|path| (path.to_ascii_lowercase(), path))
        .collect();
    let mut selected_blueprints: HashMap<String, (&str, &blueprint_guard::BlueprintScan)> =
        HashMap::new();
    let mut seen_packages = HashSet::new();
    for m in &paks {
        for (normalized, package) in &m.packages {
            let stem = normalized.with_extension("").as_str().to_ascii_lowercase();
            if !seen_packages.insert(stem.clone()) {
                continue; // The writer also gives the earlier mod precedence.
            }
            match package {
                Ok(package) => {
                    selected_blueprints.insert(stem, (m.info.name.as_str(), &package.scan));
                }
                Err(error) => {
                    tracing::debug!(
                        "Blueprint dependency guard: cannot inspect {} from '{}': {}; no verdict",
                        normalized,
                        m.info.name,
                        error
                    );
                }
            }
        }
    }

    // Dependencies are keyed by *target* package, while source identifies the
    // selected mod package with the cooked reference. This includes packages
    // appearing later in mod priority order (such as Elytras vs OBNC).
    let mut required_by: HashMap<String, Vec<(String, blueprint_guard::Dependency)>> =
        HashMap::new();
    for (source, (_, scan)) in &selected_blueprints {
        for dependency in &scan.dependencies {
            required_by
                .entry(dependency.target.clone())
                .or_default()
                .push((source.clone(), dependency.clone()));
        }
    }
    let mut rejected_blueprints = HashSet::new();
    let mut candidates = selected_blueprints.keys().cloned().collect::<Vec<_>>();
    candidates.sort();
    for stem in candidates {
        let Some(references) = required_by.get(&stem) else {
            continue; // No *installed* package has a hard reference to this one.
        };
        let Some(vanilla_asset_path) = vanilla_files.get(&format!("{stem}.uasset")) else {
            continue; // No vanilla Blueprint is replaced.
        };
        let Some(vanilla_exp_path) = vanilla_files.get(&format!("{stem}.uexp")) else {
            continue;
        };
        let Some((mod_name, replacement)) = selected_blueprints.get(&stem) else {
            continue;
        };
        let needed = references
            .iter()
            .filter(|(source, _)| !rejected_blueprints.contains(source))
            .map(|(_, dep)| dep)
            .collect::<Vec<_>>();
        if needed.is_empty() {
            continue;
        }
        let original_uasset = fsd_pak.get(vanilla_asset_path, &mut fsd_pak_reader)?;
        let original_uexp = fsd_pak.get(vanilla_exp_path, &mut fsd_pak_reader)?;
        let original = match blueprint_guard::scan(&original_uasset, &original_uexp) {
            Ok(original) => original,
            Err(error) => {
                warn!(
                    "Blueprint dependency guard: cannot inspect vanilla {}: {}; no verdict",
                    stem, error
                );
                continue;
            }
        };
        let missing = blueprint_guard::broken_references(&original, replacement, needed);
        if !missing.is_empty() {
            let referers = references
                .iter()
                .filter(|(source, _)| !rejected_blueprints.contains(source))
                .map(|(source, _)| source.as_str())
                .collect::<HashSet<_>>();
            warn!(
                "Blueprint dependency guard: excluded {} from mod '{}': unresolved imported SuperIndex symbol(s) [{}] required by {:?}; vanilla package retained",
                stem,
                mod_name,
                missing.join(", "),
                referers
            );
            rejected_blueprints.insert(stem);
        }
    }
    info!(
        "Blueprint dependency guard: scanned {} winning mod package(s); rejected {} conflicting vanilla replacement(s)",
        selected_blueprints.len(),
        rejected_blueprints.len()
    );

    // Decide, in mod order, which file of which mod wins (first mod wins; the patched
    // vanilla packages keep their old last-wins handling) and add the registry rows.
    // Reading the winners happens after, in parallel.
    enum Target<'p> {
        Write,
        DeferredUasset(&'p str),
        DeferredUexp(&'p str),
    }
    struct Op<'m, 'p> {
        mod_index: usize,
        pak_path: &'m str,
        normalized: &'m str,
        target: Target<'p>,
    }
    let deferred_keys: Vec<&str> = deferred_assets.keys().copied().collect();
    let deferred_key =
        |path: Option<&str>| path.and_then(|p| deferred_keys.iter().copied().find(|k| *k == p));
    let mut added_paths = HashSet::new();
    let mut ops: Vec<Op> = vec![];

    for (mod_index, m) in paks.iter().enumerate() {
        for normalized in m.files.keys() {
            if blueprint_guard::rejected_sidecar(normalized.as_str(), &rejected_blueprints) {
                continue;
            }
            if added_paths.contains(&normalized.as_str().to_ascii_lowercase()) {
                continue;
            }
            let Some(package) = m.packages.get(normalized) else {
                continue; // Not a .uasset/.umap with its .uexp.
            };
            let (asset_name, asset_class) = match package {
                Ok(package) => match &package.row {
                    Ok(row) => row,
                    Err(e) => {
                        return Err(IntegrationError::CtxtGenericError {
                            source: e.clone().into(),
                            mod_info: m.info.clone().into(),
                        });
                    }
                },
                Err(err) => {
                    warn!("failed to parse asset {normalized}: {err}");
                    continue;
                }
            };
            let asset_path = normalized.with_extension("");
            if !supported_registry_asset_path(asset_path.as_str()) {
                return Err(IntegrationError::ModfileInvalidAssetPath {
                    mod_info: m.info.clone().into(),
                    asset_path: normalized.to_string(),
                    mount_point: m.mount_point.clone(),
                });
            }
            mod_inputs::populate_row(
                &mut asset_registry,
                asset_path.as_str(),
                asset_name,
                asset_class,
            )
            .map_err(|e| IntegrationError::CtxtGenericError {
                source: e.into(),
                mod_info: m.info.clone().into(),
            })?;
        }

        for (normalized, pak_path) in &m.files {
            if blueprint_guard::rejected_sidecar(normalized.as_str(), &rejected_blueprints) {
                continue;
            }
            let lowercase = normalized.as_str().to_ascii_lowercase();
            if added_paths.contains(&lowercase) {
                continue;
            }

            if let Some(filename) = normalized.file_name() {
                if filename == "AssetRegistry.bin" {
                    continue;
                }
                if normalized.extension() == Some("ushaderbytecode") {
                    continue;
                }
                let lower = filename.to_lowercase();
                if lower == "initspacerig.uasset" {
                    init_spacerig_assets.insert(format_soft_class(normalized));
                }
                if lower == "initcave.uasset" {
                    init_cave_assets.insert(format_soft_class(normalized));
                }
            }

            let path = normalized.as_str();
            let target = if let Some(key) = deferred_key(path.strip_suffix(".uasset")) {
                Target::DeferredUasset(key)
            } else if let Some(key) = deferred_key(path.strip_suffix(".uexp")) {
                Target::DeferredUexp(key)
            } else {
                added_paths.insert(lowercase);
                Target::Write
            };
            ops.push(Op {
                mod_index,
                pak_path,
                normalized: path,
                target,
            });
        }
    }

    // Read (decompress) and hash the winners in parallel a chunk at a time, one chunk ahead
    // of this thread, which appends them in order.
    // ponytail: 3 chunks of 128 in memory at once; 384 huge textures would all sit there together.
    let entry_builder = bundle.entry_builder();
    std::thread::scope(|scope| -> Result<(), IntegrationError> {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let (ops, paks, entry_builder) = (&ops, &paks, &entry_builder);
        scope.spawn(move || {
            for chunk in ops.chunks(128) {
                let built = chunk
                    .par_iter()
                    .map(|op| -> Result<_, IntegrationError> {
                        let data = paks[op.mod_index].read(op.pak_path)?;
                        Ok(match op.target {
                            Target::Write => (Some(entry_builder.build_entry(false, data)?), None),
                            _ => (None, Some(data)),
                        })
                    })
                    .collect::<Vec<_>>();
                if tx.send((chunk, built)).is_err() {
                    break; // The writer stopped on an error.
                }
            }
        });
        for (chunk, built) in rx {
            for (op, built) in chunk.iter().zip(built) {
                match (&op.target, built?) {
                    (Target::Write, (Some(entry), _)) => {
                        bundle.write_entry(entry, op.normalized)?
                    }
                    (Target::DeferredUasset(key), (_, data)) => {
                        deferred_assets.get_mut(key).unwrap().uasset = data
                    }
                    (Target::DeferredUexp(key), (_, data)) => {
                        deferred_assets.get_mut(key).unwrap().uexp = data
                    }
                    _ => unreachable!(),
                }
            }
        }
        Ok(())
    })?;
    drop(paks);

    {
        let mut pcb_asset = deferred_assets[&pcb_path].parse()?;
        hook_pcb(&mut pcb_asset);
        bundle.write_asset(pcb_asset, pcb_path)?;
    }

    let mut patch_deferred = |path_str: &str,
                              f: fn(&mut _) -> Result<(), IntegrationError>|
     -> Result<(), IntegrationError> {
        let mut asset = deferred_assets[path_str].parse()?;
        f(&mut asset)?;
        bundle.write_asset(asset, path_str)
    };

    // apply patches to base assets
    for patch_path in patch_paths {
        patch_deferred(patch_path, patch)?;
    }
    patch_deferred(escape_menu_path, patch_modding_tab)?;
    patch_deferred(modding_tab_path, patch_modding_tab_item)?;
    patch_deferred(server_list_entry_path, patch_server_list_entry)?;

    let mut int_files = HashMap::new();
    collect_dir_files(&INTEGRATION_DIR, &mut int_files);

    for (path, data) in &int_files {
        bundle.write_file(data, path)?;
    }

    bundle.write_file(&meta, "meta")?;
    if let Some(key) = key {
        bundle.write_file(&key, unchanged::KEY_PATH)?;
    }

    let mut buf = vec![];
    asset_registry
        .write(&mut buf)
        .map_err(|e| IntegrationError::GenericError { msg: e.to_string() })?;
    bundle.write_file(&buf, ar_path)?;

    bundle.finish()?;
    // BufWriter's destructor ignores flush failures, so flush explicitly before
    // publishing the completed archive. Any earlier error drops the temp file.
    writer.flush()?;
    drop(writer);
    output.commit()?;
    if !rejected_blueprints.is_empty() {
        warn!(
            "Blueprint dependency guard: excluded {} conflicting vanilla package(s) from mods_P.pak",
            rejected_blueprints.len()
        );
    }

    finish_install(
        &installation,
        #[cfg(feature = "hook")]
        bundled_crash_reporter,
        &mods,
        &path_mod_pak,
        report,
    )
}

/// The steps after mods_P.pak is in place, whether it was just written or kept unchanged.
fn finish_install(
    installation: &DRGInstallation,
    #[cfg(feature = "hook")] bundled_crash_reporter: bool,
    mods: &[(ModInfo, PathBuf)],
    path_mod_pak: &Path,
    report: crate::installation::report::InstallReport,
) -> Result<crate::installation::report::InstallReport, IntegrationError> {
    #[cfg(feature = "hook")]
    if let Err(err) = crate::crash_reporter::install(
        &installation.binaries_directory(),
        &installation.installation_type,
        bundled_crash_reporter,
    ) {
        // Optional diagnostics must not prevent installing mods.
        warn!(
            "BP-Crash-Reporter installation failed (mods installed, enhanced logging unavailable): {err:#}"
        );
    }

    // Log only after the complete output pak has been written successfully.
    // This counts the final, deduplicated list, not just downloaded files.
    for (index, (mod_info, _)) in mods.iter().enumerate() {
        info!(
            "installed mod {}/{}: {} ({})",
            index + 1,
            mods.len(),
            mod_info.name,
            mod_info.resolution.get_resolvable_url_or_name()
        );
    }

    info!("Installation output: {}", path_mod_pak.display());
    info!("{}", report.details());
    if let Err(error) = crate::crash_context::write_installed_context(
        &mintfixed_crash_vanilla_pak,
        &path_mod_pak,
        &mods,
    ) {
        tracing::warn!("Mintfixed crash context export skipped: {error}");
    }
    Ok(report)
}
