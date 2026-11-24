//! Profile Management popup (create, rename, duplicate, copy, import and delete profiles).
//! Kept separate from the reusable profile selector and the JSON importer.
use super::named_combobox::NamedEntries;

#[path = "profile_management/actions.rs"]
mod actions;
#[path = "profile_management/export_files.rs"]
mod export_files;
#[path = "profile_management/export_ui.rs"]
mod export_ui;
#[path = "profile_management/picker.rs"]
mod picker;

pub(super) use export_ui::copy_options;

// Persistent input state for the single profile management menu.
#[derive(Debug, Clone, Default)]
struct ProfileManagementEditor {
    new_name: String,
    rename_name: String,
    rename_target: String,
    rename_picker_open: bool,
    rename_success: Option<String>,
    duplicate_target: String,
    duplicate_name: String,
    duplicate_picker_open: bool,
    duplicate_success: Option<String>,
    export_feedback: Option<Result<String, String>>,
    delete_target: Option<String>,
}

/// Profile actions in one popup. Inline editors avoid nesting egui popups,
/// which would otherwise close each other or lose their text-field focus.
pub(super) fn ui<E, N>(
    ui: &mut egui::Ui,
    name: &str,
    entries: &mut N,
    modified: &mut bool,
    import_ui: Option<impl FnOnce(&mut egui::Ui, &mut N, &mut bool)>,
    copy_ui: Option<impl FnOnce(&mut egui::Ui, &mut N) -> Option<Result<String, String>>>,
) where
    N: NamedEntries<E>,
{
    let response = ui
        .add(crate::gui::mint_text_button("Profile Management"))
        .on_hover_text("Create, rename, duplicate, copy, import or delete a profile");
    let popup_id = ui.make_persistent_id(format!("manage-{name}"));
    let state_id = popup_id.with("fields");
    let mut editor: ProfileManagementEditor =
        ui.data(|data| data.get_temp(state_id)).unwrap_or_default();

    if response.clicked() {
        if ui.memory(|mem| mem.is_popup_open(popup_id)) {
            ui.memory_mut(|mem| mem.close_popup());
        } else {
            // Choose any source profile rather than silently locking actions to active.
            editor = ProfileManagementEditor::default();
            ui.memory_mut(|mem| mem.open_popup(popup_id));
        }
    }

    let mut copy_ui = copy_ui;
    let mut import_ui = import_ui;
    crate::gui::management_popup::show(ui, popup_id, &response, |ui| {
        ui.vertical(|ui| {
            actions::show_create(ui, entries, &mut editor, modified);
            ui.separator();
            actions::show_rename(ui, entries, &mut editor, modified);
            ui.separator();
            actions::show_duplicate(ui, entries, &mut editor, modified);
            ui.separator();
            let export_section = egui::CollapsingHeader::new("Export Profile")
                .id_salt("manage-copy-profile")
                .show(ui, |ui| {
                    if let Some(copy_ui) = copy_ui.take()
                        && let Some(result) = copy_ui(ui, entries)
                    {
                        editor.export_feedback = Some(result);
                    }
                    if let Some(result) = &editor.export_feedback {
                        match result {
                            Ok(message) => {
                                ui.colored_label(egui::Color32::from_rgb(100, 200, 120), message);
                            }
                            Err(message) => {
                                ui.colored_label(ui.visuals().warn_fg_color, message);
                            }
                        }
                    }
                });
            crate::gui::management_popup::scroll_expanded_header(
                &export_section.header_response,
                export_section.body_response.is_some(),
            );
            ui.separator();
            let import_section = egui::CollapsingHeader::new("Import Profile (JSON)")
                .id_salt("manage-import-profile")
                .show(ui, |ui| {
                    // This renders the actual importer directly inside this menu;
                    // no nested popup or competing keyboard focus.
                    if let Some(import_ui) = import_ui.take() {
                        import_ui(ui, entries, modified);
                    }
                });
            crate::gui::management_popup::scroll_expanded_header(
                &import_section.header_response,
                import_section.body_response.is_some(),
            );
            ui.separator();
            actions::show_delete(ui, entries, &mut editor, modified, popup_id);
        });
    });
    ui.data_mut(|data| data.insert_temp(state_id, editor));
}
