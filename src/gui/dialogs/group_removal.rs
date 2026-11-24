// shared-group removal confirmation; methods moved intact from gui/mod.rs.
use super::*;

impl App {
    pub(super) fn show_group_mod_removal_confirmation(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.pending_group_mod_removal else {
            return;
        };

        let mut open = true;
        let mut confirmed = false;
        let mut cancelled = false;
        egui::Window::new("Remove mod from group?")
            .id(egui::Id::new("confirm_group_mod_removal"))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(format!(
                    "Remove '{}' from group '{}'?",
                    pending.mod_name, pending.group_name,
                ));
                ui.add_space(6.0);
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "⚠ Warning: this is a shared group.",
                );
                ui.label("This removes the mod from EVERY profile using this group.");
                ui.label("Other mods in the group will not be affected.");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add(crate::gui::mint_text_button("Cancel")).clicked() {
                        cancelled = true;
                    }
                    if ui
                        .add(
                            crate::gui::mint_text_button("Remove from group")
                                .fill(colors::DARK_RED),
                        )
                        .clicked()
                    {
                        confirmed = true;
                    }
                });
            });

        if !open || cancelled || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.pending_group_mod_removal = None;
            return;
        }
        if !confirmed {
            return;
        }

        let pending = self.pending_group_mod_removal.take().unwrap();
        let before = self.state.mod_data.deref().deref().clone();
        let removed = group_controls::remove_group_member(
            self.state.mod_data.deref_mut().deref_mut(),
            &pending.group_name,
            &pending.mod_id,
        );

        if removed.is_err() {
            self.last_action = Some(LastAction::failure(
                "The mod or group changed before removal. Nothing was deleted.".into(),
            ));
            return;
        }
        match self.state.mod_data.save() {
            Ok(()) => {
                self.profile_issues.clear();
                self.last_action = Some(LastAction::success(format!(
                    "Removed '{}' from shared group '{}'",
                    pending.mod_name, pending.group_name,
                )));
            }
            Err(error) => {
                *self.state.mod_data.deref_mut().deref_mut() = before;
                self.last_action = Some(LastAction::failure(format!(
                    "Could not save group removal: {error}",
                )));
            }
        }
    }
}
