//! Portable and Windows-safe mod filenames.
use super::IncludedFile;
use std::{collections::HashSet, path::Path};

pub(crate) fn local_archive(url: &str) -> bool {
    if url.starts_with("http://") || url.starts_with("https://") {
        return false;
    }
    let extension = Path::new(url).extension().and_then(|v| v.to_str());
    matches!(extension, Some(ext) if ext.eq_ignore_ascii_case("pak") || ext.eq_ignore_ascii_case("zip"))
}

pub(crate) fn safe_component(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .take(90)
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let sanitized = sanitized.replace("..", "_");
    let trimmed = sanitized.trim_matches('.');
    if trimmed.is_empty() {
        "mod".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Use the actual filename in portable ZIPs, not a four-digit index.
/// The manifest tracks both its original name and SHA-256 independently.
pub(crate) fn bundle_filename(url: &str) -> Result<String, String> {
    let filename = url.rsplit(['/', '\\']).next().unwrap_or_default();
    let member = format!("mods/{filename}");
    if !safe_archive_path(&member) {
        return Err(format!(
            "Unsafe or unsupported local mod filename: {filename}"
        ));
    }
    Ok(member)
}

/// Keep ordinary ZIP members unchanged. Two *different source paths* may have
/// the same basename, so allocate a unique archive entry only on collision.
/// original_name in the manifest always retains the source's real filename.
pub(crate) fn unique_bundle_filename(
    url: &str,
    used: &mut HashSet<String>,
) -> Result<String, String> {
    let base = bundle_filename(url)?;
    if used.insert(base.to_lowercase()) {
        return Ok(base);
    }
    let filename = base.strip_prefix("mods/").ok_or("Invalid ZIP filename")?;
    let (stem, extension) = filename.rsplit_once('.').ok_or("Invalid mod extension")?;
    for index in 2..=10_000 {
        let suffix = format!(" ({index})");
        // Keep the Windows filename limit even when appending a collision suffix.
        let max_stem = 240usize.saturating_sub(suffix.len() + extension.len() + 1);
        let mut boundary = stem.len().min(max_stem);
        while !stem.is_char_boundary(boundary) {
            boundary -= 1;
        }
        let candidate = format!("mods/{}{}.{}", &stem[..boundary], suffix, extension);
        if safe_archive_path(&candidate) && used.insert(candidate.to_lowercase()) {
            return Ok(candidate);
        }
    }
    Err("Too many bundled local mods with the same name".into())
}

/// Reuse the same Windows-safe filename validation as imported originals.
/// This permits spaces and Unicode in normal mod filenames but rejects ZIP
/// traversal, directory components, alternate streams and device names.
pub(crate) fn safe_archive_path(path: &str) -> bool {
    let Some(filename) = path.strip_prefix("mods/") else {
        return false;
    };
    if filename.contains("..") {
        return false;
    }
    original_file_name(&IncludedFile {
        path: path.to_owned(),
        size: 0,
        sha256: String::new(),
        original_name: Some(filename.to_owned()),
    })
    .is_ok()
}

/// Reconstruct the displayed/original filename, including for old bundles
/// created before the manifest explicitly stored original_name.
pub(crate) fn original_file_name(file: &IncludedFile) -> Result<String, String> {
    let archive_name = file
        .path
        .strip_prefix("mods/")
        .ok_or("Invalid bundled mod path")?;
    let original = if let Some(original) = &file.original_name {
        original.as_str()
    } else if archive_name.len() > 5
        && archive_name.as_bytes()[..4].iter().all(u8::is_ascii_digit)
        && archive_name.as_bytes()[4] == b'_'
    {
        &archive_name[5..]
    } else {
        archive_name
    };
    // Validate before joining with local paths. No path components, alternate
    // data streams, Windows device names, or non-PAK/ZIP output is accepted.
    if original.is_empty()
        || original.len() > 240
        || original == "."
        || original == ".."
        || original.starts_with(' ')
        || original.ends_with(' ')
        || original.ends_with('.')
        || original.chars().any(|ch| {
            ch.is_control() || matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
    {
        return Err(format!("Unsafe original mod filename: {original}"));
    }
    let stem = original
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if matches!(
        stem.as_str(),
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
        return Err(format!("Windows reserved mod filename: {original}"));
    }
    if !matches!(Path::new(original).extension().and_then(|s| s.to_str()),
        Some(ext) if ext.eq_ignore_ascii_case("pak") || ext.eq_ignore_ascii_case("zip"))
    {
        return Err(format!("Unsupported original mod filename: {original}"));
    }
    Ok(original.to_owned())
}

pub(crate) fn unique_destination_name(original: &str, used: &mut HashSet<String>) -> String {
    if used.insert(original.to_ascii_lowercase()) {
        return original.to_owned();
    }
    let (stem, extension) = original.rsplit_once('.').expect("validated filename");
    for number in 2..=100_000 {
        let suffix = format!(" ({number})");
        let keep = stem
            .len()
            .min(240usize.saturating_sub(extension.len() + suffix.len() + 1));
        let mut boundary = keep;
        while !stem.is_char_boundary(boundary) {
            boundary -= 1;
        }
        let proposed = format!("{}{}.{}", &stem[..boundary], suffix, extension);
        if used.insert(proposed.to_ascii_lowercase()) {
            return proposed;
        }
    }
    unreachable!("exhausted import filenames")
}
