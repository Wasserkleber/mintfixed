pub mod config;
mod migration;

/// Persisted identity; independent of names, list positions and mod URLs.
pub fn new_id() -> String {
    format!("{:032x}", rand::random::<u128>())
}

use std::{
    collections::{BTreeMap, HashMap},
    ops::{Deref, DerefMut},
    path::PathBuf,
    sync::Arc,
};

use fs_err as fs;
use serde::{Deserialize, Serialize};
use snafu::prelude::*;

use self::config::ConfigWrapper;
use crate::{
    Dirs,
    gui::GuiTheme,
    providers::{ModSpecification, ModStore},
};
use crate::{gui::SortBy, providers::ProviderError};
use mint_lib::{DRGInstallation, mod_info::MetaConfig};

/// An enabled occurrence captured when installation starts, including its source.
#[derive(Clone, Debug)]
pub struct InstallEntry {
    pub spec: ModSpecification,
    pub location: String,
}

impl InstallEntry {
    pub fn requested(specs: &[ModSpecification]) -> Vec<Self> {
        specs
            .iter()
            .enumerate()
            .map(|(index, spec)| Self {
                spec: spec.clone(),
                location: format!("requested mod #{} ({})", index + 1, spec.url),
            })
            .collect()
    }
}

/// Mod configuration, holds ModSpecification as well as other metadata
#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct ModConfig {
    #[serde(default = "new_id")]
    pub id: String,
    pub spec: ModSpecification,
    /// Optional build-preset help text; saved with the mod, not the provider cache.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_id: Option<String>,

    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub priority: i32,
}

#[derive(Default, Debug, Clone, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MemberOverrides(BTreeMap<String, bool>);

impl MemberOverrides {
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn enabled(&self, member: &ModConfig) -> bool {
        member
            .member_id
            .as_ref()
            .and_then(|id| self.0.get(id))
            .copied()
            .unwrap_or(member.enabled)
    }

    pub fn set(&mut self, member: &mut ModConfig, enabled: bool) {
        if enabled == member.enabled {
            if let Some(id) = &member.member_id {
                self.0.remove(id);
            }
        } else {
            let id = member
                .member_id
                .get_or_insert_with(|| format!("{:032x}", rand::random::<u128>()));
            self.0.insert(id.clone(), enabled);
        }
    }

    pub fn forget(&mut self, id: &str) {
        self.0.remove(id);
    }
}

fn default_true() -> bool {
    true
}

fn is_zero(value: &i32) -> bool {
    *value == 0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModGroup {
    #[serde(default = "new_id")]
    pub id: String,
    /// Optional description of this shared collection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tint_color: Option<String>,
    pub mods: Vec<ModConfig>,
}

#[obake::versioned]
#[obake(version("0.0.0"))]
#[obake(version("0.1.0"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModProfile {
    #[obake(cfg("0.1.0"))]
    #[serde(default = "new_id")]
    pub id: String,
    #[obake(cfg("0.0.0"))]
    pub mods: Vec<ModConfig>,

    /// A profile can contain ordered individual mods mixed with mod groups.
    #[obake(cfg("0.1.0"))]
    pub mods: Vec<ModOrGroup>,
}

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ModOrGroup {
    Group {
        #[serde(default = "new_id")]
        id: String,
        group_name: String,
        enabled: bool,
        #[serde(default, skip_serializing_if = "MemberOverrides::is_empty")]
        member_overrides: MemberOverrides,
    },
    Individual(ModConfig),
}

impl From<ModProfile!["0.0.0"]> for ModProfile!["0.1.0"] {
    fn from(_legacy: ModProfile!["0.0.0"]) -> Self {
        // The migration requires `ModData` to handle instead.
        unimplemented!("migration requires handling from `ModData`")
    }
}

