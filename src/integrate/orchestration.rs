// Mechanical source extraction from src/integrate.rs; no algorithm changes.
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

    let mut output = crate::installation::output::InstallOutput::new(&path_mod_pak)?;
    let mut writer = BufWriter::new(output.file());
    let mut bundle = ModBundleWriter::new(&mut writer, &fsd_pak.files())?;

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

    let mut init_spacerig_assets = HashSet::new();
    let mut init_cave_assets = HashSet::new();

    // Blueprint dependency preflight. Resolve first-wins assets before emitting
    // anything into mods_P.pak, then inspect actual exported SuperIndex edges.
    // Do not use the old "all vanilla exports must survive" ABI heuristic:
    // intentionally simplified UI/level Blueprints legitimately remove functions.
    let vanilla_files: HashMap<String, String> = fsd_pak
        .files()
        .into_iter()
        .map(|path| (path.to_ascii_lowercase(), path))
        .collect();
    let mut selected_blueprints: HashMap<String, (String, blueprint_guard::BlueprintScan)> =
        HashMap::new();
    let mut seen_packages = HashSet::new();
    for (mod_info, path) in &mods {
        let raw_mod_file = fs::File::open(path).with_context(|_| CtxtIoSnafu {
            mod_info: mod_info.clone(),
        })?;
        let mut reader = get_pak_from_data(Box::new(BufReader::new(raw_mod_file)))?;
        let pak = repak::PakBuilder::new()
            .reader(&mut reader)
            .with_context(|_| CtxtRepakSnafu {
                mod_info: mod_info.clone(),
            })?;
        let mount = PakPath::new(pak.mount_point());
        let pak_files = pak
            .files()
            .into_iter()
            .map(|file| -> Result<_, IntegrationError> {
                let mounted = mount.join(&file);
                Ok((
                    mounted
                        .strip_prefix("../../../")
                        .map_err(|_| IntegrationError::ModfileInvalidPrefix {
                            mod_info: mod_info.clone().into(),
                            modfile_path: mounted.to_string(),
                        })?
                        .to_path_buf(),
                    file,
                ))
            })
            .collect::<Result<HashMap<_, _>, _>>()?;
        for (normalized, pak_path) in &pak_files {
            if !matches!(normalized.extension(), Some("uasset" | "umap")) {
                continue;
            }
            let Some(uexp_path) = pak_files.get(&normalized.with_extension("uexp")) else {
                continue;
            };
            let stem = normalized.with_extension("").as_str().to_ascii_lowercase();
            if !seen_packages.insert(stem.clone()) {
                continue; // The writer also gives the earlier mod precedence.
            }
            let uasset = pak
                .get(pak_path, &mut reader)
                .with_context(|_| CtxtRepakSnafu {
                    mod_info: mod_info.clone(),
                })?;
            let uexp = pak
                .get(uexp_path, &mut reader)
                .with_context(|_| CtxtRepakSnafu {
                    mod_info: mod_info.clone(),
                })?;
            match blueprint_guard::scan(&uasset, &uexp) {
                Ok(scan) => {
                    selected_blueprints.insert(stem, (mod_info.name.clone(), scan));
                }
                Err(error) => {
                    tracing::debug!(
                        "Blueprint dependency guard: cannot inspect {} from '{}': {}; no verdict",
                        normalized,
                        mod_info.name,
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
    let mut added_paths = HashSet::new();

    for (mod_info, path) in &mods {
        let raw_mod_file = fs::File::open(path).with_context(|_| CtxtIoSnafu {
            mod_info: mod_info.clone(),
        })?;
        let mut buf = get_pak_from_data(Box::new(BufReader::new(raw_mod_file))).map_err(|e| {
            if let IntegrationError::IoError { source } = e {
                IntegrationError::CtxtIoError {
                    source,
                    mod_info: mod_info.clone().into(),
                }
            } else {
                e
            }
        })?;
        let pak = repak::PakBuilder::new()
            .reader(&mut buf)
            .with_context(|_| CtxtRepakSnafu {
                mod_info: mod_info.clone(),
            })?;

        let mount = PakPath::new(pak.mount_point());

        let pak_files = pak
            .files()
            .into_iter()
            .map(|p| -> Result<_, IntegrationError> {
                let j = mount.join(&p);
                Ok((
                    j.strip_prefix("../../../")
                        .map_err(|_| IntegrationError::ModfileInvalidPrefix {
                            mod_info: mod_info.clone().into(),
                            modfile_path: j.to_string(),
                        })?
                        .to_path_buf(),
                    p,
                ))
            })
            .collect::<Result<HashMap<_, _>, _>>()?;

        for (normalized, pak_path) in &pak_files {
            if blueprint_guard::rejected_sidecar(normalized.as_str(), &rejected_blueprints) {
                continue;
            }
            if added_paths.contains(&normalized.as_str().to_ascii_lowercase()) {
                continue;
            }
            match normalized.extension() {
                Some("uasset" | "umap")
                    if pak_files.contains_key(&normalized.with_extension("uexp")) =>
                {
                    let uasset = pak
                        .get(pak_path, &mut buf)
                        .with_context(|_| CtxtRepakSnafu {
                            mod_info: mod_info.clone(),
                        })?;

                    let uexp = pak
                        .get(
                            PakPath::new(pak_path).with_extension("uexp").as_str(),
                            &mut buf,
                        )
                        .with_context(|_| CtxtRepakSnafu {
                            mod_info: mod_info.clone(),
                        })?;

                    let asset = AssetBuilder::new(Cursor::new(uasset), EngineVersion::VER_UE4_27)
                        .bulk(Cursor::new(uexp))
                        .skip_data(true)
                        .build();

                    let asset = match asset {
                        Ok(asset) => asset,
                        Err(err) => {
                            warn!("failed to parse asset {normalized}: {err}");
                            continue;
                        }
                    };

                    let asset_path = normalized.with_extension("");
                    if !supported_registry_asset_path(asset_path.as_str()) {
                        return Err(IntegrationError::ModfileInvalidAssetPath {
                            mod_info: mod_info.clone().into(),
                            asset_path: normalized.to_string(),
                            mount_point: pak.mount_point().to_string(),
                        });
                    }

                    asset_registry
                        .populate(asset_path.as_str(), &asset)
                        .map_err(|e| IntegrationError::CtxtGenericError {
                            source: e.into(),
                            mod_info: mod_info.clone().into(),
                        })?;
                }
                _ => {}
            }
        }

        for (normalized, pak_path) in pak_files {
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
                    init_spacerig_assets.insert(format_soft_class(&normalized));
                }
                if lower == "initcave.uasset" {
                    init_cave_assets.insert(format_soft_class(&normalized));
                }
            }

            let file_data = pak
                .get(&pak_path, &mut buf)
                .with_context(|_| CtxtRepakSnafu {
                    mod_info: mod_info.clone(),
                })?;
            if let Some(raw) = normalized
                .as_str()
                .strip_suffix(".uasset")
                .and_then(|path| deferred_assets.get_mut(path))
            {
                raw.uasset = Some(file_data);
            } else if let Some(raw) = normalized
                .as_str()
                .strip_suffix(".uexp")
                .and_then(|path| deferred_assets.get_mut(path))
            {
                raw.uexp = Some(file_data);
            } else {
                bundle.write_file(&file_data, normalized.as_str())?;
                added_paths.insert(lowercase);
            }
        }
    }

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

    bundle.write_meta(config, &mods)?;

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
    Ok(report)
}
