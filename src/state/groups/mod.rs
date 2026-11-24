pub(crate) mod palette;

use super::*;

impl MemberOverrides {
    pub(super) fn is_empty(&self) -> bool {
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
#[cfg(test)]
mod group_tint_serialization_tests {
    use super::ModGroup;

    #[test]
    fn chosen_tint_survives_serialization_and_legacy_groups_default_to_none() {
        let group = ModGroup {
            tint_color: Some("red".to_owned()),
            ..Default::default()
        };
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
