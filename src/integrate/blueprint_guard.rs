//! Conservative validation of *referenced* cooked Blueprint symbols.
//!
//! The export table is not a full ABI. Function removals are legitimate in many
//! overrides. We only reject a vanilla replacement when a surviving mod package
//! has a serialized UFunction/class SuperIndex import pointing at an export that
//! vanilla provides and the replacement does not. This does NOT validate bytecode,
//! native interfaces, dynamic loads, function signatures, or vanilla dependants.

use std::collections::{BTreeMap, HashSet};
use std::io::Cursor;

use unreal_asset::AssetBuilder;
use unreal_asset::engine_version::EngineVersion;
use unreal_asset::exports::ExportBaseTrait;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum RequiredSymbol {
    Class(String),
    Function { owner: String, function: String },
}

impl RequiredSymbol {
    pub(super) fn display(&self) -> String {
        match self {
            Self::Class(class) => format!("class {class}"),
            Self::Function { owner, function } => format!("function {owner}:{function}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct Dependency {
    /// Lower-case `FSD/Content/...` package path, no extension.
    pub(super) target: String,
    pub(super) symbol: RequiredSymbol,
}

#[derive(Clone, Debug, Default)]
pub(super) struct BlueprintScan {
    classes: HashSet<String>,
    functions: HashSet<(String, String)>,
    /// Only references actually used as an exported struct's SuperIndex.
    pub(super) dependencies: Vec<Dependency>,
}

impl BlueprintScan {
    pub(super) fn provides(&self, symbol: &RequiredSymbol) -> bool {
        match symbol {
            RequiredSymbol::Class(class) => self.classes.contains(&class.to_ascii_lowercase()),
            RequiredSymbol::Function { owner, function } => self
                .functions
                .contains(&(owner.to_ascii_lowercase(), function.to_ascii_lowercase())),
        }
    }
}

/// `/Game/Enemies/A.B` => `fsd/content/enemies/a`.
/// Do not mistake `/Script/...` or plugin imports for assets in the game pak.
fn game_package_key(path: &str) -> Option<String> {
    let path = path.replace('\\', "/");
    let base = path.split('.').next()?;
    if !base.get(..6).is_some_and(|prefix| prefix.eq_ignore_ascii_case("/game/")) {
        return None;
    }
    Some(format!("fsd/content/{}", base[6..].to_ascii_lowercase()))
}

/// Resolve a SuperIndex import: Function -> generated class -> Package.
/// Do not treat every import-map entry as a hard dependency: imports can be
/// unreferenced. SuperIndex is strong evidence that the linker will need it.
fn super_dependency(
    imports: &[unreal_asset::Import],
    start: i32,
) -> Option<Dependency> {
    if start >= 0 {
        return None;
    }
    let function_or_class = imports.get((-1_i64 - i64::from(start)) as usize)?;
    let import_type = function_or_class.class_name.get_owned_content();
    let symbol_name = function_or_class.object_name.get_owned_content();
    let mut outer = function_or_class.outer_index.index;
    let symbol = if import_type.eq_ignore_ascii_case("Function") {
        if outer >= 0 {
            return None;
        }
        let class_import = imports.get((-1_i64 - i64::from(outer)) as usize)?;
        let owner = class_import.object_name.get_owned_content();
        outer = class_import.outer_index.index;
        RequiredSymbol::Function {
            owner: owner.to_ascii_lowercase(),
            function: symbol_name.to_ascii_lowercase(),
        }
    } else if import_type.eq_ignore_ascii_case("Class")
        || import_type.eq_ignore_ascii_case("BlueprintGeneratedClass")
        || import_type.eq_ignore_ascii_case("WidgetBlueprintGeneratedClass")
        || import_type.eq_ignore_ascii_case("AnimBlueprintGeneratedClass")
    {
        RequiredSymbol::Class(symbol_name.to_ascii_lowercase())
    } else {
        return None;
    };

    // The outer chain normally ends at a Package import named /Game/....
    // Bound traversal to protect against malformed or cyclic import tables.
    for _ in 0..imports.len() {
        if outer >= 0 {
            return None;
        }
        let package_or_outer = imports.get((-1_i64 - i64::from(outer)) as usize)?;
        let path = package_or_outer.object_name.get_owned_content();
        if let Some(target) = game_package_key(&path) {
            return Some(Dependency { target, symbol });
        }
        outer = package_or_outer.outer_index.index;
    }
    None
}

/// Parse exported Blueprint identities and exported struct SuperIndex imports.
/// `skip_data(true)` avoids deserializing Blueprint bytecode.
pub(super) fn scan(uasset: &[u8], uexp: &[u8]) -> Result<BlueprintScan, unreal_asset::Error> {
    let asset = AssetBuilder::new(Cursor::new(uasset), EngineVersion::VER_UE4_27)
        .bulk(Cursor::new(uexp))
        .skip_data(true)
        .build()?;
    let exports = &asset.asset_data.exports;
    let mut result = BlueprintScan::default();
    let mut class_indices = HashSet::new();

    for (index, export) in exports.iter().enumerate() {
        let base = export.get_base_export();
        let name = base.object_name.get_owned_content();
        if name.ends_with("_C") && !name.starts_with("Default__") && base.outer_index.index == 0 {
            class_indices.insert(index as i32 + 1);
            result.classes.insert(name.to_ascii_lowercase());
        }
        if let Some(dependency) = super_dependency(&asset.imports, base.super_index.index) {
            result.dependencies.push(dependency);
        }
    }

    for export in exports {
        let base = export.get_base_export();
        if !class_indices.contains(&base.outer_index.index) {
            continue;
        }
        let is_function = if base.class_index.index < 0 {
            asset.imports
                .get((-1_i64 - i64::from(base.class_index.index)) as usize)
                .is_some_and(|import| import.object_name.get_owned_content() == "Function")
        } else {
            false
        };
        if !is_function {
            continue;
        }
        let class = &exports[(base.outer_index.index - 1) as usize];
        let owner = class.get_base_export().object_name.get_owned_content();
        let name = base.object_name.get_owned_content();
        result
            .functions
            .insert((owner.to_ascii_lowercase(), name.to_ascii_lowercase()));
    }
    result.dependencies.sort_by(|a, b| {
        (&a.target, a.symbol.display()).cmp(&(&b.target, b.symbol.display()))
    });
    result.dependencies.dedup();
    Ok(result)
}

/// Only reject symbols that the *vanilla* package exports and a surviving mod
/// package needs, but that the winning replacement no longer exports.
pub(super) fn broken_references<'a>(
    vanilla: &BlueprintScan,
    replacement: &BlueprintScan,
    incoming: impl IntoIterator<Item = &'a Dependency>,
) -> Vec<String> {
    let mut missing = BTreeMap::<String, ()>::new();
    for dep in incoming {
        if vanilla.provides(&dep.symbol) && !replacement.provides(&dep.symbol) {
            missing.insert(dep.symbol.display(), ());
        }
    }
    missing.into_keys().collect()
}

/// Exclude the whole cooked-package family; keep the game's vanilla files.
pub(super) fn rejected_sidecar(path: &str, rejected: &HashSet<String>) -> bool {
    let lower = path.to_ascii_lowercase();
    [".m.ubulk", ".o.ubulk", ".uptnl", ".uasset", ".uexp", ".ubulk"]
        .iter()
        .find_map(|suffix| lower.strip_suffix(suffix))
        .is_some_and(|stem| rejected.contains(stem))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo(class: &str, functions: &[&str]) -> BlueprintScan {
        let mut scan = BlueprintScan::default();
        scan.classes.insert(class.to_ascii_lowercase());
        for name in functions {
            scan.functions.insert((class.to_ascii_lowercase(), name.to_ascii_lowercase()));
        }
        scan
    }

    fn needed(target: &str, class: &str, name: &str) -> Dependency {
        Dependency {
            target: target.to_string(),
            symbol: RequiredSymbol::Function {
                owner: class.to_ascii_lowercase(),
                function: name.to_ascii_lowercase(),
            },
        }
    }

    #[test]
    fn obnc_replacement_breaks_elytras_startwave_super_index() {
        let key = "fsd/content/enemies/waves/wavecontrollers/ewc_sw_plague_rockpoxinfectedenemies";
        let old = demo("EWC_SW_Plague_RockpoxInfectedEnemies_C", &["StartWave"]);
        let obnc = demo("EWC_Generic_C", &["StartWave"]);
        let reference = needed(key, "EWC_SW_Plague_RockpoxInfectedEnemies_C", "StartWave");
        assert_eq!(broken_references(&old, &obnc, [&reference]), vec!["function ewc_sw_plague_rockpoxinfectedenemies_c:startwave"]);
    }

    #[test]
    fn intentional_removed_function_with_no_dependants_is_allowed() {
        let old = demo("HUD_SentryGunManager_C", &["AddSentry", "PreConstruct"]);
        let simplified = demo("HUD_SentryGunManager_C", &["AddSentry"]);
        assert!(broken_references(&old, &simplified, []).is_empty());
    }

    #[test]
    fn same_function_in_replacement_satisfies_requirement() {
        let old = demo("Example_C", &["StartWave"]);
        let replacement = demo("Example_C", &["StartWave", "Extra"]);
        let dep = needed("fsd/content/example", "Example_C", "StartWave");
        assert!(broken_references(&old, &replacement, [&dep]).is_empty());
    }

    #[test]
    fn non_vanilla_symbol_is_not_a_reason_to_reject() {
        let old = demo("Example_C", &["Present"]);
        let replacement = demo("Example_C", &["Present"]);
        let dep = needed("fsd/content/example", "Example_C", "NotInVanilla");
        assert!(broken_references(&old, &replacement, [&dep]).is_empty());
    }

    #[test]
    fn package_key_handles_game_path_and_ignores_script() {
        assert_eq!(game_package_key("/Game/Enemies/A.A"), Some("fsd/content/enemies/a".to_string()));
        assert_eq!(game_package_key("/Script/CoreUObject"), None);
    }

    #[test]
    fn rejected_package_excludes_its_uexp_and_bulk_sidecars() {
        let rejected = HashSet::from(["fsd/content/a".to_string()]);
        for suffix in [".uasset", ".uexp", ".m.ubulk", ".o.ubulk", ".ubulk", ".uptnl"] {
            assert!(rejected_sidecar(&format!("FSD/Content/A{suffix}"), &rejected));
        }
        assert!(!rejected_sidecar("FSD/Content/AB.uasset", &rejected));
    }
}
