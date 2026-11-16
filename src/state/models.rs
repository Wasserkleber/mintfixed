use super::*;

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
pub struct MemberOverrides(pub(super) BTreeMap<String, bool>);

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

impl ModConfig {
    pub fn duplicate(&self) -> Self {
        let mut copy = self.clone();
        copy.id = new_id();
        copy.member_id = self.member_id.as_ref().map(|_| new_id());
        copy
    }
}
