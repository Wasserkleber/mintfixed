use crate::state::{ModData_v0_1_0 as ModData, ModProfile_v0_1_0 as ModProfile};

pub trait NamedEntries<E> {
    fn len(&self) -> usize;
    fn contains(&self, name: &str) -> bool;
    fn select(&mut self, name: String);
    fn selected_name(&self) -> &str;
    fn add_new(&mut self, name: &str);
    fn remove_selected(&mut self);
    fn rename_named(&mut self, source: &str, new_name: String);
    fn duplicate_named(&mut self, source: &str, new_name: String);
    fn entries<'s>(&'s mut self) -> Box<dyn Iterator<Item = (&'s String, &'s E)> + 's>;
}

impl NamedEntries<ModProfile> for ModData {
    fn len(&self) -> usize {
        self.profiles.len()
    }
    fn contains(&self, name: &str) -> bool {
        self.profiles.contains_key(name)
    }
    fn select(&mut self, name: String) {
        self.active_profile = name;
    }
    fn selected_name(&self) -> &str {
        &self.active_profile
    }
    fn add_new(&mut self, name: &str) {
        self.profiles.insert(name.to_owned(), Default::default());
        self.active_profile = name.to_string();
    }
    fn remove_selected(&mut self) {
        self.remove_active_profile();
    }
    fn rename_named(&mut self, source: &str, new_name: String) {
        let profile = self.profiles.remove(source).expect("existing profile");
        self.profiles.insert(new_name.clone(), profile);
        if self.active_profile == source {
            self.active_profile = new_name;
        }
    }
    fn duplicate_named(&mut self, source: &str, new_name: String) {
        let profile = self
            .profiles
            .get(source)
            .expect("existing profile")
            .duplicate();
        self.profiles.insert(new_name, profile);
        // Managing another profile must not silently switch the active profile.
    }
    fn entries<'s>(&'s mut self) -> Box<dyn Iterator<Item = (&'s String, &'s ModProfile)> + 's> {
        Box::new(self.profiles.iter())
    }
}

/// Render and return whether any changes were made
pub(crate) fn ui<E, N>(
    ui: &mut egui::Ui,
    name: &str,
    entries: &mut N,
    additional_ui: Option<impl FnOnce(&mut egui::Ui, &mut N)>,
    after_rename: Option<impl FnOnce(&mut egui::Ui, &mut N, &mut bool)>,
    copy_ui: Option<impl FnOnce(&mut egui::Ui, &mut N) -> Option<Result<String, String>>>,
    allow_edit: bool,
) -> bool
where
    N: NamedEntries<E>,
{
    let mut modified = false;
    ui.push_id(name, |ui| {
        ui.horizontal(|ui| {
            ui.add_enabled_ui(allow_edit, |ui| {
                super::profile_management::ui(
                    ui,
                    name,
                    entries,
                    &mut modified,
                    after_rename,
                    copy_ui,
                );
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                if let Some(additional_ui) = additional_ui {
                    additional_ui(ui, entries);
                }

                ui.with_layout(ui.layout().with_main_justify(true), |ui| {
                    mk_dropdown(ui, name, entries, &mut modified);
                });
            });
        });
    });
    modified
}

fn mk_dropdown<E, N>(ui: &mut egui::Ui, name: &str, entries: &mut N, modified: &mut bool)
where
    N: NamedEntries<E>,
{
    let mut selected = entries.selected_name().to_owned();

    egui::ComboBox::from_id_salt(format!("dropdown-{name}"))
        .width(ui.available_width())
        .selected_text(selected.clone())
        .show_ui(ui, |ui| {
            entries.entries().for_each(|(k, _)| {
                ui.selectable_value(&mut selected, k.to_owned(), k);
            })
        });

    if selected != entries.selected_name() {
        entries.select(selected);
        *modified = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_inactive_keeps_active_and_profile_id() {
        let mut data = ModData::default();
        data.profiles.insert("other".to_owned(), Default::default());
        let original_id = data.profiles["other"].id.clone();
        data.rename_named("other", "renamed".to_owned());
        assert_eq!(data.active_profile, "default");
        assert!(!data.profiles.contains_key("other"));
        assert_eq!(data.profiles["renamed"].id, original_id);
    }

    #[test]
    fn rename_active_updates_selection() {
        let mut data = ModData::default();
        data.rename_named("default", "renamed".to_owned());
        assert_eq!(data.active_profile, "renamed");
        assert!(data.profiles.contains_key("renamed"));
    }

    #[test]
    fn duplicate_inactive_keeps_active_and_creates_new_id() {
        let mut data = ModData::default();
        data.profiles.insert("other".to_owned(), Default::default());
        data.duplicate_named("other", "copy".to_owned());
        assert_eq!(data.active_profile, "default");
        assert_eq!(
            data.profiles["copy"].mods.len(),
            data.profiles["other"].mods.len()
        );
        assert_ne!(data.profiles["copy"].id, data.profiles["other"].id);
    }
}
