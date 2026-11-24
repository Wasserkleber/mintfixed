//! The integration pre-pass: each mod pak is opened once and each of its packages read and parsed once, all in
//! parallel. The guard (`blueprint_guard`) and the registry rows both come from that one parse; the sequential
//! passes in `orchestration` only decide, in mod order, what wins.
use std::io::{Read, Seek};

use anyhow::anyhow;
use rayon::prelude::*;
use uasset_utils::asset_registry::{
    AssetData, ExportPath, MapHandle, NameIndex, NameIndexFlagged, Pair, Readable, Type,
    get_root_export,
};
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
                let uasset = pak
                    .get(pak_path, &mut std::io::BufReader::new(source.reader()))
                    .with_context(|_| CtxtRepakSnafu {
                        mod_info: info.clone(),
                    })?;
                Ok((normalized.clone(), parse(&uasset)))
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

    /// Buffered: repak reads an entry's header field by field, and unbuffered each field of a file-backed pak would
    /// be its own read call.
    pub(super) fn read(&self, pak_path: &str) -> Result<Vec<u8>, IntegrationError> {
        self.pak
            .get(pak_path, &mut std::io::BufReader::new(self.source.reader()))
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

/// Parses the package summary, name, import and export maps, which all live in the `.uasset`. With `skip_data`
/// nothing reads the export data in the `.uexp`, so an empty one stands in for it and the (often large) `.uexp`
/// is never read or decompressed here.
fn parse(uasset: &[u8]) -> Result<Package, String> {
    let asset = AssetBuilder::new(Cursor::new(uasset), EngineVersion::VER_UE4_27)
        .bulk(Cursor::new(&[][..]))
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

    if has_row(registry, &object_path_str, None) {
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

/// Whether `registry` already has a row for `object_path` (its name and number), the check `populate` makes.
fn has_row(registry: &AssetRegistry, object_path: &str, number: Option<u32>) -> bool {
    registry.names.0.get_index_of(object_path).is_some_and(|i| {
        let wanted = NameIndexFlagged(i as u32, number);
        registry.asset_data.iter().any(|a| a.object_path == wanted)
    })
}

/// A mod's own `AssetRegistry.bin`. Without it a mod's packages get only `populate_row`'s tagless rows, and the tags
/// it shipped never reach the merged registry. For the packages the mod wins, `copy_rows` takes its rows, tags
/// included, in place of those.
pub(super) struct ModRegistry {
    registry: AssetRegistry,
    /// Lowercased package name -> its rows.
    by_package: HashMap<String, Vec<usize>>,
}

impl ModRegistry {
    /// Reads the mod's `FSD/AssetRegistry.bin`, if it ships one MINT can merge; a registry it cannot is logged and
    /// skipped, and the mod's packages get `populate_row`'s rows as before.
    pub(super) fn load(m: &ModPak, game: &AssetRegistry) -> Option<Self> {
        let (_, pak_path) = m
            .files
            .iter()
            .find(|(n, _)| n.as_str().eq_ignore_ascii_case("FSD/AssetRegistry.bin"))?;
        let parsed = m
            .read(pak_path)
            .map_err(|e| e.to_string())
            .and_then(|data| Self::parse(&data, game));
        parsed
            .inspect_err(|e| {
                warn!(
                    "'{}': its AssetRegistry.bin is not merged, its rows lose their tags: {e}",
                    m.info.name
                )
            })
            .ok()
    }

    pub(super) fn parse(data: &[u8], game: &AssetRegistry) -> Result<Self, String> {
        // uasset_utils asserts (panics) where it meets what it does not model, such as numbered tag pairs.
        let registry = std::panic::catch_unwind(|| AssetRegistry::read(&mut Cursor::new(data)))
            .map_err(|_| "uasset_utils cannot read it".to_string())?
            .map_err(|e| e.to_string())?;
        if (registry.version, registry.version_int) != (game.version, game.version_int) {
            return Err(format!(
                "version {} is not the game's {}",
                registry.version_int, game.version_int
            ));
        }
        // uasset_utils reads neither numbered tag pairs nor asset bundles, so such a file was misread.
        if registry.store.pair_count != 0 || registry.asset_data.iter().any(|a| a.bundle_count != 0)
        {
            return Err("it has numbered tag pairs or asset bundles".into());
        }
        let mut by_package: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, row) in registry.asset_data.iter().enumerate() {
            let package = registry.names[row.package_name].to_ascii_lowercase();
            by_package.entry(package).or_default().push(i);
        }
        Ok(Self {
            registry,
            by_package,
        })
    }

    /// Copies the rows for the package at pak path `path` (no extension) into `to`, each with its tags, skipping one
    /// whose ObjectPath `to` already has (a replaced vanilla asset keeps vanilla's row, as with `populate_row`).
    /// False when this registry has no row for the package.
    pub(super) fn copy_rows(&self, to: &mut AssetRegistry, path: &str) -> bool {
        let Some(package) = pak_path_to_game_path(PakPath::new(path)) else {
            return false;
        };
        let Some(rows) = self.by_package.get(&package.to_ascii_lowercase()) else {
            return false;
        };
        let from = &self.registry;
        let s = &from.store;
        let name = |to: &mut AssetRegistry, n: NameIndexFlagged| {
            NameIndexFlagged(to.get_name(&from.names[n]).0, n.1)
        };
        let export_path = |to: &mut AssetRegistry, e: &ExportPath| ExportPath {
            object_path: name(to, e.object_path),
            package_path: name(to, e.package_path),
            asset_class: name(to, e.asset_class),
        };
        fn push<T>(to: &mut Vec<T>, value: T) -> u32 {
            to.push(value);
            to.len() as u32 - 1
        }
        for row in rows.iter().map(|&i| &from.asset_data[i]) {
            if has_row(to, &from.names[row.object_path], row.object_path.1) {
                continue;
            }
            let pair_begin = to.store.pairs.len() as u32;
            for p in &s.pairs[row.tags.pair_begin as usize..][..row.tags.num as usize] {
                let i = p.index as usize;
                let index = match p.type_ {
                    Type::AnsiString => push(&mut to.store.ansi_strings, s.ansi_strings[i].clone()),
                    Type::WideString => push(&mut to.store.wide_strings, s.wide_strings[i].clone()),
                    Type::LocalizedText => push(&mut to.store.texts, s.texts[i].clone()),
                    Type::NumberlessName => {
                        let n = name(to, s.nbl_names[i]);
                        push(&mut to.store.nbl_names, n)
                    }
                    Type::Name => {
                        let n = name(to, s.names[i]);
                        push(&mut to.store.names, n)
                    }
                    Type::NumberlessExportPath => {
                        let e = export_path(to, &s.nbl_export_paths[i]);
                        push(&mut to.store.nbl_export_paths, e)
                    }
                    Type::ExportPath => {
                        let e = export_path(to, &s.export_paths[i]);
                        push(&mut to.store.export_paths, e)
                    }
                };
                let key = to.get_name(&from.names[p.name]).0;
                to.store.pairs.push(Pair {
                    name: name_index(key),
                    type_: p.type_,
                    index,
                });
            }
            let row = AssetData {
                object_path: name(to, row.object_path),
                package_path: name(to, row.package_path),
                asset_class: name(to, row.asset_class),
                package_name: name(to, row.package_name),
                asset_name: name(to, row.asset_name),
                tags: MapHandle {
                    has_numberless_keys: true,
                    num: row.tags.num,
                    pair_begin,
                },
                bundle_count: 0,
                chunk_ids: row.chunk_ids.clone(),
                flags: row.flags,
            };
            to.asset_data.push(row);
        }
        true
    }
}

/// uasset_utils keeps `NameIndex`'s field private, so one is made the way its reader makes it.
fn name_index(i: u32) -> NameIndex {
    NameIndex::read(&mut Cursor::new(i.to_le_bytes())).unwrap()
}

#[cfg(test)]
#[path = "mod_inputs/tests.rs"]
mod tests;
