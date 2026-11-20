//! The integration pre-pass: each mod pak is opened once and each of its packages read and parsed once, all in
//! parallel. The guard (`blueprint_guard`) and the registry rows both come from that one parse; the sequential
//! passes in `orchestration` only decide, in mod order, what wins.
use std::io::{Read, Seek};

use anyhow::anyhow;
use rayon::prelude::*;
use uasset_utils::asset_registry::{AssetData, MapHandle, get_root_export};
use uasset_utils::paths::{PakPathBuf, pak_path_to_game_path};
use unreal_asset::reader::ArchiveTrait as _;

use super::blueprint_guard::{self, BlueprintScan};
use super::pak_reader::PakSource;
use super::*;

pub(super) struct ModPak<'a> {
    pub(super) info: &'a ModInfo,
    pub(super) source: PakSource,
    pub(super) pak: repak::PakReader,
    pub(super) mount_point: String,
    /// Mounted path without the `../../../` prefix -> path inside the pak.
    pub(super) files: HashMap<PakPathBuf, String>,
    /// Every `.uasset`/`.umap` that has its `.uexp`, keyed like `files`; `Err` is the parser's message.
    pub(super) packages: HashMap<PakPathBuf, Result<Package, String>>,
}

pub(super) struct Package {
    pub(super) scan: BlueprintScan,
    /// The registry row's root export name and class, or the reason `populate` would fail.
    pub(super) row: Result<(String, String), String>,
}

impl<'a> ModPak<'a> {
    pub(super) fn open(info: &'a ModInfo, path: &Path) -> Result<Self, IntegrationError> {
        let file = fs::File::open(path).with_context(|_| CtxtIoSnafu {
            mod_info: info.clone(),
        })?;
        let source = PakSource::open(file).map_err(|e| match e {
            IntegrationError::IoError { source } => IntegrationError::CtxtIoError {
                source,
                mod_info: info.clone().into(),
            },
            e => e,
        })?;
        let pak = repak::PakBuilder::new()
            .reader(&mut source.reader())
            .with_context(|_| CtxtRepakSnafu {
                mod_info: info.clone(),
            })?;
        let mount = PakPath::new(pak.mount_point());
        let files = pak
            .files()
            .into_iter()
            .map(|p| -> Result<_, IntegrationError> {
                let j = mount.join(&p);
                Ok((
                    j.strip_prefix("../../../")
                        .map_err(|_| IntegrationError::ModfileInvalidPrefix {
                            mod_info: info.clone().into(),
                            modfile_path: j.to_string(),
                        })?
                        .to_path_buf(),
                    p,
                ))
            })
            .collect::<Result<HashMap<_, _>, _>>()?;

        let packages = files
            .par_iter()
            .filter(|(normalized, _)| {
                matches!(normalized.extension(), Some("uasset" | "umap"))
                    && files.contains_key(&normalized.with_extension("uexp"))
            })
            .map(|(normalized, pak_path)| -> Result<_, IntegrationError> {
                let mut reader = source.reader();
                let mut get = |path: &str| {
                    pak.get(path, &mut reader).with_context(|_| CtxtRepakSnafu {
                        mod_info: info.clone(),
                    })
                };
                let uasset = get(pak_path)?;
                let uexp = get(&files[&normalized.with_extension("uexp")])?;
                Ok((normalized.clone(), parse(&uasset, &uexp)))
            })
            .collect::<Result<HashMap<_, _>, _>>()?;

        Ok(Self {
            info,
            source,
            mount_point: pak.mount_point().to_string(),
            pak,
            files,
            packages,
        })
    }

    pub(super) fn read(&self, pak_path: &str) -> Result<Vec<u8>, IntegrationError> {
        self.pak
            .get(pak_path, &mut self.source.reader())
            .with_context(|_| CtxtRepakSnafu {
                mod_info: self.info.clone(),
            })
    }
}

/// Opens every mod in parallel; on failure, the error of the first failing mod in install order.
pub(super) fn open_all(mods: &[(ModInfo, PathBuf)]) -> Result<Vec<ModPak<'_>>, IntegrationError> {
    mods.par_iter()
        .map(|(info, path)| ModPak::open(info, path))
        .collect::<Vec<_>>()
        .into_iter()
        .collect()
}

fn parse(uasset: &[u8], uexp: &[u8]) -> Result<Package, String> {
    let asset = AssetBuilder::new(Cursor::new(uasset), EngineVersion::VER_UE4_27)
        .bulk(Cursor::new(uexp))
        .skip_data(true)
        .build()
        .map_err(|e| e.to_string())?;
    Ok(Package {
        scan: blueprint_guard::scan_asset(&asset),
        row: row_names(&asset).map_err(|e| e.to_string()),
    })
}

fn row_names<C: Read + Seek>(asset: &Asset<C>) -> anyhow::Result<(String, String)> {
    let root = get_root_export(asset).ok_or_else(|| anyhow!("no root export"))?;
    let root = asset.get_export(root).unwrap();
    let name = root.get_base_export().object_name.get_owned_content();
    let class = asset
        .get_import(root.get_base_export().class_index)
        .ok_or_else(|| anyhow!("bad import ref"))?
        .object_name
        .get_owned_content();
    Ok((name, class))
}

/// `AssetRegistry::populate` (uasset_utils 4977261) fed with the names the pre-pass read instead of the parsed
/// package, so the package need not be kept or parsed again. Row for row the same, its quirks included.
pub(super) fn populate_row(
    registry: &mut AssetRegistry,
    path: &str,
    asset_name_str: &str,
    asset_class_str: &str,
) -> anyhow::Result<()> {
    let game_path = PakPathBuf::from(
        pak_path_to_game_path(PakPath::new(path))
            .ok_or_else(|| anyhow!("failed to get game path"))?,
    );
    let package_path_str = game_path
        .parent()
        .ok_or_else(|| anyhow!("no path parent"))?
        .as_str();
    let package_name_str = game_path.as_str();
    let object_path_str = format!("{game_path}.{asset_name_str}");

    if let Some(name_index) = registry.names.0.get_index_of(&object_path_str)
        && registry
            .asset_data
            .iter()
            .any(|a| a.object_path.0 as usize == name_index)
    {
        return Ok(());
    }

    let tagless = || MapHandle {
        has_numberless_keys: true,
        num: 0,
        pair_begin: 0,
    };
    let object_path = registry.get_name(&object_path_str);
    let package_path = registry.get_name(package_path_str);
    let asset_class = registry.get_name(asset_class_str);
    let package_name = registry.get_name(package_name_str);
    let asset_name = registry.get_name(asset_name_str);
    registry.asset_data.push(AssetData {
        object_path,
        package_path,
        asset_class,
        package_name,
        asset_name,
        tags: tagless(),
        bundle_count: 0,
        chunk_ids: vec![],
        flags: 0,
    });

    if let (Some(asset_name_str), Some(object_path_str), Some(asset_class_str)) = (
        asset_name_str.strip_suffix("_C"),
        object_path_str.strip_suffix("_C"),
        asset_class_str.strip_suffix("GeneratedClass"),
    ) {
        let object_path = registry.get_name(object_path_str);
        let asset_class = registry.get_name(asset_class_str);
        let asset_name = registry.get_name(asset_name_str);
        registry.asset_data.push(AssetData {
            object_path,
            package_path,
            asset_class,
            package_name,
            asset_name,
            tags: tagless(),
            bundle_count: 0,
            chunk_ids: vec![],
            flags: 0,
        });
    }
    Ok(())
}
