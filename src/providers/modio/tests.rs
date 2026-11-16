use super::*;
use crate::state::config::ConfigWrapper;

#[test]
fn test_modio_url() {
    let valid_mod = ModIoModUrl {
        name_id: "build-inspector",
        mod_id: None,
        modfile_id: None,
    };
    assert_eq!(
        parse_url("https://mod.io/g/drg/m/build-inspector").ok(),
        Some(valid_mod)
    );
    assert_eq!(
        parse_url("https://mod.io/g/drg/m/build-inspector/").ok(),
        Some(valid_mod)
    );
    assert_eq!(
        parse_url("https://mod.io/g/drg/m/build-inspector#discussion").ok(),
        Some(valid_mod)
    );
    assert_eq!(
        parse_url("https://mod.io/g/drg/m/build-inspector/#discussion").ok(),
        Some(valid_mod)
    );

    let valid_mod = ModIoModUrl {
        name_id: "build-inspector",
        mod_id: Some(2101319),
        modfile_id: None,
    };
    assert_eq!(
        parse_url("https://mod.io/g/drg/m/build-inspector#2101319").ok(),
        Some(valid_mod)
    );
    assert_eq!(
        parse_url("https://mod.io/g/drg/m/build-inspector/#2101319").ok(),
        Some(valid_mod)
    );

    let valid_mod = ModIoModUrl {
        name_id: "build-inspector",
        mod_id: Some(2101319),
        modfile_id: Some(3169221),
    };
    assert_eq!(
        parse_url("https://mod.io/g/drg/m/build-inspector#2101319/3169221").ok(),
        Some(valid_mod)
    );
    assert_eq!(
        parse_url("https://mod.io/g/drg/m/build-inspector/#2101319/3169221").ok(),
        Some(valid_mod)
    );

    // should no panic
    parse_url("https://mod.io/g/drg/m/build-inspector#12345678912456789123456789").ok();
}

#[test]
fn unavailable_modio_error_classification_is_strict() {
    assert!(unavailable_modio_status(Some(404), false));
    assert!(unavailable_modio_status(Some(410), false));
    assert!(!unavailable_modio_status(Some(403), false));
    assert!(!unavailable_modio_status(Some(403), true));
    assert!(!unavailable_modio_status(Some(401), true));
    assert!(!unavailable_modio_status(Some(429), false));
    assert!(!unavailable_modio_status(Some(500), false));
    assert!(!unavailable_modio_status(None, false));
    assert!(is_unavailable_error(&ProviderError::NoModsForNameId {
        name_id: "deleted".into(),
    }));
    assert!(!is_unavailable_error(&ProviderError::DrgModioError {
        source: DrgModioError::MissingOauthToken,
    }));
}

#[tokio::test]
async fn deleted_mod_does_not_block_other_cache_updates() {
    let cache = Arc::new(RwLock::new(ConfigWrapper::<VersionAnnotatedCache>::memory(
        VersionAnnotatedCache::default(),
    )));
    let old = ModioMod {
        name_id: "deleted".into(),
        name: "Deleted mod".into(),
        latest_modfile: Some(1),
        modfiles: Vec::new(),
        tags: HashSet::new(),
    };
    let updated = ModioMod {
        name_id: "working".into(),
        name: "Working mod (updated)".into(),
        latest_modfile: Some(22),
        modfiles: Vec::new(),
        tags: HashSet::new(),
    };
    {
        let mut lock = cache.write().unwrap();
        let state = lock.get_mut::<ModioCache>(MODIO_PROVIDER_ID);
        state.mods.insert(1, old.clone());
        state.mods.insert(
            2,
            ModioMod {
                latest_modfile: Some(2),
                ..updated.clone()
            },
        );
        state.mod_id_map.insert("deleted".into(), 1);
        state.mod_id_map.insert("working".into(), 2);
        state.dependencies.insert(1, Vec::new());
        state.dependencies.insert(2, Vec::new());
        state.last_update_time = Some(UNIX_EPOCH);
    }
    let mut mock = MockDrgModio::new();
    mock.expect_fetch_mod_updates_since()
        .times(1)
        .returning(|_, _| Ok([1, 2].into_iter().collect()));
    mock.expect_fetch_mod().returning(move |_, id| {
        if id == 1 {
            Err(DrgModioError::GenericError {
                msg: "mod.io returned no mod for the requested ID",
            })
        } else {
            Ok(updated.clone())
        }
    });
    mock.expect_fetch_dependencies()
        .returning(|_, _| Ok(Vec::new()));
    let provider = ModioProvider::new(mock);
    provider.update_cache(cache.clone()).await.unwrap();
    assert!(is_unavailable(&cache, "https://mod.io/g/drg/m/deleted#1"));
    assert!(!is_unavailable(&cache, "https://mod.io/g/drg/m/working#2"));
    let lock = cache.read().unwrap();
    let c = lock.get::<ModioCache>(MODIO_PROVIDER_ID).unwrap();
    assert_eq!(c.mods.get(&1), Some(&old));
    assert_eq!(c.mods.get(&2).unwrap().latest_modfile, Some(22));
    assert!(c.last_update_time.unwrap() > UNIX_EPOCH);
}

