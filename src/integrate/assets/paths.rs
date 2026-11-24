// Mechanical source extraction from src/integrate.rs; no algorithm changes.
use super::*;

pub(super) fn supported_registry_asset_path(path: &str) -> bool {
    uasset_utils::paths::pak_path_to_game_path(PakPath::new(path)).is_some()
}

#[cfg(test)]
mod pak_asset_path_tests {
    use super::supported_registry_asset_path;

    #[test]
    fn malformed_enemy_cull_path_is_rejected() {
        assert!(!supported_registry_asset_path(
            "FSD/Jellyfish/ENE_Jelly_Spawn_Child"
        ));
    }

    #[test]
    fn valid_drg_asset_path_is_accepted() {
        assert!(supported_registry_asset_path(
            "FSD/Content/Enemies/Jellyfish/ENE_Jelly_Spawn_Child"
        ));
    }

    #[test]
    fn engine_content_paths_remain_supported() {
        assert!(supported_registry_asset_path(
            "Engine/Content/EngineMaterials/DefaultMaterial"
        ));
    }
}
