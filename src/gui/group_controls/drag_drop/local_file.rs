//! Local mod name actions. Keep filesystem paths out of the browser opener.
use super::Drag;
use std::path::{Path, PathBuf};

impl Drag {
    pub(in crate::gui) fn is_local_mod_path(spec: &str) -> bool {
        // A missing local file has no FileProvider metadata, so also recognize
        // absolute paths and local archive extensions without checking existence.
        if spec.is_empty() || spec.contains("://") {
            return false;
        }
        let path = Path::new(spec);
        path.is_absolute()
            || path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| {
                    ext.eq_ignore_ascii_case("pak") || ext.eq_ignore_ascii_case("zip")
                })
            || path.exists()
    }
}

fn absolute_path(spec: &str) -> PathBuf {
    std::path::absolute(spec).unwrap_or_else(|_| PathBuf::from(spec))
}

pub(super) fn decorate_name(
    response: egui::Response,
    spec: &str,
    selecting: bool,
) -> egui::Response {
    let response = response.on_hover_ui(|ui| {
        let path = absolute_path(spec);
        ui.weak("Local mod");
        ui.label(path.to_string_lossy().into_owned());
        if !path.is_file() {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "Local file is missing or inaccessible",
            );
            if !selecting {
                ui.weak("Click to open its folder, if available");
            }
        } else if !selecting {
            ui.weak("Click to open file location");
        }
        if selecting {
            ui.weak("Click to select; Shift+click for a range");
        }
    });
    response.context_menu(|ui| {
        if ui.button("Open file location").clicked() {
            reveal(spec);
            ui.close_menu();
        }
        if ui.button("Copy file path").clicked() {
            ui.ctx()
                .copy_text(absolute_path(spec).to_string_lossy().into_owned());
            ui.close_menu();
        }
    });
    response
}

pub(super) fn reveal(spec: &str) {
    let path = absolute_path(spec);
    // D-Bus and desktop launchers can block; never run them on the GUI thread.
    let _worker = std::thread::spawn(move || {
        if path.is_file() && try_select_file(&path) {
            return;
        }
        if let Some(parent) = path.parent().filter(|parent| parent.is_dir()) {
            if let Err(error) = opener::open(parent) {
                tracing::warn!(path = %path.display(), %error, "Could not open local mod folder");
            }
        } else {
            tracing::warn!(path = %path.display(), "Local mod folder does not exist");
        }
    });
}

#[cfg(target_os = "windows")]
fn try_select_file(path: &Path) -> bool {
    use std::os::windows::process::CommandExt;
    // Explorer expects /select,"path" as a single command-line fragment.
    // Windows file names cannot contain a double quote.
    std::process::Command::new("explorer.exe")
        .raw_arg(format!("/select,\"{}\"", path.display()))
        .spawn()
        .is_ok()
}

#[cfg(target_os = "linux")]
fn try_select_file(path: &Path) -> bool {
    // FileManager1.ShowItems selects the file on compatible Linux desktops.
    // dbus-send is optional; the caller falls back to opening the parent folder.
    let Ok(uri) = url::Url::from_file_path(path) else {
        return false;
    };
    // dbus-send uses commas to separate array entries.
    let arg = format!("array:string:{}", uri.as_str().replace(',', "%2C"));
    std::process::Command::new("dbus-send")
        .args([
            "--session",
            "--print-reply",
            "--reply-timeout=1500",
            "--dest=org.freedesktop.FileManager1",
            "/org/freedesktop/FileManager1",
            "org.freedesktop.FileManager1.ShowItems",
        ])
        .arg(arg)
        .arg("string:")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(target_os = "macos")]
fn try_select_file(path: &Path) -> bool {
    std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
fn try_select_file(_path: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_paths_are_recognized_even_when_missing() {
        assert!(Drag::is_local_mod_path("missing/mod.pak"));
        assert!(Drag::is_local_mod_path("missing/mod.ZIP"));
        assert!(Drag::is_local_mod_path(r"C:\Mods\Missing.PAK"));
    }

    #[test]
    fn remote_archives_are_not_local_paths() {
        assert!(!Drag::is_local_mod_path("https://example.com/mod.pak"));
        assert!(!Drag::is_local_mod_path("https://mod.io/g/drg/m/example"));
        assert!(!Drag::is_local_mod_path(""));
    }
}
