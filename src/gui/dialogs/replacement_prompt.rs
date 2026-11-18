use super::*;
use mint_lib::mod_replacements;
use std::collections::HashSet;

#[derive(Clone, Copy)]
enum Choice {
    UseReplacement,
    KeepOriginal,
    Cancel,
}

impl App {
    pub(super) fn show_replacement_prompt(&mut self, ctx: &egui::Context) {
        let Some(mut pending) = self.pending_replacement_addition.take() else {
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
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .button(format!("Use {} (recommended)", rule.replacement_name))
                        .clicked()
                    {
                        choice = Some(Choice::UseReplacement);
                    }
                    if ui.button(format!("Keep {}", rule.original_name)).clicked() {
                        choice = Some(Choice::KeepOriginal);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(Choice::Cancel);
                    }
                });
            });
        match choice {
            None => self.pending_replacement_addition = Some(pending),
            Some(Choice::Cancel) => {}
            Some(selected) => {
                let same_profile = self
                    .state
                    .mod_data
                    .profiles
                    .get(&self.state.mod_data.active_profile)
                    .is_some_and(|p| p.id == pending.profile_id);
                if !same_profile {
                    self.last_action = Some(LastAction::failure(
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
                self.pending_replacement_addition = Some(pending);
                ctx.request_repaint();
            }
        }
    }
}