#[obake::versioned]
#[obake(version("0.0.0"))]
#[obake(version("0.1.0"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModData {
    pub active_profile: String,
    #[obake(cfg("0.0.0"))]
    pub profiles: BTreeMap<String, ModProfile!["0.0.0"]>,
    #[obake(cfg("0.1.0"))]
    pub profiles: BTreeMap<String, ModProfile!["0.1.0"]>,
    #[obake(cfg("0.1.0"))]
    pub groups: BTreeMap<String, ModGroup>,
    /// Lua bitfix scripts are global, independent of active_profile.
    #[obake(cfg("0.1.0"))]
    #[serde(default)]
    pub bitfix_scripts: Vec<ModConfig>,
}

impl ModData!["0.1.0"] {
    pub fn for_each_mod_predicate<
        F: FnMut(&ModConfig),
        G: FnMut(bool /* mod group enabled? */) -> bool,
        P: FnMut(&ModConfig) -> bool,
    >(
        &self,
        profile: &str,
        mut f: F,
        mut g: G,
        mut p: P,
    ) {
        for ref mod_or_group in &self.profiles.get(profile).unwrap().mods {
            match mod_or_group {
                ModOrGroup::Group {
                    group_name,
                    enabled,
                    ..
                } => {
                    if g(*enabled) {
                        for mc in &self.groups.get(group_name).unwrap().mods {
                            if p(mc) {
                                f(mc);
                            }
                        }
                    }
                }
                ModOrGroup::Individual(mc) => {
                    if p(mc) {
                        f(mc);
                    }
                }
            }
        }
    }

    pub fn for_each_mod_predicate_mut<
        F: FnMut(&mut ModConfig),
        G: FnMut(bool /* mod group enabled? */) -> bool,
        P: FnMut(&mut ModConfig) -> bool,
    >(
        &mut self,
        profile: &str,
        mut f: F,
        mut g: G,
        mut p: P,
    ) {
        for ref mut mod_or_group in &mut self.profiles.get_mut(profile).unwrap().mods {
            match mod_or_group {
                ModOrGroup::Group {
                    group_name,
                    enabled,
                    ..
                } => {
                    if g(*enabled) {
                        for mc in &mut self.groups.get_mut(group_name).unwrap().mods {
                            if p(mc) {
                                f(mc);
                            }
                        }
                    }
                }
                ModOrGroup::Individual(mc) => {
                    if p(mc) {
                        f(mc);
                    }
                }
            }
        }
    }

    pub fn for_each_mod<F: FnMut(&ModConfig)>(&self, profile: &str, f: F) {
        self.for_each_mod_predicate(profile, f, |_| true, |_| true)
    }