#[tokio::test]
async fn test_check_pass() {
    let mut mock = MockDrgModio::new();
    mock.expect_check().times(1).returning(|| Ok(()));
    let modio_provider = ModioProvider::new(mock);
    assert!(modio_provider.check().await.is_ok());
}

#[tokio::test]
async fn test_check_fail() {
    let mut mock = MockDrgModio::new();
    mock.expect_check()
        .times(1)
        .returning(|| Err(DrgModioError::MissingOauthToken));
    let modio_provider = ModioProvider::new(mock);
    assert!(modio_provider.check().await.is_err());
}

struct FullMod {
    mod_: ModioMod,
    dependencies: Vec<u32>,
}

static MODS: OnceLock<HashMap<u32, FullMod>> = OnceLock::new();

#[tokio::test]
async fn test_fetch_mod_simple() {
    let mods = MODS.get_or_init(|| {
        [(
            3,
            FullMod {
                mod_: ModioMod {
                    name_id: "test-mod".to_string(),
                    name: "Test Mod".to_string(),
                    latest_modfile: Some(5),
                    modfiles: vec![ModioFile {
                        id: 5,
                        date_added: 12345,
                        version: None,
                        changelog: None,
                    }],
                    tags: HashSet::new(),
                },
                dependencies: vec![],
            },
        )]
        .into_iter()
        .collect::<HashMap<_, _>>()
    });
    let mod_names = mods
        .iter()
        .map(|(id, m)| (m.mod_.name_id.as_str(), id))
        .collect::<HashMap<_, _>>();
    let mut mock = MockDrgModio::new();

    mock.expect_fetch_mods_by_name()
        .times(1)
        .returning(move |name| {
            mod_names
                .get(name)
                .map(|id| vec![ModioModResponse { id: **id }])
                .ok_or(DrgModioError::GenericError { msg: "not found" })
        });
    mock.expect_fetch_mod().times(1).returning(move |_, id| {
        mods.get(&id)
            .map(|m| m.mod_.clone())
            .ok_or(DrgModioError::GenericError { msg: "not found" })
    });
    mock.expect_fetch_dependencies()
        .times(1)
        .returning(move |_, id| {
            mods.get(&id)
                .map(|m| m.dependencies.clone())
                .ok_or(DrgModioError::GenericError { msg: "not found" })
        });

    let cache = Arc::new(RwLock::new(ConfigWrapper::<VersionAnnotatedCache>::memory(
        VersionAnnotatedCache::default(),
    )));

    let modio_provider = ModioProvider::new(mock);
    let resolved_mod = modio_provider
        .resolve_mod(
            &ModSpecification::new("https://mod.io/g/drg/m/test-mod".to_string()),
            false,
            cache.clone(),
        )
        .await
        .unwrap();

    let resolved_mod = match resolved_mod {
        ModResponse::Redirect(spec) => spec,
        _ => unreachable!(),
    };
    let _resolved_mod = modio_provider
        .resolve_mod(&resolved_mod, false, cache.clone())
        .await
        .unwrap();
    let lock = cache.read().unwrap();
    let modio_cache = lock.get::<ModioCache>(MODIO_PROVIDER_ID).unwrap();

    assert_eq!(
        modio_cache.mod_id_map,
        [("test-mod".to_string(), 3)].into_iter().collect()
    );
    assert_eq!(
        modio_cache.dependencies,
        [(3, vec![])].into_iter().collect()
    );
    assert_eq!(
        modio_cache.mods,
        [(3, mods.get(&3).map(|m| m.mod_.clone()).unwrap())]
            .into_iter()
            .collect()
    );
}
