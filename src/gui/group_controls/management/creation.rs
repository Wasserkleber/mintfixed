use super::*;

pub(super) fn show_creation_and_attachment(
    ui: &mut Ui,
    data: &mut Data,
    name: &mut String,
    changed: &mut bool,
    success_message: &mut Option<String>,
) {
    let profile = data.profiles.get_mut(&data.active_profile).unwrap();
    ui.label("New group");
    ui.text_edit_singleline(name);
    let trimmed = name.trim();
    if ui
        .add_enabled(
            !trimmed.is_empty() && !data.groups.contains_key(trimmed),
            crate::gui::mint_text_button("Create group"),
        )
        .clicked()
    {
        let created = trimmed.to_owned();
        data.groups.insert(created.clone(), Default::default());
        attach(profile, &created);
        name.clear();
        *changed = true;
        *success_message = Some(format!("Created group \"{created}\"."));
        ui.memory_mut(|mem| mem.close_popup());
    }
    ui.separator();
    let eligible: Vec<_> = data
        .groups
        .keys()
        .filter(|key| {
            !profile.mods.iter().any(|item| {
        matches!(item, ModOrGroup::Group { group_name, .. } if group_name == *key)
    })
        })
        .collect();
    if eligible.is_empty() {
        ui.add_enabled(false, crate::gui::mint_text_button("Add existing group"));
        ui.label("No other groups available for this profile.");
    } else {
        let attach_section = egui::CollapsingHeader::new("Add existing group")
            .id_salt(("attach-existing", &data.active_profile))
            .show(ui, |ui| {
                // Keep the expanded list compact; additional groups remain scrollable.
                group_picker_scroll().show(ui, |ui| {
                    for key in eligible {
                        if group_option_row(
                            ui,
                            key.as_str(),
                            false,
                            data.groups
                                .get(key.as_str())
                                .and_then(|group| group.tint_color.as_deref()),
                        )
                        .clicked()
                        {
                            attach(profile, key);
                            *changed = true;
                            *success_message = Some(format!(
                                "Added existing group \"{key}\" to the active profile."
                            ));
                            ui.memory_mut(|mem| mem.close_popup());
                            break;
                        }
                    }
                });
            });
        crate::gui::management_popup::scroll_expanded_header(
            &attach_section.header_response,
            attach_section.body_response.is_some(),
        );
    }
}
