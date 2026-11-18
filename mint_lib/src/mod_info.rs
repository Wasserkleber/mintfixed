use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// Tags from mod.io.
#[derive(Debug, Clone)]
pub struct ModioTags {
    pub qol: bool,
    pub gameplay: bool,
    pub audio: bool,
    pub visual: bool,
    pub framework: bool,
    pub versions: BTreeSet<String>,
    pub required_status: RequiredStatus,
    pub approval_status: ApprovalStatus,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum RequiredStatus {
    RequiredByAll,
    Optional,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ApprovalStatus {
    Verified,
    Approved,
    Sandbox,
}

/// Whether a mod can be resolved by clients or not
#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub enum ResolvableStatus {
    Unresolvable(String),
    Resolvable,
}

/// Returned from ModStore
#[derive(Debug, Clone)]
pub struct ModInfo {
    pub provider: &'static str,
    pub name: String,
    pub spec: ModSpecification,          // unpinned version
    pub versions: Vec<ModSpecification>, // pinned versions TODO make this a different type
    pub resolution: ModResolution,
    pub suggested_require: bool,
    pub suggested_dependencies: Vec<ModSpecification>, // ModResponse
    pub modio_tags: Option<ModioTags>,                 // only available for mods from mod.io
    pub modio_id: Option<u32>,                         // only available for mods from mod.io
}

/// Returned from ModProvider
#[derive(Debug, Clone)]
pub enum ModResponse {
    Redirect(ModSpecification),
    Resolve(ModInfo),
}

/// Points to a mod, optionally a specific version
#[derive(
    Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct ModSpecification {
    pub url: String,
}

impl ModSpecification {
    pub fn new(url: String) -> Self {
        Self { url }
    }
    pub fn replacement_rule(&self) -> Option<&'static crate::mod_replacements::ReplacementRule> {
        crate::mod_replacements::rule_for_original(self)
    }
    pub fn satisfies_dependency(&self, other: &ModSpecification) -> bool {
        self.matches_identity(other) || crate::mod_replacements::satisfies_alternative(self, other)
    }
    /// Match the same mod (including pinned/unpinned versions), not an alternative.
    pub fn matches_identity(&self, other: &ModSpecification) -> bool {
        if self.url == other.url {
            return true;
        }
        // Pins and latest URLs of the same mod.io mod satisfy its identity.
        // Never confuse URL prefixes (e.g. /foo vs /foobar or .pak vs .pak2).
        match (
            modio_dependency_identity(&self.url),
            modio_dependency_identity(&other.url),
        ) {
            (Some((name_a, id_a)), Some((name_b, id_b))) => match (id_a, id_b) {
                (Some(a), Some(b)) => a == b,
                _ => name_a == name_b,
            },
            _ => false,
        }
    }
}

// This intentionally avoids a mod.io provider dependency in mint_lib.
// Accept only the DRG mod.io URL grammar used by the provider. A numeric mod ID
// wins over a renamed slug when both sides include it.
fn modio_dependency_identity(url: &str) -> Option<(&str, Option<u32>)> {
    let remainder = url.strip_prefix("https://mod.io/g/drg/m/")?;
    let (path, fragment) = match remainder.split_once('#') {
        Some((path, fragment)) => (path, Some(fragment)),
        None => (remainder, None),
    };
    let slug = path.strip_suffix('/').unwrap_or(path);
    if slug.is_empty() || slug.contains('/') || slug.contains('?') {
        return None;
    }
    let id = match fragment {
        None => None,
        Some(fragment)
            if !fragment.is_empty() && fragment.bytes().all(|b| b.is_ascii_lowercase()) =>
        {
            None
        }
        Some(fragment) => {
            let (id, file) = match fragment.split_once('/') {
                Some((id, file)) => (id, Some(file)),
                None => (fragment, None),
            };
            if id.is_empty()
                || !id.bytes().all(|b| b.is_ascii_digit())
                || file.is_some_and(|file| {
                    file.is_empty() || !file.bytes().all(|b| b.is_ascii_digit())
                })
            {
                return None;
            }
            Some(id.parse::<u32>().ok()?)
        }
    };
    Some((slug, id))
}

