use crate::gui::*;
use mint_lib::mod_replacements;
use std::collections::HashSet;

#[derive(Clone, Copy)]
enum Choice {
    UseReplacement,
    KeepOriginal,
    Cancel,
}

impl App {
    pub(in crate::gui) fn show_replacement_prompt(&mut self, ctx: &egui::Context) {
        let Some(mut pending) = self.resolve_state.pending_replacement_addition.take() else {
            return;
        };
        let Some(rule) = pending
            .specs
            .iter()
            .filter_map(|spec| mod_replacements::rule_for_original(spec))
            .find(|rule| !pending.decided.contains(&rule.original))
            .cloned()
        else {
            // All choices are complete. Continue through the existing add-mod path.
            message::ResolveMods::send_confirmed(self, ctx, pending.specs, pending.is_dependency);
            return;
        };
        // mintfixed_modhub_enabled_menu_warning_v2: use the profile's enabled mod iterator (including group members).
        let mintfixed_existing_modhub_menu = if rule.original == "https://mod.io/g/drg/m/mod-hub" {
            let uui = ModSpecification::new("https://mod.io/g/drg/m/unified-ui".to_owned());
            let emm = ModSpecification::new("https://mod.io/g/drg/m/emm".to_owned());
            let mut has_uui = false;
            let mut has_emm = false;
            self.state.mod_data.for_each_enabled_mod(
                &self.state.mod_data.active_profile,
                |mod_config| {
                    has_uui |= mod_config.spec.matches_identity(&uui);
                    has_emm |= mod_config.spec.matches_identity(&emm);
                },
            );
            match (has_uui, has_emm) {
                (true, true) => Some("Unified UI (UUI) and EMM"),
                (true, false) => Some("Unified UI (UUI)"),
                (false, true) => Some("EMM"),
                (false, false) => None,
            }
        } else {
            None
        };
        let mut choice = None;
        egui::Window::new(format!("{} alternative", rule.original_name))
            .id(egui::Id::new("mod_replacement_confirmation"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_min_width(390.0);
                ui.label(format!(
                    "You're trying to add {}. We recommend using {} instead.",
                    rule.original_name, rule.replacement_name
                ));
                if !rule.description.is_empty() {
                    ui.label(&rule.description);
                }
                // mintfixed_modhub_enabled_menu_warning_v2: advisory only; Keep ModHub remains available.
                if let Some(existing) = mintfixed_existing_modhub_menu {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        format!("{existing} is already enabled in this profile. Installing ModHub too is usually unnecessary. Only keep ModHub if a particular mod specifically requires it."),
                    );
                }
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add(crate::gui::mint_text_button(format!("Use {} (recommended)", rule.replacement_name)))
                        .clicked()
                    {
                        choice = Some(Choice::UseReplacement);
                    }
                    if ui.add(crate::gui::mint_text_button(format!("Keep {}", rule.original_name))).clicked() {
                        choice = Some(Choice::KeepOriginal);
                    }
                    if ui.add(crate::gui::mint_text_button("Cancel")).clicked() {
                        choice = Some(Choice::Cancel);
                    }
                });
            });
        match choice {
            None => self.resolve_state.pending_replacement_addition = Some(pending),
            Some(Choice::Cancel) => {}
            Some(selected) => {
                let same_profile = self
                    .state
                    .mod_data
                    .profiles
                    .get(&self.state.mod_data.active_profile)
                    .is_some_and(|p| p.id == pending.profile_id);
                if !same_profile {
                    self.status_state.last_action = Some(LastAction::failure(
                        "Mod addition cancelled: the active profile changed.".into(),
                    ));
                    return;
                }
                if matches!(selected, Choice::UseReplacement) {
                    for spec in &mut pending.specs {
                        if mod_replacements::rule_for_original(spec)
                            .is_some_and(|r| r.original == rule.original)
                        {
                            *spec = ModSpecification::new(rule.replacement.clone());
                        }
                    }
                    let mut seen = HashSet::new();
                    pending.specs.retain(|spec| seen.insert(spec.clone()));
                }
                pending.decided.insert(rule.original);
                self.resolve_state.pending_replacement_addition = Some(pending);
                ctx.request_repaint();
            }
        }
    }
}