    pub fn for_each_enabled_mod(&self, profile: &str, mut f: impl FnMut(&ModConfig)) {
        for item in &self.profiles[profile].mods {
            match item {
                ModOrGroup::Individual(mc) if mc.enabled => f(mc),
                ModOrGroup::Group {
                    group_name,
                    enabled: true,
                    member_overrides,
                    ..
                } => {
                    for mc in &self.groups[group_name].mods {
                        if member_overrides.enabled(mc) {
                            f(mc);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    pub fn ordered_install_entries(&self, profile: &str) -> Vec<InstallEntry> {
        let mut entries = Vec::new();
        // Global Bitfix scripts are installed regardless of the active profile.
        for (index, mc) in self.bitfix_scripts.iter().enumerate() {
            if mc.enabled {
                entries.push((
                    mc.priority,
                    InstallEntry {
                        spec: mc.spec.clone(),
                        location: format!("global Bitfix script #{}", index + 1),
                    },
                ));
            }
        }
        for (row, item) in self.profiles[profile].mods.iter().enumerate() {
            match item {
                ModOrGroup::Individual(mc) if mc.enabled && !is_bitfix_url(&mc.spec.url) => entries
                    .push((
                        mc.priority,
                        InstallEntry {
                            spec: mc.spec.clone(),
                            location: format!("profile {profile:?}, standalone row {}", row + 1),
                        },
                    )),
                ModOrGroup::Group {
                    group_name,
                    enabled: true,
                    member_overrides,
                    ..
                } => {
                    for (index, mc) in self.groups[group_name].mods.iter().enumerate() {
                        if member_overrides.enabled(mc) && !is_bitfix_url(&mc.spec.url) {
                            entries.push((mc.priority, InstallEntry {
                                spec: mc.spec.clone(),
                                location: format!("profile {profile:?}, group {group_name:?} (row {}), mod #{}", row + 1, index + 1),
                            }));
                        }
                    }
                }
                _ => {}
            }
        }
        // Stable sorting keeps saved order when priorities are equal.
        entries.sort_by_key(|(priority, _)| std::cmp::Reverse(*priority));
        entries.into_iter().map(|(_, entry)| entry).collect()
    }

    pub fn ordered_install_specs(&self, profile: &str) -> Vec<ModSpecification> {
        self.ordered_install_entries(profile)
            .into_iter()
            .map(|entry| entry.spec)
            .collect()
    }

    pub fn for_each_mod_mut<F: FnMut(&mut ModConfig)>(&mut self, profile: &str, f: F) {
        self.for_each_mod_predicate_mut(profile, f, |_| true, |_| true)
    }

    pub fn any_mod<F: FnMut(&ModConfig, Option<bool> /* mod group enabled? */) -> bool>(
        &self,
        profile: &str,
        mut f: F,
    ) -> bool {
        self.profiles.get(profile).unwrap().mods.iter().any(|m| {
            let f = &mut f;
            match m {
                ModOrGroup::Individual(mc) => f(mc, None),
                ModOrGroup::Group {
                    group_name,
                    enabled,
                    ..
                } => self
                    .groups
                    .get(group_name)
                    .unwrap()
                    .mods
                    .iter()
                    .any(|mc| f(mc, Some(*enabled))),
            }
        })
    }

    pub fn any_mod_mut<
        F: FnMut(&mut ModConfig, Option<&mut bool> /* mod group enabled? */) -> bool,
    >(
        &mut self,
        profile: &str,
        mut f: F,
    ) -> bool {
        self.profiles
            .get_mut(profile)
            .unwrap()
            .mods
            .iter_mut()
            .any(|m| {
                let f = &mut f;
                match m {
                    ModOrGroup::Individual(mc) => f(mc, None),
                    ModOrGroup::Group {
                        group_name,
                        enabled,
                        ..
                    } => self
                        .groups
                        .get_mut(group_name)
                        .unwrap()
                        .mods
                        .iter_mut()
                        .any(|mc| f(mc, Some(enabled))),
                }
            })
    }

    pub fn enable_dependency(&mut self, profile: &str, spec: &ModSpecification) -> bool {
        for item in &mut self.profiles.get_mut(profile).unwrap().mods {
            match item {
                ModOrGroup::Individual(mc) if mc.spec.satisfies_dependency(spec) => {
                    mc.enabled = true;
                    return true;
                }
                ModOrGroup::Group {
                    group_name,
                    enabled,
                    member_overrides,
                    ..
                } => {
                    let group = self.groups.get_mut(group_name).unwrap();
                    if let Some(mc) = group
                        .mods
                        .iter_mut()
                        .find(|mc| mc.spec.satisfies_dependency(spec))
                    {
                        *enabled = true;
                        member_overrides.set(mc, true);
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }
}

impl Default for ModData!["0.1.0"] {
    fn default() -> Self {
        Self {
            active_profile: "default".to_string(),
            profiles: [("default".to_string(), Default::default())]
                .into_iter()
                .collect(),
            groups: [("default".to_string(), Default::default())]
                .into_iter()
                .collect(),
            bitfix_scripts: Vec::new(),
        }
    }
}

impl From<ModData!["0.0.0"]> for ModData!["0.1.0"] {
    fn from(legacy: ModData!["0.0.0"]) -> Self {
        let mut new_profiles = Vec::new();
        for (name, profile) in legacy.profiles {
            let new_profile = ModProfile_v0_1_0 {
                id: crate::state::new_id(),
                mods: profile
                    .mods
                    .into_iter()
                    .map(ModOrGroup::Individual)
                    .collect(),
            };
            new_profiles.push((name, new_profile));
        }

        Self {
            active_profile: legacy.active_profile,
            profiles: new_profiles.into_iter().collect(),
            groups: BTreeMap::default(),
            bitfix_scripts: Vec::new(),
        }
    }
}

/// Recognize `.lua` script paths and URLs without mistaking `.lua.zip` for a patch.
fn is_bitfix_url(url: &str) -> bool {
    url.split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase()
        .ends_with(".lua")
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "version")]
pub enum VersionAnnotatedModData {
    #[serde(rename = "0.0.0")]
    V0_0_0(ModData!["0.0.0"]),
    // The notag fork (Strappazzon/drg-mint-notag) writes this same layout as "0.2.0"; read it too, so a
    // notag install opens here. New writes use the stable-ID schema below.
    #[serde(rename = "0.1.0", alias = "0.2.0")]
    V0_1_0(ModData!["0.1.0"]),
    // Keep the internal obake type name for source compatibility. The disk schema is 0.3.0.
    #[serde(rename = "0.3.0")]
    V0_3_0(ModData!["0.1.0"]),
}

impl Default for ModData!["0.0.0"] {
    fn default() -> Self {
        Self {
            active_profile: "default".to_string(),
            profiles: [("default".to_string(), Default::default())]
                .into_iter()
                .collect(),
        }
    }
}

impl Default for VersionAnnotatedModData {
    fn default() -> Self {
        VersionAnnotatedModData::V0_3_0(Default::default())
    }
}

impl Deref for VersionAnnotatedModData {
    type Target = ModData!["0.1.0"];

    fn deref(&self) -> &Self::Target {
        match self {
            VersionAnnotatedModData::V0_0_0(_) => unreachable!(),
            VersionAnnotatedModData::V0_1_0(md) | VersionAnnotatedModData::V0_3_0(md) => md,
        }
    }
}

impl DerefMut for VersionAnnotatedModData {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            VersionAnnotatedModData::V0_0_0(_) => unreachable!(),
            VersionAnnotatedModData::V0_1_0(md) | VersionAnnotatedModData::V0_3_0(md) => md,
        }
    }
}

impl ModData!["0.1.0"] {
    pub fn get_active_profile(&self) -> &ModProfile!["0.1.0"] {
        &self.profiles[&self.active_profile]
    }

    pub fn get_active_profile_mut(&mut self) -> &mut ModProfile!["0.1.0"] {
        self.profiles.get_mut(&self.active_profile).unwrap()
    }

    pub fn remove_active_profile(&mut self) {
        self.profiles.remove(&self.active_profile);
        self.active_profile = self.profiles.keys().next().unwrap().to_string();
    }
}

#[obake::versioned]
#[obake(version("0.0.0"))]
#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub provider_parameters: HashMap<String, HashMap<String, String>>,
    pub drg_pak_path: Option<PathBuf>,
    /// When configured, newly discovered local mods are added to the active profile.
    #[serde(default)]
    pub local_mod_folder: Option<PathBuf>,
    pub gui_theme: Option<GuiTheme>,
    pub sorting_config: Option<SortingConfig>,
    // notag's config "0.1.0" adds these two; kept (unused here) so a notag install gets its settings back.
    #[serde(default = "default_true")]
    pub confirm_deletion: bool,
    #[serde(default = "default_true")]
    pub show_changelog: bool,
    /// Install and autolaunch the pinned BP-Crash-Reporter on integrate.
    #[serde(default = "default_true")]
    pub bundled_crash_reporter: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SortingConfig {
    pub sort_category: SortBy,
    pub is_ascending: bool,
}

impl Default for SortingConfig {
    fn default() -> Self {
        Self {
            sort_category: SortBy::Enabled,
            is_ascending: true,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "version")]
pub enum VersionAnnotatedConfig {
    // notag's "0.1.0" is this layout plus the two flags above; read it too. We still write our own "0.0.0".
    #[serde(rename = "0.0.0", alias = "0.1.0")]
    V0_0_0(Config!["0.0.0"]),
    #[serde(other)]
    Unsupported,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MaybeVersionedConfig {
    Versioned(VersionAnnotatedConfig),
    Legacy(Config!["0.0.0"]),
}

impl Default for MaybeVersionedConfig {
    fn default() -> Self {
        MaybeVersionedConfig::Versioned(Default::default())
    }
}

impl Default for VersionAnnotatedConfig {
    fn default() -> Self {
        VersionAnnotatedConfig::V0_0_0(Default::default())
    }
}

impl Deref for VersionAnnotatedConfig {
    type Target = Config!["0.0.0"];

    fn deref(&self) -> &Self::Target {
        match self {
            VersionAnnotatedConfig::V0_0_0(cfg) => cfg,
            VersionAnnotatedConfig::Unsupported => unreachable!(),
        }
    }
}

impl DerefMut for VersionAnnotatedConfig {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            VersionAnnotatedConfig::V0_0_0(cfg) => cfg,
            VersionAnnotatedConfig::Unsupported => unreachable!(),
        }
    }
}

impl Default for Config!["0.0.0"] {
    fn default() -> Self {
        Self {
            provider_parameters: Default::default(),
            drg_pak_path: DRGInstallation::find()
                .as_ref()
                .map(DRGInstallation::main_pak),
            local_mod_folder: None,
            gui_theme: Some(GuiTheme::Dark),
            sorting_config: None,
            confirm_deletion: true,
            show_changelog: true,
            bundled_crash_reporter: true,
        }
    }
}

impl From<&VersionAnnotatedConfig> for MetaConfig {
    fn from(value: &VersionAnnotatedConfig) -> Self {
        MetaConfig {
            bundled_crash_reporter: value.bundled_crash_reporter,
        }
    }
}

#[derive(Debug, Snafu)]
pub enum StateError {
    #[snafu(display("failed to deserialize user config"))]
    CfgDeserializationFailed { source: serde_json::Error },
    #[snafu(display("unsupported config version"))]
    UnsupportedCfgVersion,
    #[snafu(display("failed to read config.json"))]
    CfgReadFailed { source: std::io::Error },
    #[snafu(display("failed to save config"))]
    CfgSaveFailed { source: std::io::Error },
    #[snafu(display("failed to serialize user config"))]
    CfgSerializationFailed { source: serde_json::Error },
    #[snafu(transparent)]
    IoError { source: std::io::Error },
    #[snafu(transparent)]
    PersistError { source: tempfile::PersistError },
    #[snafu(transparent)]
    ProviderError { source: ProviderError },
    #[snafu(display("failed to deserialize mod data"))]
    ModDataDeserializationFailed { source: serde_json::Error },
    #[snafu(display("unsupported mod data version: {version}"))]
    UnsupportedModDataVersion { version: String },
    #[snafu(display("invalid mod data: {reason}"))]
    InvalidModData { reason: String },
    #[snafu(display("failed to deserialize legacy profiles"))]
    LegacyProfilesDeserializationFailed { source: serde_json::Error },
    #[snafu(display("invalid bundled mod.io preset: {reason}"))]
    InvalidBundledPreset { reason: String },
}

pub struct State {
    pub dirs: Dirs,
    pub config: ConfigWrapper<VersionAnnotatedConfig>,
    pub mod_data: ConfigWrapper<VersionAnnotatedModData>,
    pub store: Arc<ModStore>,
}

impl State {
    /// Known version conflicts, including disabled profile entries.
    pub fn profile_version_warnings(&self, profile: &str) -> Vec<String> {
        crate::mod_identity::ProfileIssues::collect(&self.mod_data, profile, &self.store).versions
    }

    pub fn init(dirs: Dirs) -> Result<Self, StateError> {
        let config_path = dirs.config_dir.join("config.json");

        let config = read_config_or_default(&config_path)?;
        let config = ConfigWrapper::<VersionAnnotatedConfig>::new(&config_path, config);
        config.save().unwrap();

        let legacy_mod_profiles_path = dirs.config_dir.join("profiles.json");
        let mod_data_path = dirs.config_dir.join("mod_data.json");
        // Apply embedded defaults only for a genuinely new mod list.
        let fresh = !mod_data_path.exists() && !legacy_mod_profiles_path.exists();
        let mut mod_data = read_mod_data_or_default(&mod_data_path, legacy_mod_profiles_path)?;
        if fresh {
            match &mut mod_data {
                VersionAnnotatedModData::V0_1_0(data) | VersionAnnotatedModData::V0_3_0(data) => {
                    crate::preinstall::apply(data)
                        .map_err(|reason| StateError::InvalidBundledPreset { reason })?;
                }
                VersionAnnotatedModData::V0_0_0(_) => unreachable!("mod data should be upgraded"),
            }
        }
        if fresh {
            config::write_atomic(&mod_data_path, &mod_data)?;
        }
        let mod_data = ConfigWrapper::<VersionAnnotatedModData>::new(mod_data_path, mod_data);
        // The loader commits migrations before creating an auto-saving wrapper.

        let store = ModStore::new(&dirs.cache_dir, &config.provider_parameters)?.into();

        Ok(Self {
            dirs,
            config,
            mod_data,
            store,
        })
    }
}

fn read_config_or_default(config_path: &PathBuf) -> Result<VersionAnnotatedConfig, StateError> {
    Ok(match fs::read(config_path) {
        Ok(buf) => {
            let config = serde_json::from_slice::<MaybeVersionedConfig>(&buf)
                .context(CfgDeserializationFailedSnafu)?;
            match config {
                MaybeVersionedConfig::Versioned(v) => match v {
                    VersionAnnotatedConfig::V0_0_0(v) => VersionAnnotatedConfig::V0_0_0(v),
                    VersionAnnotatedConfig::Unsupported => UnsupportedCfgVersionSnafu.fail()?,
                },
                MaybeVersionedConfig::Legacy(legacy) => {
                    VersionAnnotatedConfig::V0_0_0(Config_v0_0_0 {
                        provider_parameters: legacy.provider_parameters,
                        drg_pak_path: legacy.drg_pak_path,
                        ..Default::default()
                    })
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => VersionAnnotatedConfig::default(),
        Err(e) => Err(e)?,
    })
}

fn read_mod_data_or_default(
    mod_data_path: &std::path::Path,
    legacy_mod_profiles_path: PathBuf,
) -> Result<VersionAnnotatedModData, StateError> {
    migration::load(mod_data_path, &legacy_mod_profiles_path)
}

impl Default for ModGroup {
    fn default() -> Self {
        Self {
            tint_color: None,
            id: new_id(),
            description: None,
            mods: Vec::new(),
        }
    }
}
// ponytail: obake's versioned structs can't take #[derive(Default)] (it conflicts across versions).
#[allow(clippy::derivable_impls)]
impl Default for ModProfile_v0_0_0 {
    fn default() -> Self {
        Self { mods: Vec::new() }
    }
}
impl Default for ModProfile_v0_1_0 {
    fn default() -> Self {
        Self {
            id: new_id(),
            mods: Vec::new(),
        }
    }
}
impl ModOrGroup {
    pub fn id(&self) -> &str {
        match self {
            Self::Group { id, .. } => id,
            Self::Individual(member) => &member.id,
        }
    }
}
impl ModProfile_v0_1_0 {
    /// Copy occurrences, but keep references pointing at the same shared groups.
    pub fn duplicate(&self) -> Self {
        let mut copy = self.clone();
        copy.id = new_id();
        for row in &mut copy.mods {
            match row {
                ModOrGroup::Group { id, .. } => *id = new_id(),
                ModOrGroup::Individual(member) => {
                    member.id = new_id();
                    member.member_id = None;
                }
            }
        }
        copy
    }
}

impl ModConfig {
    pub fn duplicate(&self) -> Self {
        let mut copy = self.clone();
        copy.id = new_id();
        copy.member_id = self.member_id.as_ref().map(|_| new_id());
        copy
    }
}

#[cfg(test)]
mod mod_data_tests {
    use super::{
        ModConfig, ModData_v0_1_0 as ModData, ModGroup, ModOrGroup, ModProfile_v0_1_0 as ModProfile,
    };
    use crate::providers::ModSpecification;

    #[test]
    fn enabling_legacy_group_dependency_is_profile_local() {
        let mut data: ModData = serde_json::from_value(serde_json::json!({
            "active_profile": "A",
            "profiles": {
                "A": {"mods": [{"group_name": "tools", "enabled": false}]},
                "B": {"mods": [{"group_name": "tools", "enabled": true}]}
            },
            "groups": {"tools": {"mods": [
                {"spec": {"url": "a"}, "required": false, "enabled": false},
                {"spec": {"url": "b"}, "required": false, "enabled": false}
            ]}}
        }))
        .unwrap();

        assert!(data.enable_dependency("A", &ModSpecification::new("a".into())));
        data.groups.get_mut("tools").unwrap().mods.swap(0, 1);
        let data: ModData = serde_json::from_value(serde_json::to_value(data).unwrap()).unwrap();
        for profile in ["A", "B"] {
            let mut mods = Vec::new();
            data.for_each_enabled_mod(profile, |mc| mods.push(mc.spec.url.clone()));
            if profile == "A" {
                assert_eq!(mods, ["a"]);
            } else {
                assert!(mods.is_empty());
            }
        }
    }

    #[test]
    fn test_for_each_mod() {
        let mod_1 = ModConfig {
            id: crate::state::new_id(),
            description: None,
            member_id: None,
            spec: ModSpecification::new("a".to_string()),
            required: false,
            enabled: false,
            priority: 50,
        };

        let mod_2 = ModConfig {
            id: crate::state::new_id(),
            description: None,
            member_id: None,
            spec: ModSpecification::new("b".to_string()),
            required: true,
            enabled: false,
            priority: 50,
        };

        let mod_3 = ModConfig {
            id: crate::state::new_id(),
            description: None,
            member_id: None,
            spec: ModSpecification::new("c".to_string()),
            required: false,
            enabled: true,
            priority: 50,
        };

        let mod_data = ModData {
            bitfix_scripts: Vec::new(),
            active_profile: "default".to_string(),
            profiles: [(
                "default".to_string(),
                ModProfile {
                    id: crate::state::new_id(),
                    mods: vec![
                        ModOrGroup::Individual(mod_1),
                        ModOrGroup::Group {
                            id: crate::state::new_id(),
                            member_overrides: Default::default(),
                            group_name: "mg1".to_string(),
                            enabled: false,
                        },
                    ],
                },
            )]
            .into(),
            groups: [(
                "mg1".to_string(),
                ModGroup {
                    tint_color: None,
                    id: crate::state::new_id(),
                    description: None,
                    mods: vec![mod_2, mod_3],
                },
            )]
            .into(),
        };

        let mut counter = 0;
        mod_data.for_each_mod("default", |_| {
            counter += 1;
        });
        assert_eq!(counter, 3);
    }

    #[test]
    fn test_for_each_enabled_mod() {
        let mod_1 = ModConfig {
            id: crate::state::new_id(),
            description: None,
            member_id: None,
            spec: ModSpecification::new("a".to_string()),
            required: false,
            enabled: false,
            priority: 50,
        };

        let mod_2 = ModConfig {
            id: crate::state::new_id(),
            description: None,
            member_id: None,
            spec: ModSpecification::new("b".to_string()),
            required: true,
            enabled: false,
            priority: 50,
        };

        let mod_3 = ModConfig {
            id: crate::state::new_id(),
            description: None,
            member_id: None,
            spec: ModSpecification::new("c".to_string()),
            required: false,
            enabled: true,
            priority: 50,
        };

        let mod_data = ModData {
            bitfix_scripts: Vec::new(),
            active_profile: "default".to_string(),
            profiles: [(
                "default".to_string(),
                ModProfile {
                    id: crate::state::new_id(),
                    mods: vec![
                        ModOrGroup::Individual(mod_1),
                        ModOrGroup::Group {
                            id: crate::state::new_id(),
                            member_overrides: Default::default(),
                            group_name: "mg1".to_string(),
                            enabled: true,
                        },
                    ],
                },
            )]
            .into(),
            groups: [(
                "mg1".to_string(),
                ModGroup {
                    tint_color: None,
                    id: crate::state::new_id(),
                    description: None,
                    mods: vec![mod_2, mod_3],
                },
            )]
            .into(),
        };

        let mut counter = 0;
        mod_data.for_each_enabled_mod("default", |_| {
            counter += 1;
        });
        assert_eq!(counter, 1);
    }

    #[test]
    fn test_any_mod() {
        let mod_1 = ModConfig {
            id: crate::state::new_id(),
            description: None,
            member_id: None,
            spec: ModSpecification::new("a".to_string()),
            required: false,
            enabled: false,
            priority: 50,
        };

        let mod_2 = ModConfig {
            id: crate::state::new_id(),
            description: None,
            member_id: None,
            spec: ModSpecification::new("b".to_string()),
            required: true,
            enabled: false,
            priority: 50,
        };

        let mod_3 = ModConfig {
            id: crate::state::new_id(),
            description: None,
            member_id: None,
            spec: ModSpecification::new("c".to_string()),
            required: false,
            enabled: true,
            priority: 50,
        };

        let mod_data = ModData {
            bitfix_scripts: Vec::new(),
            active_profile: "default".to_string(),
            profiles: [(
                "default".to_string(),
                ModProfile {
                    id: crate::state::new_id(),
                    mods: vec![
                        ModOrGroup::Individual(mod_1),
                        ModOrGroup::Group {
                            id: crate::state::new_id(),
                            member_overrides: Default::default(),
                            group_name: "mg1".to_string(),
                            enabled: true,
                        },
                    ],
                },
            )]
            .into(),
            groups: [(
                "mg1".to_string(),
                ModGroup {
                    tint_color: None,
                    id: crate::state::new_id(),
                    description: None,
                    mods: vec![mod_2, mod_3],
                },
            )]
            .into(),
        };

        let any_required = mod_data.any_mod("default", |mc, _| mc.required);
        assert!(any_required);
    }
    #[test]
    fn global_bitfix_is_identical_in_every_profile() {
        let mut data = ModData::default();
        data.profiles.insert("other".into(), ModProfile::default());
        data.bitfix_scripts.push(ModConfig {
            id: crate::state::new_id(),
            description: None,
            member_id: None,
            spec: ModSpecification::new("C:\\mods\\test.lua".into()),
            required: false,
            enabled: true,
            priority: 0,
        });
        assert_eq!(
            data.ordered_install_specs("default"),
            data.ordered_install_specs("other")
        );
        assert_eq!(data.ordered_install_specs("other").len(), 1);
        data.bitfix_scripts[0].enabled = false;
        assert!(data.ordered_install_specs("default").is_empty());
        assert!(data.ordered_install_specs("other").is_empty());
    }
}

#[cfg(test)]
mod group_tint_serialization_tests {
    use super::ModGroup;

    #[test]
    fn chosen_tint_survives_serialization_and_legacy_groups_default_to_none() {
        let mut group = ModGroup::default();
        group.tint_color = Some("red".to_owned());
        let value = serde_json::to_value(&group).unwrap();
        assert_eq!(
            value.get("tint_color").and_then(|v| v.as_str()),
            Some("red")
        );
        let decoded: ModGroup = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(decoded.tint_color.as_deref(), Some("red"));

        let mut legacy = value;
        legacy.as_object_mut().unwrap().remove("tint_color");
        let decoded: ModGroup = serde_json::from_value(legacy).unwrap();
        assert!(decoded.tint_color.is_none());
    }
}
