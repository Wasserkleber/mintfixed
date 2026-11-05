//! Build-time, mod.io-only first-run preset.
//! Change assets/preinstall.json and rebuild to ship a different preset.
use std::collections::HashSet;

use serde::Deserialize;

use crate::providers::{ModSpecification, modio};
use crate::state::{
    MemberOverrides, ModConfig, ModData_v0_1_0 as ModData, ModGroup, ModOrGroup,
    ModProfile_v0_1_0 as ModProfile,
};

const EMBEDDED_PRESET: &str = include_str!("../assets/preinstall.json");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Preset {
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default = "default_profile")]
    profile: String,
    #[serde(default)]
    items: Vec<PresetItem>,
}

fn default_profile() -> String {
    "default".to_owned()
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum PresetItem {
    Mod {
        url: String,
        #[serde(default)]
        description: Option<String>,
        #[serde(default = "default_true")]
        enabled: bool,
        #[serde(default)]
        required: bool,
        #[serde(default)]
        priority: i32,
    },
    Group {
        name: String,
        #[serde(default)]
        description: Option<String>,
        #[serde(default = "default_true")]
        enabled: bool,
        #[serde(default)]
        mods: Vec<PresetMod>,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PresetMod {
    url: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    priority: i32,
}

fn clean_description(description: Option<String>) -> Option<String> {
    description.map(|value| value.trim().to_owned()).filter(|value| !value.is_empty())
}

fn config(url: String, enabled: bool, required: bool, priority: i32, description: Option<String>) -> ModConfig {
    ModConfig {
        id: crate::state::new_id(),
        spec: ModSpecification::new(url),
        description: clean_description(description),
        member_id: None,
        required,
        enabled,
        priority,
    }
}

fn validate_mod(url: &str, seen: &mut HashSet<String>) -> Result<(), String> {
    if !modio::is_valid_modio_url(url) {
        return Err(format!("not a supported DRG mod.io URL: {url}"));
    }
    if !seen.insert(url.to_owned()) {
        return Err(format!("duplicate mod.io URL in bundled preset: {url}"));
    }
    Ok(())
}

/// Populate a newly created mod_data file with preset entries only.
/// Returns whether enabled mods were added; downloading/installing is exclusively manual.
/// This is deliberately not called for existing installations or legacy migrations.
pub(crate) fn apply(data: &mut ModData) -> Result<bool, String> {
    apply_json(EMBEDDED_PRESET, data)
}

fn apply_json(raw: &str, data: &mut ModData) -> Result<bool, String> {
    let preset: Preset = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    if !preset.enabled || preset.items.is_empty() {
        return Ok(false);
    }
    if preset.profile.trim().is_empty() {
        return Err("preset profile must not be empty".into());
    }

    // Validate everything before mutating mod_data, so errors cannot partially seed a profile.
    let mut seen_mods = HashSet::new();
    let mut seen_groups = HashSet::new();
    for item in &preset.items {
        match item {
            PresetItem::Mod { url, .. } => validate_mod(url, &mut seen_mods)?,
            PresetItem::Group { name, mods, .. } => {
                if name.trim().is_empty() || !seen_groups.insert(name.clone()) {
                    return Err(format!("invalid or duplicate preset group: {name:?}"));
                }
                for m in mods {
                    validate_mod(&m.url, &mut seen_mods)?;
                }
            }
        }
    }

    let mut profile = ModProfile::default();
    let mut groups = std::collections::BTreeMap::new();
    for item in preset.items {
        match item {
            PresetItem::Mod { url, description, enabled, required, priority } => {
                profile.mods.push(ModOrGroup::Individual(config(url, enabled, required, priority, description)));
            }
            PresetItem::Group { name, description, enabled, mods } => {
                groups.insert(name.clone(), ModGroup {
                    id: crate::state::new_id(),
                    description: clean_description(description),
                    mods: mods.into_iter().map(|m| config(m.url, m.enabled, m.required, m.priority, m.description)).collect(),
                });
                profile.mods.push(ModOrGroup::Group {
                    id: crate::state::new_id(),
                    group_name: name,
                    enabled,
                    member_overrides: MemberOverrides::default(),
                });
            }
        }
    }
    data.active_profile = preset.profile.clone();
    data.profiles.clear();
    data.profiles.insert(preset.profile, profile);
    data.groups = groups;
    Ok(!data.ordered_install_specs(&data.active_profile).is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_individuals_and_order_survive_seeding() {
        let mut data = ModData::default();
        let has_enabled = apply_json(r#"{
          "profile": "Starter",
          "items": [
            {"type":"group", "name":"Tools", "mods":[
              {"url":"https://mod.io/g/drg/m/tool-one", "priority":10},
              {"url":"https://mod.io/g/drg/m/tool-two", "enabled":false}
            ]},
            {"type":"mod", "url":"https://mod.io/g/drg/m/single", "enabled":true}
          ]
        }"#, &mut data).unwrap();
        assert!(has_enabled);
        assert_eq!(data.active_profile, "Starter");
        assert_eq!(data.profiles["Starter"].mods.len(), 2);
        assert_eq!(data.groups["Tools"].mods.len(), 2);
        assert!(!data.groups["Tools"].mods[1].enabled);
        let installed = data.ordered_install_specs("Starter");
        assert_eq!(installed.len(), 2);
        assert_eq!(installed[0].url, "https://mod.io/g/drg/m/tool-one");
        assert_eq!(installed[1].url, "https://mod.io/g/drg/m/single");
    }

    #[test]
    fn disabled_top_level_preset_skips_all_entries() {
        let mut data = ModData::default();
        let had_enabled_mods = apply_json(r#"{
          "enabled": false,
          "profile": "Starter",
          "items": [
            {"type":"mod", "url":"https://evil.example/not-a-modio-url"},
            {"type":"group", "name":"Utilities", "mods":[
              {"url":"https://mod.io/g/drg/m/example"}
            ]}
          ]
        }"#, &mut data).unwrap();
        assert!(!had_enabled_mods);
        assert_eq!(data.active_profile, "default");
        assert!(data.profiles["default"].mods.is_empty());
        assert!(!data.groups.contains_key("Utilities"));
    }


    #[test]
    fn descriptions_survive_mod_data_serialization() {
        let mut data = ModData::default();
        apply_json(r#"{
          "profile": "Starter",
          "items": [
            {"type":"mod", "url":"https://mod.io/g/drg/m/standalone", "description":"Standalone help"},
            {"type":"group", "name":"Utilities", "description":"Utilities help", "mods":[
              {"url":"https://mod.io/g/drg/m/in-group", "description":"Group member help"}
            ]}
          ]
        }"#, &mut data).unwrap();
        let restored: ModData = serde_json::from_str(&serde_json::to_string(&data).unwrap()).unwrap();
        let items = &restored.profiles["Starter"].mods;
        let ModOrGroup::Individual(single) = &items[0] else { panic!("expected standalone mod") };
        assert_eq!(single.description.as_deref(), Some("Standalone help"));
        assert_eq!(restored.groups["Utilities"].description.as_deref(), Some("Utilities help"));
        assert_eq!(restored.groups["Utilities"].mods[0].description.as_deref(), Some("Group member help"));
    }

    #[test]
    fn old_mod_configs_without_descriptions_still_deserialize() {
        let mut data = ModData::default();
        apply_json(r#"{"items":[{"type":"mod", "url":"https://mod.io/g/drg/m/old"}]}"#, &mut data).unwrap();
        let ModOrGroup::Individual(mod_config) = &data.profiles["default"].mods[0] else { panic!("expected mod") };
        assert!(mod_config.description.is_none());
    }

    #[test]
    fn invalid_preset_does_not_touch_data() {
        let mut data = ModData::default();
        let err = apply_json(r#"{"items":[{"type":"mod", "url":"https://evil.example/mod"}]}"#, &mut data);
        assert!(err.is_err());
        assert!(data.profiles["default"].mods.is_empty());
    }

    #[test]
    fn empty_preset_preserves_initial_state() {
        let mut data = ModData::default();
        assert!(!apply_json(r#"{"items":[]}"#, &mut data).unwrap());
        assert!(data.groups.contains_key("default"));
    }

    #[test]
    fn disabled_preset_never_installs() {
        let mut data = ModData::default();
        assert!(!apply_json(r#"{"items":[{"type":"mod", "url":"https://mod.io/g/drg/m/example", "enabled":false}]}"#, &mut data).unwrap());
        assert_eq!(data.profiles["default"].mods.len(), 1);
    }
}
