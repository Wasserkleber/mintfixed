use uasset_utils::asset_registry::{Dependencies, Names, Store, Writable};

use super::*;

/// A registry holding one row per `(package, class, tags)`, each tag an ANSI string, plus a `Parent` name tag
/// when the value starts with `/`, to cover the remapping of a name value.
/// (package, class, tags) of one row.
type Row<'a> = (&'a str, &'a str, &'a [(&'a str, &'a str)]);

fn registry(rows: &[Row]) -> AssetRegistry {
    let mut r = AssetRegistry {
        version: [7; 16],
        version_int: 8,
        hash_version: 0,
        names: Names(Default::default()),
        store: Store {
            pair_count: 0,
            texts: vec![],
            nbl_names: vec![],
            names: vec![],
            nbl_export_paths: vec![],
            export_paths: vec![],
            ansi_strings: vec![],
            wide_strings: vec![],
            pairs: vec![],
        },
        asset_data: vec![],
        dependencies: Dependencies {
            dependencies_size: 0,
            dependencies: vec![],
            package_data_buffer_size: 0,
        },
    };
    for (package, class, tags) in rows {
        let asset = package.rsplit('/').next().unwrap();
        let pair_begin = r.store.pairs.len() as u32;
        for (key, value) in *tags {
            let name = name_index(r.get_name(key).0);
            let (type_, index) = if value.starts_with('/') {
                let n = r.get_name(value);
                r.store.nbl_names.push(n);
                (Type::NumberlessName, r.store.nbl_names.len() - 1)
            } else {
                r.store.ansi_strings.push(value.to_string());
                (Type::AnsiString, r.store.ansi_strings.len() - 1)
            };
            r.store.pairs.push(Pair {
                name,
                type_,
                index: index as u32,
            });
        }
        let row = AssetData {
            object_path: r.get_name(&format!("{package}.{asset}")),
            package_path: r.get_name(package.rsplit_once('/').unwrap().0),
            asset_class: r.get_name(class),
            package_name: r.get_name(package),
            asset_name: r.get_name(asset),
            tags: MapHandle {
                has_numberless_keys: true,
                num: tags.len() as u16,
                pair_begin,
            },
            bundle_count: 0,
            chunk_ids: vec![],
            flags: 0,
        };
        r.asset_data.push(row);
    }
    r
}

fn bytes(r: &AssetRegistry) -> Vec<u8> {
    let mut out = vec![];
    r.write(&mut out).unwrap();
    out
}

/// The tags of the row for `object_path`, read back as (key, value) strings.
fn tags(r: &AssetRegistry, object_path: &str) -> Vec<(String, String)> {
    let row = r
        .asset_data
        .iter()
        .find(|a| r.names[a.object_path] == object_path)
        .unwrap();
    r.store.pairs[row.tags.pair_begin as usize..][..row.tags.num as usize]
        .iter()
        .map(|p| {
            let i = p.index as usize;
            let value = match p.type_ {
                Type::AnsiString => r.store.ansi_strings[i].clone(),
                Type::NumberlessName => r.names[r.store.nbl_names[i]].clone(),
                t => panic!("unexpected {t:?}"),
            };
            (r.names[p.name].clone(), value)
        })
        .collect()
}

#[test]
fn mod_rows_keep_their_tags() {
    let mut game = registry(&[(
        "/Game/UI/M_Vanilla",
        "Material",
        &[("MaterialDomain", "MD_Surface")],
    )]);
    let ui = [
        ("MaterialDomain", "MD_UI"),
        ("Parent", "/Game/_ElytrasMods/M_Base.M_Base"),
    ];
    let ours = registry(&[
        (
            "/Game/_ElytrasMods/MI_Gold_UI",
            "MaterialInstanceConstant",
            &ui,
        ),
        // A replaced vanilla package keeps vanilla's row.
        (
            "/Game/UI/M_Vanilla",
            "Material",
            &[("MaterialDomain", "MD_UI")],
        ),
    ]);
    let ours = ModRegistry::parse(&bytes(&ours), &game).unwrap();

    // The package path as MINT sees it in the pak, any case.
    assert!(ours.copy_rows(&mut game, "FSD/Content/_ElytrasMods/mi_gold_ui"));
    assert!(ours.copy_rows(&mut game, "FSD/Content/UI/M_Vanilla"));
    assert!(!ours.copy_rows(&mut game, "FSD/Content/_ElytrasMods/Unlisted"));

    // Through the file MINT writes.
    let merged = AssetRegistry::read(&mut Cursor::new(bytes(&game))).unwrap();
    assert_eq!(merged.asset_data.len(), 2);
    assert_eq!(
        tags(&merged, "/Game/_ElytrasMods/MI_Gold_UI.MI_Gold_UI"),
        ui.map(|(k, v)| (k.to_string(), v.to_string()))
    );
    assert_eq!(
        tags(&merged, "/Game/UI/M_Vanilla.M_Vanilla"),
        [("MaterialDomain".to_string(), "MD_Surface".to_string())]
    );
}

#[test]
fn another_version_is_not_merged() {
    let game = registry(&[]);
    let mut ours = registry(&[]);
    ours.version_int = 7;
    assert!(ModRegistry::parse(&bytes(&ours), &game).is_err());
    assert!(ModRegistry::parse(b"not a registry", &game).is_err());
}