/// Points to a specific version of a specific mod
#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct ModResolution {
    pub url: ModIdentifier,
    pub status: ResolvableStatus,
}

impl ModResolution {
    pub fn resolvable(url: ModIdentifier) -> Self {
        Self {
            url,
            status: ResolvableStatus::Resolvable,
        }
    }
    pub fn unresolvable(url: ModIdentifier, name: String) -> Self {
        Self {
            url,
            status: ResolvableStatus::Unresolvable(name),
        }
    }
    /// Used to get the URL if resolvable or just return the mod name if not
    pub fn get_resolvable_url_or_name(&self) -> &str {
        match &self.status {
            ResolvableStatus::Resolvable => &self.url.0,
            ResolvableStatus::Unresolvable(name) => name,
        }
    }
}

/// Mod identifier used for tracking gameplay affecting status.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ModIdentifier(pub String);

impl ModIdentifier {
    pub fn new(s: String) -> Self {
        Self(s)
    }
}
impl From<String> for ModIdentifier {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}
impl From<&str> for ModIdentifier {
    fn from(value: &str) -> Self {
        Self::new(value.to_owned())
    }
}

/// Stripped down mod info stored in the mod pak to be used in game
#[derive(Debug, Serialize, Deserialize)]
pub struct Meta {
    pub version: String,
    pub mods: Vec<MetaMod>,
    pub config: MetaConfig,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct MetaConfig {
    /// Install and autolaunch the bundled BP-Crash-Reporter. Read by integrate only, so it is
    /// not serialized: the hook learns it from the `mintfixed-bpcrash/disabled` marker instead.
    #[serde(skip)]
    pub bundled_crash_reporter: bool,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct MetaMod {
    pub name: String,
    pub version: String,
    pub url: String,
    pub author: String,
    pub approval: ApprovalStatus,
    pub required: bool,
}
impl Meta {
    pub fn to_server_list_string(&self) -> String {
        use itertools::Itertools;

        ["mint".into(), self.version.to_string()]
            .into_iter()
            .chain(
                self.mods
                    .iter()
                    .sorted_by_key(|m| (std::cmp::Reverse(m.approval), &m.name))
                    .take(100)
                    .flat_map(|m| {
                        [
                            match m.approval {
                                ApprovalStatus::Verified => 'V',
                                ApprovalStatus::Approved => 'A',
                                ApprovalStatus::Sandbox => 'S',
                            }
                            .into(),
                            m.name.replace(';', ""),
                        ]
                    }),
            )
            .join(";")
    }
}

#[cfg(test)]
mod dependency_identity_tests {
    use super::ModSpecification;

    fn matches(a: &str, b: &str) -> bool {
        ModSpecification::new(a.to_owned())
            .satisfies_dependency(&ModSpecification::new(b.to_owned()))
    }

    #[test]
    fn pinned_and_unpinned_modio_specs_match() {
        assert!(matches(
            "https://mod.io/g/drg/m/example#123/456",
            "https://mod.io/g/drg/m/example#123"
        ));
        assert!(matches(
            "https://mod.io/g/drg/m/example",
            "https://mod.io/g/drg/m/example#123/456"
        ));
        assert!(matches(
            "https://mod.io/g/drg/m/renamed#123/456",
            "https://mod.io/g/drg/m/old#123"
        ));
    }

    #[test]
    fn prefixes_and_different_mod_ids_never_match() {
        assert!(!matches(
            "https://mod.io/g/drg/m/example",
            "https://mod.io/g/drg/m/example-plus"
        ));
        assert!(!matches(
            "https://mod.io/g/drg/m/example#123",
            "https://mod.io/g/drg/m/example#124"
        ));
        assert!(!matches(
            "https://example.test/mod.pak",
            "https://example.test/mod.pak2"
        ));
    }

    #[test]
    fn invalid_modio_fragments_do_not_match_other_versions() {
        assert!(!matches(
            "https://mod.io/g/drg/m/example#123/not-a-file",
            "https://mod.io/g/drg/m/example#123"
        ));
    }
}
