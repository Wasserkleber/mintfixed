//! Startup update of Mintfixed's pinned BP-Crash-Reporter release.
//! Runs on a worker thread to keep egui responsive.

use std::sync::mpsc::{self, Receiver};

use eframe::egui;
use mint_lib::DRGInstallation;

use crate::state::State;

enum Event {
    Started,
    Finished(Result<bool, String>),
}

enum Status {
    Updating,
    Updated,
    Failed(String),
}

pub(super) struct StartupUpdate {
    events: Receiver<Event>,
    window: Option<Status>,
    dismissed: bool,
}

impl StartupUpdate {
    pub(super) fn start(state: &State, ctx: &egui::Context) -> Self {
        let (tx, events) = mpsc::channel();
        if state.config.bundled_crash_reporter
            && let Some(pak_path) = state.config.drg_pak_path.clone()
        {
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let notify_tx = tx.clone();
                let result = DRGInstallation::from_pak_path(&pak_path)
                    .map_err(|error| error.to_string())
                    .and_then(|installation| {
                        crate::crash_reporter::install_with_notice(
                            &installation.binaries_directory(),
                            &installation.installation_type,
                            true,
                            || {
                                let _ = notify_tx.send(Event::Started);
                                ctx.request_repaint();
                            },
                        )
                        .map_err(|error| format!("{error:#}"))
                    });
                if let Err(error) = &result {
                    tracing::warn!("BP-Crash-Reporter startup check failed: {error}");
                }
                let _ = tx.send(Event::Finished(result));
                ctx.request_repaint();
            });
        }
        Self {
            events,
            window: None,
            dismissed: false,
        }
    }

    pub(super) fn show(&mut self, ctx: &egui::Context) {
        for event in self.events.try_iter() {
            match event {
                Event::Started if !self.dismissed => self.window = Some(Status::Updating),
                Event::Finished(Ok(true)) if !self.dismissed => self.window = Some(Status::Updated),
                Event::Finished(Err(error)) if self.window.is_some() && !self.dismissed => {
                    self.window = Some(Status::Failed(error))
                }
                _ => {}
            }
        }
        let Some(status) = &self.window else {
            return;
        };
        let mut open = true;
        let mut dismiss = false;
        egui::Window::new("BP-Crash-Reporter update")
            .id(egui::Id::new("mintfixed-bpcrash-startup-update"))
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_min_width(360.0);
                match status {
                    Status::Updating => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(format!(
                                "Updating BP-Crash-Reporter to v{}...",
                                crate::crash_reporter::version()
                            ));
                        });
                        ui.label("The update is SHA-256 verified before installation.");
                        ui.label("You can continue using Mintfixed while it finishes.");
                    }
                    Status::Updated => {
                        ui.heading("Crash reporter updated");
                        ui.label(format!(
                            "BP-Crash-Reporter v{} is installed and ready.",
                            crate::crash_reporter::version()
                        ));
                        ui.label("Existing configuration and reports were preserved.");
                        if ui.button("OK").clicked() {
                            dismiss = true;
                        }
                    }
                    Status::Failed(error) => {
                        ui.heading("Crash reporter update failed");
                        ui.label(
                            "The previous executable was retained or restored where possible.",
                        );
                        ui.label("Close DRG and restart Mintfixed to retry.");
                        ui.collapsing("Error details", |ui| {
                            ui.label(error);
                        });
                        if ui.button("Close").clicked() {
                            dismiss = true;
                        }
                    }
                }
            });
        if dismiss || !open {
            self.dismissed = true;
            self.window = None;
        }
    }
}
