//! Verified flat extraction, reuse of identical mods and rollback.
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use zip::ZipArchive;
use super::{IncludedFile, copy_and_hash, local_archive};
use super::filenames::{original_file_name, unique_destination_name};

/// Never follow symbolic links while discovering reusable local mods. Only
/// hash files whose size could match an entry in the package.
fn existing_mod_files(
    root: &Path,
    wanted_sizes: &HashSet<u64>,
) -> Result<std::collections::HashMap<(u64, String), Vec<PathBuf>>, String> {
    let mut index: std::collections::HashMap<(u64, String), Vec<PathBuf>> =
        std::collections::HashMap::new();
    let mut pending = vec![root.to_path_buf()];
    let mut visited = 0usize;
    while let Some(dir) = pending.pop() {
        let iter = fs::read_dir(&dir)
            .map_err(|e| format!("Cannot scan configured mod folder {}: {e}", dir.display()))?;
        for entry in iter {
            let entry = entry.map_err(|e| format!("Cannot scan configured mod folder: {e}"))?;
            visited += 1;
            if visited > 100_000 {
                return Err(
                    "Configured mod folder contains too many entries to scan safely".into(),
                );
            }
            let path = entry.path();
            let kind = entry
                .file_type()
                .map_err(|e| format!("Cannot inspect {}: {e}", path.display()))?;
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                // Never mistake another import's unfinished temporary files for usable mods.
                if !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".mintfixed-")
                {
                    pending.push(path);
                }
                continue;
            }
            if !kind.is_file() || !local_archive(&path.to_string_lossy()) {
                continue;
            }
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if !wanted_sizes.contains(&meta.len()) {
                continue;
            }
            let Ok(mut input) = File::open(&path) else {
                continue;
            };
            let Ok((size, checksum)) = copy_and_hash(&mut input, &mut std::io::sink(), meta.len())
            else {
                continue;
            };
            if size == meta.len() {
                index.entry((size, checksum)).or_default().push(path);
            }
        }
    }
    Ok(index)
}

/// Tracks only files that *this* import created. If the extraction fails,
/// or the group import cannot complete, existing files are never removed.
pub(crate) struct InstalledModFiles {
    pub(crate) mapping: BTreeMap<String, String>,
    created: Vec<PathBuf>,
    committed: bool,
}
impl InstalledModFiles {
    pub(crate) fn commit(&mut self) {
        self.committed = true;
    }
    /// Transfer cleanup responsibility to a pending profile import.
    pub(crate) fn into_created(mut self) -> Vec<PathBuf> {
        self.committed = true;
        std::mem::take(&mut self.created)
    }
}
impl Drop for InstalledModFiles {
    fn drop(&mut self) {
        if !self.committed {
            for path in self.created.iter().rev() {
                let _ = fs::remove_file(path);
            }
        }
    }
}

/// Shared profile/group import: check each ZIP payload, reuse existing files by
/// size and SHA-256, and install only new files straight into Settings' folder.
/// The temporary directory is for validation only and is never preserved.
pub(crate) fn extract_mod_files(
    archive: &mut ZipArchive<File>,
    files: &[IncludedFile],
    folder: &Path,
) -> Result<InstalledModFiles, String> {
    if !folder.is_dir() {
        return Err(format!(
            "Configured mod folder does not exist: {}",
            folder.display()
        ));
    }
    let wanted_sizes: HashSet<u64> = files.iter().map(|f| f.size).collect();
    let index = existing_mod_files(folder, &wanted_sizes)?;

    // Reserve names belonging to every existing root entry, including folders,
    // so a new mod never replaces a file or clashes with a directory.
    let mut used_names = HashSet::new();
    for entry in fs::read_dir(folder).map_err(|e| format!("Cannot read mod folder: {e}"))? {
        let entry = entry.map_err(|e| format!("Cannot inspect mod folder entry: {e}"))?;
        used_names.insert(entry.file_name().to_string_lossy().to_ascii_lowercase());
    }
    let staging = tempfile::Builder::new()
        .prefix(".mintfixed-bundle-")
        .tempdir_in(folder)
        .map_err(|e| format!("Cannot prepare temporary import: {e}"))?;

    let mut installed = InstalledModFiles {
        mapping: BTreeMap::new(),
        created: Vec::new(),
        committed: false,
    };
    let mut planned = std::collections::HashMap::<(u64, String), PathBuf>::new();
    let mut to_install = Vec::<(PathBuf, PathBuf)>::new();

    // Verify *all* bundled bytes before installing any mod into the root.
    for (number, file) in files.iter().enumerate() {
        let original = original_file_name(file)?;
        let scratch = staging.path().join(format!("payload-{number:04}"));
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&scratch)
            .map_err(|e| format!("Cannot create temporary mod: {e}"))?;
        let mut payload = archive
            .by_name(&file.path)
            .map_err(|e| format!("Missing bundled mod {}: {e}", file.path))?;
        let (size, digest) = copy_and_hash(&mut payload, &mut output, file.size)?;
        output
            .flush()
            .map_err(|e| format!("Cannot flush staged mod: {e}"))?;
        drop(output);
        if size != file.size || !digest.eq_ignore_ascii_case(&file.sha256) {
            return Err(format!(
                "Bundled mod failed integrity verification: {}",
                file.path
            ));
        }
        let key = (size, digest.to_ascii_lowercase());
        // Prefer an identical file with the original filename, otherwise any
        // byte-identical local file, including one inside a legacy subfolder.
        let existing = index.get(&key).and_then(|candidates| {
            candidates
                .iter()
                .find(|path| {
                    path.file_name()
                        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case(&original))
                })
                .or_else(|| candidates.first())
        });
        let resolved = if let Some(path) = existing {
            path.clone()
        } else if let Some(path) = planned.get(&key) {
            path.clone()
        } else {
            let filename = unique_destination_name(&original, &mut used_names);
            let destination = folder.join(filename);
            planned.insert(key, destination.clone());
            to_install.push((scratch.clone(), destination.clone()));
            destination
        };
        installed
            .mapping
            .insert(file.path.clone(), resolved.to_string_lossy().into_owned());
    }

    // hard_link is an atomic no-overwrite operation on Windows/NTFS and
    // Unix filesystems. Fallback uses exclusive create_new for filesystems
    // without hardlinks; both paths refuse an existing destination.
    for (staged, destination) in to_install {
        match fs::hard_link(&staged, &destination) {
            Ok(()) => {
                installed.created.push(destination);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(format!(
                    "Another file appeared during import: {} (retry import)",
                    destination.display()
                ));
            }
            Err(_) => {
                let mut input =
                    File::open(&staged).map_err(|e| format!("Cannot reopen verified mod: {e}"))?;
                let mut output = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&destination)
                    .map_err(|e| format!("Cannot safely create {}: {e}", destination.display()))?;
                // Mark as owned *before* copying, so partial writes are cleaned.
                installed.created.push(destination);
                std::io::copy(&mut input, &mut output)
                    .map_err(|e| format!("Cannot finish imported mod copy: {e}"))?;
                output
                    .flush()
                    .map_err(|e| format!("Cannot flush imported mod: {e}"))?;
            }
        }
    }
    Ok(installed)
}
