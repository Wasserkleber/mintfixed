// Main frame concern extracted from app/frame.rs; original statements and ordering preserved.
use super::*;

impl App {
    pub(super) fn frame_overlays(&mut self, ctx: &egui::Context) {
        // begin draw

        self.show_update_window(ctx);
        #[cfg(all(feature = "hook", target_os = "windows"))]
        self.startup_crash_reporter_update.show(ctx);
        self.show_provider_parameters(ctx);
        self.show_profile_windows(ctx);
        if missing_local_files::show(ctx, self.state.mod_data.deref_mut().deref_mut()) {
            self.state.mod_data.save().unwrap();
            self.edit_history
                .reset(&self.state.mod_data, self.state.mod_data.revision());
        }
        self.show_settings(ctx);
        folder_import::tick(self, ctx);
        folder_import::show_preview(self, ctx);
        self.show_lints_toggle(ctx);
        self.show_lint_report(ctx);
        // Check the configured installation without accessing the disk every frame.
        if self
            .crash_monitor
            .as_ref()
            .is_none_or(|monitor| !monitor.matches_installation(&self.state))
        {
            self.crash_monitor = Some(crash_notice::CrashMonitor::start(&self.state, ctx));
        }
        if let Some(monitor) = &mut self.crash_monitor {
            monitor.poll(&mut self.recent_crash);
        }
        crash_notice::show(ctx, &mut self.recent_crash);

        self.crash_history.show(ctx, &self.state);
        self.show_bitfix_menu(ctx);
        self.show_bitfix_conflict(ctx);
        self.show_replacement_prompt(ctx);
    }
}
