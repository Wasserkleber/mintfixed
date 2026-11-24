//! Scope selection, mod filters and focus picker.
use super::super::*;

impl App {
    pub(super) fn show_lint_scope(&mut self, ui: &mut egui::Ui, available_mods: &[ModSpecification]) {
        ui.label("What should be scanned?");
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(
                &mut self.lint_scope,
                super::LintScope::All,
                "All enabled",
            );
            ui.selectable_value(
                &mut self.lint_scope,
                super::LintScope::Selected,
                "Selected mods",
            );
            ui.selectable_value(
                &mut self.lint_scope,
                super::LintScope::Focus,
                "Compare one mod",
            );
        });
        match self.lint_scope {
            super::LintScope::Selected => {
                ui.horizontal(|ui| {
                    if ui.small_button("Select all").clicked() {
                        let filter = self.lint_filter.trim().to_lowercase();
                        self.lint_selected_mods = available_mods
                            .iter()
                            .filter(|spec| {
                                if filter.is_empty() {
                                    return true;
                                }
                                let label = self
                                    .state
                                    .store
                                    .get_mod_info(spec)
                                    .map(|info| info.name)
                                    .unwrap_or_else(|| spec.url.clone());
                                label.to_lowercase().contains(&filter)
                                    || spec.url.to_lowercase().contains(&filter)
                            })
                            .cloned()
                            .collect();
                    }
                    if ui.small_button("Clear selection").clicked() {
                        self.lint_selected_mods.clear();
                    }
                    ui.label(format!(
                        "{} selected",
                        available_mods
                            .iter()
                            .filter(|s| self.lint_selected_mods.contains(*s))
                            .count()
                    ));
                });
                ui.add(
                    egui::TextEdit::singleline(&mut self.lint_filter)
                        .hint_text("Filter mod list..."),
                );
                egui::ScrollArea::vertical()
                    .max_height(165.0)
                    .show(ui, |ui| {
                        for spec in available_mods {
                            let label = self
                                .state
                                .store
                                .get_mod_info(spec)
                                .map(|info| info.name)
                                .unwrap_or_else(|| spec.url.clone());
                            if !label
                                .to_lowercase()
                                .contains(&self.lint_filter.to_lowercase())
                                && !spec
                                    .url
                                    .to_lowercase()
                                    .contains(&self.lint_filter.to_lowercase())
                            {
                                continue;
                            }
                            let mut checked =
                                self.lint_selected_mods.contains(spec);
                            if ui
                                .checkbox(&mut checked, label)
                                .on_hover_text(&spec.url)
                                .changed()
                            {
                                if checked {
                                    self.lint_selected_mods.insert(spec.clone());
                                } else {
                                    self.lint_selected_mods.remove(spec);
                                }
                            }
                        }
                    });
            }
            super::LintScope::Focus => {
                if self
                    .lint_focus
                    .as_ref()
                    .is_none_or(|f| !available_mods.contains(f))
                {
                    self.lint_focus = available_mods.first().cloned();
                }
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.lint_focus_filter)
                            .hint_text("Search mod name or URL...")
                            .desired_width(360.0),
                    );
                    if !self.lint_focus_filter.is_empty()
                        && ui.small_button("Clear").clicked()
                    {
                        self.lint_focus_filter.clear();
                    }
                });
                let filter = self.lint_focus_filter.trim().to_lowercase();
                let selected_label = self
                    .lint_focus
                    .as_ref()
                    .map(|spec| {
                        self.state
                            .store
                            .get_mod_info(spec)
                            .map(|info| info.name)
                            .unwrap_or_else(|| spec.url.clone())
                    })
                    .unwrap_or_else(|| "No enabled mods".to_owned());

                if filter.is_empty() {
                    egui::ComboBox::from_id_salt("lint_focus_combo")
                        .selected_text(selected_label)
                        .width(ui.available_width().min(460.0))
                        .show_ui(ui, |ui| {
                            for spec in available_mods {
                                let label = self
                                    .state
                                    .store
                                    .get_mod_info(spec)
                                    .map(|info| info.name)
                                    .unwrap_or_else(|| spec.url.clone());
                                ui.selectable_value(
                                    &mut self.lint_focus,
                                    Some(spec.clone()),
                                    label,
                                )
                                .on_hover_text(&spec.url);
                            }
                        });
                } else {
                    // Render matches inline: the regular ComboBox remains closed when typing.
                    let matches: Vec<_> = available_mods
                        .iter()
                        .filter(|spec| {
                            let label = self
                                .state
                                .store
                                .get_mod_info(spec)
                                .map(|info| info.name)
                                .unwrap_or_else(|| spec.url.clone());
                            label.to_lowercase().contains(&filter)
                                || spec.url.to_lowercase().contains(&filter)
                        })
                        .collect();
                    ui.small(format!(
                        "{} matching mods - click one to select",
                        matches.len()
                    ));
                    egui::ScrollArea::vertical()
                        .id_salt("lint_focus_live_results")
                        .max_height(165.0)
                        .show(ui, |ui| {
                            if matches.is_empty() {
                                ui.label("No matching enabled mods");
                            }
                            for spec in matches {
                                let label = self
                                    .state
                                    .store
                                    .get_mod_info(spec)
                                    .map(|info| info.name)
                                    .unwrap_or_else(|| spec.url.clone());
                                let selected =
                                    self.lint_focus.as_ref() == Some(spec);
                                if ui
                                    .selectable_label(selected, label)
                                    .on_hover_text(&spec.url)
                                    .clicked()
                                {
                                    self.lint_focus = Some(spec.clone());
                                    self.lint_focus_filter.clear();
                                }
                            }
                        });
                    ui.small(format!("Selected: {selected_label}"));
                }
                ui.small(
                    "Compares shared asset paths against every other enabled mod.",
                );
            }
            super::LintScope::All => {
                ui.small(format!(
                    "Scanning {} enabled mods in this profile",
                    available_mods.len()
                ));
            }
        }
    }
}
