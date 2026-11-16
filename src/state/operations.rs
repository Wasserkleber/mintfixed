use super::*;

/// A profile cannot be safely used for installation when its references are missing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectionError {
    MissingProfile { profile: String },
    MissingGroup { profile: String, group_name: String },
}

impl std::fmt::Display for SelectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingProfile { profile } => write!(f, "profile {profile:?} does not exist"),
            Self::MissingGroup {
                profile,
                group_name,
            } => {
                write!(
                    f,
                    "profile {profile:?} refers to missing group {group_name:?}"
                )
            }
        }
    }
}
impl std::error::Error for SelectionError {}

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

// Source coordinates are the original saved row and group-member indices.
// The model is immutable: never alter a shared group's member when resolving
// one particular profile's effective enabled state.
#[derive(Clone, Copy, Debug)]
enum EffectiveSource<'a> {
    Standalone {
        row: usize,
    },
    Group {
        row: usize,
        index: usize,
        group_name: &'a str,
    },
}

impl ModData_v0_1_0 {
    fn visit_effective_mods(
        &self,
        profile: &str,
        mut f: impl FnMut(&ModConfig, EffectiveSource<'_>),
    ) {
        // Non-installation consumers deliberately tolerate missing references.
        // Installation uses validate_profile_references first and never builds a partial bundle.
        let Some(profile_data) = self.profiles.get(profile) else {
            return;
        };
        for (row, item) in profile_data.mods.iter().enumerate() {
            match item {
                ModOrGroup::Individual(mc) if mc.enabled => {
                    f(mc, EffectiveSource::Standalone { row });
                }
                ModOrGroup::Group {
                    group_name,
                    enabled: true,
                    member_overrides,
                    ..
                } => {
                    if let Some(group) = self.groups.get(group_name) {
                        for (index, mc) in group.mods.iter().enumerate() {
                            if member_overrides.enabled(mc) {
                                f(
                                    mc,
                                    EffectiveSource::Group {
                                        row,
                                        index,
                                        group_name,
                                    },
                                );
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

impl ModData_v0_1_0 {
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
        let Some(profile_data) = self.profiles.get(profile) else {
            return;
        };
        for mod_or_group in &profile_data.mods {
            match mod_or_group {
                ModOrGroup::Group {
                    group_name,
                    enabled,
                    ..
                } => {
                    if g(*enabled) {
                        if let Some(group) = self.groups.get(group_name) {
                            for mc in &group.mods {
                                if p(mc) {
                                    f(mc);
                                }
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
        let Some(profile_data) = self.profiles.get_mut(profile) else {
            return;
        };
        for mod_or_group in &mut profile_data.mods {
            match mod_or_group {
                ModOrGroup::Group {
                    group_name,
                    enabled,
                    ..
                } => {
                    if g(*enabled) {
                        if let Some(group) = self.groups.get_mut(group_name) {
                            for mc in &mut group.mods {
                                if p(mc) {
                                    f(mc);
                                }
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
        self.visit_effective_mods(profile, |mc, _| f(mc));
    }

    /// Validate the whole requested profile before building a bundle.
    /// A missing group, including a disabled one, is treated as broken data.
    pub fn validate_profile_references(&self, profile: &str) -> Result<(), SelectionError> {
        let selected =
            self.profiles
                .get(profile)
                .ok_or_else(|| SelectionError::MissingProfile {
                    profile: profile.to_owned(),
                })?;
        for item in &selected.mods {
            if let ModOrGroup::Group { group_name, .. } = item {
                if !self.groups.contains_key(group_name) {
                    return Err(SelectionError::MissingGroup {
                        profile: profile.to_owned(),
                        group_name: group_name.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Fallible entry point for CLI and other installation callers.
    pub fn try_ordered_install_entries(
        &self,
        profile: &str,
    ) -> Result<Vec<InstallEntry>, SelectionError> {
        self.validate_profile_references(profile)?;
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
        self.visit_effective_mods(profile, |mc, source| {
            // Lua in a regular profile/group is not a Bitfix script.
            if is_bitfix_url(&mc.spec.url) {
                return;
            }
            let location = match source {
                EffectiveSource::Standalone { row } => {
                    format!("profile {profile:?}, standalone row {}", row + 1)
                }
                EffectiveSource::Group {
                    row,
                    index,
                    group_name,
                } => {
                    format!(
                        "profile {profile:?}, group {group_name:?} (row {}), mod #{}",
                        row + 1,
                        index + 1
                    )
                }
            };
            entries.push((
                mc.priority,
                InstallEntry {
                    spec: mc.spec.clone(),
                    location,
                },
            ));
        });
        // Stable sort keeps saved occurrence order for equal priorities.
        entries.sort_by_key(|(priority, _)| std::cmp::Reverse(*priority));
        Ok(entries.into_iter().map(|(_, entry)| entry).collect())
    }

    /// Compatibility API: never panic or return a partially resolved selection.
    /// Install entry points should prefer `try_ordered_install_entries` to surface errors.
    pub fn ordered_install_entries(&self, profile: &str) -> Vec<InstallEntry> {
        match self.try_ordered_install_entries(profile) {
            Ok(entries) => entries,
            Err(error) => {
                tracing::error!(%error, "cannot resolve installation selection");
                eprintln!("mintfixed: cannot resolve installation selection: {error}");
                Vec::new()
            }
        }
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
        let Some(profile_data) = self.profiles.get(profile) else {
            return false;
        };
        profile_data.mods.iter().any(|m| match m {
            ModOrGroup::Individual(mc) => f(mc, None),
            ModOrGroup::Group {
                group_name,
                enabled,
                ..
            } => self
                .groups
                .get(group_name)
                .is_some_and(|group| group.mods.iter().any(|mc| f(mc, Some(*enabled)))),
        })
    }

    pub fn any_mod_mut<
        F: FnMut(&mut ModConfig, Option<&mut bool> /* mod group enabled? */) -> bool,
    >(
        &mut self,
        profile: &str,
        mut f: F,
    ) -> bool {
        let Some(profile_data) = self.profiles.get_mut(profile) else {
            return false;
        };
        profile_data.mods.iter_mut().any(|m| match m {
            ModOrGroup::Individual(mc) => f(mc, None),
            ModOrGroup::Group {
                group_name,
                enabled,
                ..
            } => self
                .groups
                .get_mut(group_name)
                .is_some_and(|group| group.mods.iter_mut().any(|mc| f(mc, Some(enabled)))),
        })
    }

    pub fn enable_dependency(&mut self, profile: &str, spec: &ModSpecification) -> bool {
        let Some(profile_data) = self.profiles.get_mut(profile) else {
            return false;
        };
        for item in &mut profile_data.mods {
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
                    let Some(group) = self.groups.get_mut(group_name) else {
                        continue;
                    };
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

/// Recognize `.lua` script paths and URLs without mistaking `.lua.zip` for a patch.
fn is_bitfix_url(url: &str) -> bool {
    url.split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase()
        .ends_with(".lua")
}
