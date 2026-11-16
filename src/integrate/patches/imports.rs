// Mechanical source extraction from src/integrate.rs; no algorithm changes.
use super::*;

pub(super) type ImportChain<'a> = Vec<Import<'a>>;

pub(super) struct Import<'a> {
    class_package: &'a str,
    class_name: &'a str,
    object_name: &'a str,
}
impl<'a> Import<'a> {
    pub(super) fn new(
        class_package: &'a str,
        class_name: &'a str,
        object_name: &'a str,
    ) -> Import<'a> {
        Import {
            class_package,
            class_name,
            object_name,
        }
    }
}

pub(super) fn get_import<R: Read + Seek>(
    asset: &mut Asset<R>,
    import: ImportChain,
) -> PackageIndex {
    let mut pi = PackageIndex::new(0);
    for i in import {
        let ai = &asset
            .imports
            .iter()
            .enumerate()
            .find(|(_, ai)| {
                ai.class_package.get_content(|n| n == i.class_package)
                    && ai.class_name.get_content(|n| n == i.class_name)
                    && ai.object_name.get_content(|n| n == i.object_name)
                    && ai.outer_index == pi
            })
            .map(|(index, _)| PackageIndex::from_import(index as i32).unwrap());
        pi = ai.unwrap_or_else(|| {
            let new_import = unreal_asset::Import::new(
                asset.add_fname(i.class_package),
                asset.add_fname(i.class_name),
                pi,
                asset.add_fname(i.object_name),
                false,
            );
            asset.add_import(new_import)
        });
    }
    pi
}
