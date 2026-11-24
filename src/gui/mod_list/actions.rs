// mod-list actions and sorting configuration; methods moved intact from gui/mod.rs.
use crate::gui::*;

impl App {
    pub(in crate::gui) fn apply_mod_history(&mut self, ctx: &egui::Context, redo: bool) {
        match self.edit_history.restore(&mut self.state.mod_data, redo) {
            Ok(true) => {
                // History restores the saved list, not the transient drag selection.
                Drag::clear_history_selection(ctx);
                self.profile_issues.clear();
                ctx.request_repaint();
                self.last_action = Some(LastAction::success(if redo {
                    "Redid mod-list change".into()
                } else {
                    "Undid mod-list change".into()
                }));
            }
            Ok(false) => {}
            Err(error) => self.last_action = Some(LastAction::failure(error)),
        }
    }

    pub(in crate::gui) fn parse_mods(&self) -> Vec<ModSpecification> {
        self.resolve_mod
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .map(|l| ModSpecification::new(l.to_string()))
            .collect()
    }

    pub(in crate::gui) fn get_sorting_config(&self) -> Option<SortingConfig> {
        self.state.config.sorting_config.clone()
    }

    pub(in crate::gui) fn update_sorting_config(
        &mut self,
        sort_category: Option<SortBy>,
        is_ascending: bool,
    ) {
        self.state.config.sorting_config = sort_category.map(|sort_category| SortingConfig {
            sort_category,
            is_ascending,
        });
        self.state.config.save().unwrap();
    }
}
