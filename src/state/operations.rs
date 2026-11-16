use super::*;

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
        for (row, item) in self.profiles[profile].mods.iter().enumerate() {
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
                    for (index, mc) in self.groups[group_name].mods.iter().enumerate() {
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
        self.visit_effective_mods(profile, |mc, _| f(mc));
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
        self.visit_effective_mods(profile, |mc, source| {
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

/// Recognize `.lua` script paths and URLs without mistaking `.lua.zip` for a patch.
fn is_bitfix_url(url: &str) -> bool {
    url.split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase()
        .ends_with(".lua")
}
