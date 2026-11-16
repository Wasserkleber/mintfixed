use super::*;

impl ModData_v0_1_0 {
    pub fn get_active_profile(&self) -> &ModProfile_v0_1_0 {
        &self.profiles[&self.active_profile]
    }

    pub fn get_active_profile_mut(&mut self) -> &mut ModProfile_v0_1_0 {
        self.profiles.get_mut(&self.active_profile).unwrap()
    }

    pub fn remove_active_profile(&mut self) {
        self.profiles.remove(&self.active_profile);
        self.active_profile = self.profiles.keys().next().unwrap().to_string();
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
