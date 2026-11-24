use super::*;

/// Remove a member by its stable entry ID, cleaning up overrides in every
/// profile that references this shared group. Never delete by a stale row index.
pub(in crate::gui) fn remove_group_member(
    data: &mut Data,
    group_name: &str,
    mod_id: &str,
) -> Result<(), &'static str> {
    let group = data
        .groups
        .get_mut(group_name)
        .ok_or("Group no longer exists")?;
    let index = group
        .mods
        .iter()
        .position(|mc| mc.id == mod_id)
        .ok_or("Mod is no longer in the group")?;
    let removed = group.mods.remove(index);
    if let Some(member_id) = removed.member_id {
        for entry in data
            .profiles
            .values_mut()
            .flat_map(|profile| &mut profile.mods)
        {
            if let ModOrGroup::Group {
                group_name: entry_group,
                member_overrides,
                ..
            } = entry
                && entry_group == group_name
            {
                member_overrides.forget(&member_id);
            }
        }
    }
    Ok(())
}

/// Renaming changes the shared collection's label, never its contents or order.
pub(in crate::gui::group_controls) fn rename_group(
    data: &mut Data,
    old: &str,
    requested: &str,
) -> Result<bool, &'static str> {
    let name = requested.trim();
    if name.is_empty() {
        return Err("Enter a group name");
    }
    if !data.groups.contains_key(old) {
        return Err("That group no longer exists");
    }
    if name == old {
        return Ok(false);
    }
    if data.groups.contains_key(name) {
        return Err("A group with that name already exists");
    }
    let group = data.groups.remove(old).unwrap();
    data.groups.insert(name.to_owned(), group);
    for profile in data.profiles.values_mut() {
        for item in &mut profile.mods {
            if let ModOrGroup::Group { group_name, .. } = item
                && group_name == old
            {
                *group_name = name.to_owned();
            }
        }
    }
    Ok(true)
}

/// Delete the shared collection and detach it from every profile.
/// Standalone mods and other groups are untouched.
pub(super) fn delete_group(data: &mut Data, name: &str) -> Result<(), &'static str> {
    if data.groups.remove(name).is_none() {
        return Err("That group no longer exists");
    }
    for profile in data.profiles.values_mut() {
        profile.mods.retain(
            |item| !matches!(item, ModOrGroup::Group { group_name, .. } if group_name == name),
        );
    }
    Ok(())
}

/// Create a separate shared group with the same members and their settings.
/// Only the active profile gets a reference to the new group.
pub(super) fn duplicate_group(
    data: &mut Data,
    source: &str,
    requested: &str,
) -> Result<String, &'static str> {
    let name = requested.trim();
    if name.is_empty() {
        return Err("Enter a name for the duplicate group");
    }
    if data.groups.contains_key(name) {
        return Err("A group with that name already exists");
    }
    let original = data
        .groups
        .get(source)
        .ok_or("That group no longer exists")?;
    let mut copy = original.clone();
    copy.id = crate::state::new_id();
    copy.mods = original.mods.iter().map(ModConfig::duplicate).collect();

    // Keep this profile's switches when the source group is attached here.
    let reference = data.profiles[&data.active_profile]
        .mods
        .iter()
        .find_map(|item| {
            if let ModOrGroup::Group {
                group_name,
                enabled,
                member_overrides,
                ..
            } = item
                && group_name == source
            {
                Some(ModOrGroup::Group {
                    id: crate::state::new_id(),
                    group_name: name.to_owned(),
                    enabled: *enabled,
                    member_overrides: {
                        let mut overrides = crate::state::MemberOverrides::default();
                        for (old, new) in original.mods.iter().zip(&mut copy.mods) {
                            overrides.set(new, member_overrides.enabled(old));
                        }
                        overrides
                    },
                })
            } else {
                None
            }
        })
        .unwrap_or(ModOrGroup::Group {
            id: crate::state::new_id(),
            group_name: name.to_owned(),
            enabled: true,
            member_overrides: Default::default(),
        });
    data.groups.insert(name.to_owned(), copy);
    data.profiles
        .get_mut(&data.active_profile)
        .unwrap()
        .mods
        .insert(0, reference);
    Ok(name.to_owned())
}
