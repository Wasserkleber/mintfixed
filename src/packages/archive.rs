//! Shared ZIP envelope, limits and structural validation.
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

pub(crate) const PREFIX: &str = "mintfixed-bundle:";
pub(crate) const MAX_FILES: usize = 1000;
pub(crate) const MAX_TOTAL: u64 = 8 * 1024 * 1024 * 1024;
pub(crate) const MAX_DEFINITION: u64 = 8 * 1024 * 1024;
pub(crate) const MAX_MANIFEST: u64 = 512 * 1024;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct IncludedFile {
    pub(crate) path: String,
    pub(crate) size: u64,
    pub(crate) sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) original_name: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Manifest {
    pub(crate) format: String,
    pub(crate) version: u32,
    pub(crate) name: String,
    pub(crate) definition: String,
    pub(crate) files: Vec<IncludedFile>,
}


#[derive(Clone, Copy)]
pub(crate) enum ArchiveKind {
    Group,
    Profile,
}

impl ArchiveKind {
    fn message(self, group: &'static str, profile: &'static str) -> &'static str {
        match self {
            Self::Group => group,
            Self::Profile => profile,
        }
    }
}

pub(crate) fn bounded_text(reader: &mut impl Read, max: u64, kind: ArchiveKind) -> Result<String, String> {
    let mut bytes = Vec::new();
    reader.take(max + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > max {
        return Err(kind.message("Group metadata exceeds the allowed size", "Oversized profile ZIP metadata").into());
    }
    String::from_utf8(bytes).map_err(|e| match kind {
        ArchiveKind::Group => format!("Group metadata is not UTF-8: {e}"),
        ArchiveKind::Profile => format!("Invalid UTF-8 in profile ZIP: {e}"),
    })
}

/// Validate the common ZIP envelope before either format interprets its manifest.
/// Manifest membership and reference matching remain format-specific.
pub(crate) fn validate_entries<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    kind: ArchiveKind,
) -> Result<HashSet<String>, String> {
    if archive.len() > MAX_FILES + 2 {
        return Err(kind.message("Too many ZIP entries", "Too many files in profile ZIP").into());
    }
    let mut names = HashSet::new();
    let mut total = 0_u64;
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|e| e.to_string())?;
        let name = entry.name();
        if entry.is_dir()
            || name.contains('\\')
            || name.contains('\0')
            || name.starts_with('/')
            || name.contains("..")
            || name.contains(':')
        {
            return Err(match kind {
                ArchiveKind::Group => format!("Unsafe path in group ZIP: {name}"),
                ArchiveKind::Profile => format!("Unsafe entry in profile ZIP: {name}"),
            });
        }
        if entry.unix_mode().is_some_and(|mode| mode & 0o170000 == 0o120000) {
            return Err(kind.message("Symbolic links are not allowed in group packages", "Symlinks are not allowed in portable profile ZIPs").into());
        }
        if !names.insert(name.to_ascii_lowercase()) {
            return Err(kind.message("Duplicate ZIP entry names", "Duplicate filenames in profile ZIP").into());
        }
        total = total.checked_add(entry.size()).ok_or(kind.message("ZIP size overflow", "Profile ZIP size overflow"))?;
        if total > MAX_TOTAL + MAX_DEFINITION + MAX_MANIFEST {
            return Err(kind.message("ZIP exceeds the 8 GiB limit", "Profile ZIP exceeds 8 GiB limit").into());
        }
        if name.starts_with("mods/") && entry.compression() != CompressionMethod::Stored {
            return Err(kind.message("Packaged mod files must use ZIP stored mode", "Bundled mod files must use ZIP stored mode").into());
        }
    }
    Ok(names)
}

/// Write a new package in stored mode, removing incomplete exports on failure.
/// The supplied manifest describes the format; payload metadata is computed here.
pub(crate) fn write_package(
    destination: &Path,
    mut manifest: Manifest,
    definition: &[u8],
    sources: Vec<(PathBuf, String, u64)>,
    kind: ArchiveKind,
) -> Result<(), String> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|e| match kind {
            ArchiveKind::Group => format!("Cannot create package {}: {e}", destination.display()),
            ArchiveKind::Profile => format!("Cannot create portable profile ZIP {}: {e}", destination.display()),
        })?;
    let outcome = (|| -> Result<(), String> {
        let mut archive = ZipWriter::new(file);
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        archive.start_file(manifest.definition.as_str(), stored).map_err(|e| e.to_string())?;
        archive.write_all(definition).map_err(|e| e.to_string())?;
        for (source, member, expected_size) in sources {
            archive.start_file(member.as_str(), stored).map_err(|e| e.to_string())?;
            let mut input = File::open(&source)
                .map_err(|e| format!("Cannot open {}: {e}", source.display()))?;
            let (size, hash) = super::copy_and_hash(&mut input, &mut archive, MAX_TOTAL)?;
            if size != expected_size {
                return Err(match kind {
                    ArchiveKind::Group => format!("Local file changed during export: {}", source.display()),
                    ArchiveKind::Profile => format!("File changed during export: {}", source.display()),
                });
            }
            manifest.files.push(IncludedFile {
                path: member,
                size,
                sha256: hash,
                original_name: Some(
                    source.file_name().ok_or("Invalid original filename")?.to_string_lossy().into_owned(),
                ),
            });
        }
        let json = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
        if json.len() as u64 > MAX_MANIFEST {
            return Err(kind.message("Too much package metadata", "Profile package manifest exceeds 512 KiB").into());
        }
        archive.start_file("manifest.json", stored).map_err(|e| e.to_string())?;
        archive.write_all(&json).map_err(|e| e.to_string())?;
        archive.finish().map_err(|e| match kind {
            ArchiveKind::Group => format!("Cannot finalize ZIP: {e}"),
            ArchiveKind::Profile => format!("Cannot finish profile ZIP: {e}"),
        })?;
        Ok(())
    })();
    if outcome.is_err() {
        let _ = fs::remove_file(destination);
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn both_formats_reject_unsafe_and_case_duplicate_entries() {
        for names in [
            vec!["../escape.pak"],
            vec!["mods/One.pak", "mods/one.pak"],
        ] {
            let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
            let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            for name in names {
                writer.start_file(name, options).unwrap();
                writer.write_all(b"test").unwrap();
            }
            let mut bytes = writer.finish().unwrap();
            bytes.set_position(0);
            let mut archive = ZipArchive::new(bytes).unwrap();
            for kind in [ArchiveKind::Group, ArchiveKind::Profile] {
                assert!(validate_entries(&mut archive, kind).is_err());
            }
        }
    }

    #[test]
    fn both_formats_enforce_metadata_size_and_utf8() {
        for kind in [ArchiveKind::Group, ArchiveKind::Profile] {
            assert_eq!(bounded_text(&mut &b"ok"[..], 2, kind).unwrap(), "ok");
            assert!(bounded_text(&mut &b"too long"[..], 2, kind).is_err());
            assert!(bounded_text(&mut &[0xff][..], 2, kind).is_err());
        }
    }
}
