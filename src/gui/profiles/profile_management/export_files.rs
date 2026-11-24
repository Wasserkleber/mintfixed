//! Export paths, safe filenames and no-overwrite file creation.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum ProfileExportKind {
    #[default]
    Json,
    Txt,
}

impl ProfileExportKind {
    fn extension(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Txt => "txt",
        }
    }
}

/// Create a new export file beside the executable, without replacing an old one.
pub(super) fn save_profile_export(
    profile_name: &str,
    kind: ProfileExportKind,
    contents: &str,
) -> Result<std::path::PathBuf, String> {
    let exe = std::env::current_exe()
        .map_err(|error| format!("Cannot locate the mintfixed executable: {error}"))?;
    save_profile_export_beside(&exe, profile_name, kind.extension(), contents)
}

fn save_profile_export_beside(
    exe: &std::path::Path,
    name: &str,
    extension: &str,
    contents: &str,
) -> Result<std::path::PathBuf, String> {
    use std::io::Write as _;

    let folder = exe
        .parent()
        .ok_or("Cannot determine the executable's folder")?
        .join("mintfixed exports");
    std::fs::create_dir_all(&folder)
        .map_err(|error| format!("Cannot create export folder {}: {error}", folder.display()))?;

    // Make profile names safe for Windows filenames (including reserved device names).
    let safe: String = name
        .chars()
        .take(100)
        .map(|ch| match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            ch if ch.is_control() => '_',
            ch => ch,
        })
        .collect();
    let safe = safe.trim_matches(|ch: char| ch == '.' || ch.is_whitespace());
    let safe = if safe.is_empty() { "profile" } else { safe };
    let device = safe
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let reserved = matches!(
        device.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    );
    let safe = if reserved {
        format!("_{safe}")
    } else {
        safe.to_owned()
    };

    for number in 1..=10_000 {
        let filename = if number == 1 {
            format!("{safe}.{extension}")
        } else {
            format!("{safe} ({number}).{extension}")
        };
        let path = folder.join(filename);
        // Atomic create_new also handles another export starting at the same time.
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                if let Err(error) = file.write_all(contents.as_bytes()) {
                    drop(file);
                    let _ = std::fs::remove_file(&path);
                    return Err(format!(
                        "Could not write export {}: {error}",
                        path.display()
                    ));
                }
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "Could not create export {}: {error}",
                    path.display()
                ));
            }
        }
    }
    Err(format!(
        "Too many exports for profile \"{name}\" in {}",
        folder.display()
    ))
}

/// Export the selected profile as JSON, TXT or a portable ZIP bundle.
/// JSON preserves group definitions, order and overrides; TXT is enabled mod URLs.
// Portable profiles use the same export directory as JSON/TXT profiles and groups.
// The ZIP writer uses create_new(), so choosing a free path never overwrites a file.
pub(super) fn portable_profile_export_path(name: &str) -> Result<std::path::PathBuf, String> {
    let exe = std::env::current_exe()
        .map_err(|error| format!("Cannot locate the mintfixed executable: {error}"))?;
    portable_profile_export_path_beside(&exe, name)
}

fn portable_profile_export_path_beside(
    exe: &std::path::Path,
    name: &str,
) -> Result<std::path::PathBuf, String> {
    let folder = exe
        .parent()
        .ok_or("Cannot determine the executable's folder")?
        .join("mintfixed exports");
    std::fs::create_dir_all(&folder)
        .map_err(|error| format!("Cannot create export folder {}: {error}", folder.display()))?;
    let mut safe = crate::gui::group_controls::group_package::safe_component(name);
    // Windows device names are invalid even when followed by .zip.
    if matches!(
        safe.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    ) {
        safe.insert(0, '_');
    }
    for number in 1..=10_000 {
        let filename = if number == 1 {
            format!("{safe}-profile.zip")
        } else {
            format!("{safe}-profile ({number}).zip")
        };
        let path = folder.join(filename);
        match std::fs::metadata(&path) {
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(path),
            Err(error) => {
                return Err(format!(
                    "Cannot access export path {}: {error}",
                    path.display()
                ));
            }
        }
    }
    Err(format!(
        "Too many portable profile exports in {}",
        folder.display()
    ))
}

#[cfg(test)]
mod profile_export_file_tests {
    use super::*;
    #[test]
    fn portable_zip_export_path_uses_group_export_folder_and_does_not_overwrite() {
        let base = std::env::temp_dir().join(format!(
            "mintfixed-portable-zip-path-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        std::fs::create_dir_all(&base).unwrap();
        let exe = base.join("mintfixed.exe");
        let first = portable_profile_export_path_beside(&exe, "Test: Profile").unwrap();
        assert_eq!(first.parent().unwrap(), base.join("mintfixed exports"));
        assert_eq!(first.file_name().unwrap(), "Test__Profile-profile.zip");
        std::fs::write(&first, b"original").unwrap();
        let second = portable_profile_export_path_beside(&exe, "Test: Profile").unwrap();
        assert_eq!(second.file_name().unwrap(), "Test__Profile-profile (2).zip");
        assert_eq!(std::fs::read(&first).unwrap(), b"original");
        let reserved = portable_profile_export_path_beside(&exe, "CON").unwrap();
        assert_eq!(reserved.file_name().unwrap(), "_CON-profile.zip");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn profile_export_creates_exe_adjacent_folder_and_never_overwrites() {
        let base = std::env::temp_dir().join(format!(
            "mintfixed-profile-export-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&base).unwrap();
        let exe = base.join("mintfixed.exe");
        let first = save_profile_export_beside(&exe, "Test: Profile", "json", "first").unwrap();
        assert_eq!(first.parent().unwrap(), base.join("mintfixed exports"));
        assert_eq!(first.file_name().unwrap(), "Test_ Profile.json");
        let second = save_profile_export_beside(&exe, "Test: Profile", "json", "second").unwrap();
        assert_eq!(second.file_name().unwrap(), "Test_ Profile (2).json");
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "first");
        assert_eq!(std::fs::read_to_string(&second).unwrap(), "second");
        let txt = save_profile_export_beside(&exe, "CON", "txt", "url\n").unwrap();
        assert_eq!(txt.file_name().unwrap(), "_CON.txt");
        std::fs::remove_dir_all(base).unwrap();
    }
}
