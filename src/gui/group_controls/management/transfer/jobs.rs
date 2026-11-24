use super::*;

pub(super) fn poll_package_jobs(
    ui: &Ui,
    export_editor: &mut ExportEditor,
    import_editor: &mut ImportEditor,
    requested_import_name: &mut Option<String>,
    success_message: &mut Option<String>,
) {
    // ZIP copying and SHA-256 verification run off the GUI thread.
    if let Some(job) = &export_editor.job {
        let completed = job.lock().ok().and_then(|mut value| value.take());
        if let Some(result) = completed {
            export_editor.job = None;
            match result {
                Ok(path) => {
                    let message = format!("Exported group package to {}", path.display());
                    export_editor.success = Some(message.clone());
                    export_editor.error = None;
                    *success_message = Some(message);
                }
                Err(error) => {
                    export_editor.success = None;
                    export_editor.error = Some(error);
                }
            }
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(300));
        }
    }
    if let Some(job) = &import_editor.job {
        let completed = job.lock().ok().and_then(|mut value| value.take());
        if let Some(result) = completed {
            import_editor.job = None;
            match result {
                Ok((name, group)) => {
                    import_editor.pending = Some(PendingGroupImport {
                        suggested_name: Some(name.clone()),
                        group,
                    });
                    import_editor.bundle_path = None;
                    *requested_import_name = Some(name);
                }
                Err(error) => import_editor.error = Some(error),
            }
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(300));
        }
    }
}
