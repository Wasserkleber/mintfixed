// Main eframe UI frame and event handling; extracted without changing behavior.
use super::*;

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame_begin(ctx);
        self.refresh_unavailable_overview();
        self.frame_overlays(ctx);
        self.frame_toolbar(ctx);
        self.frame_status_details(ctx);
        self.frame_main_panel(ctx);
        self.show_group_mod_removal_confirmation(ctx);
        if !ctx.input(|i| i.pointer.primary_down()) {
            self.edit_history
                .sync(&self.state.mod_data, self.state.mod_data.revision());
        }
    }
}
