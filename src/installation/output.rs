//! Stage an installation archive beside its destination until it is complete.
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

pub(crate) struct InstallOutput {
    temporary: tempfile::NamedTempFile,
    destination: PathBuf,
}

impl InstallOutput {
    pub(crate) fn new(destination: &Path) -> io::Result<Self> {
        let parent = destination.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Output archive has no parent directory",
            )
        })?;
        remove_abandoned(parent);
        Ok(Self {
            temporary: tempfile::Builder::new()
                .prefix(".mint-install-")
                .suffix(".tmp")
                .tempfile_in(parent)?,
            destination: destination.to_owned(),
        })
    }

    pub(crate) fn file(&mut self) -> &mut File {
        self.temporary.as_file_mut()
    }

    /// Call only after the archive index and all buffered bytes were written.
    pub(crate) fn commit(self) -> io::Result<()> {
        self.temporary.as_file().sync_all()?;
        self.temporary.persist(&self.destination).map_err(|error| {
            io::Error::new(error.error.kind(), format!(
                "Could not replace {}: {}. The previous archive was not removed. Close the game and retry.",
                self.destination.display(), error.error
            ))
        })?;
        Ok(())
    }
}

/// Deletes staging archives that killed integrations left beside the output: `NamedTempFile` removes its file
/// only when dropped, which a killed process never does. They are multi-GB each.
/// Untouched for an hour means abandoned; a live one is written to throughout its integration.
fn remove_abandoned(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > std::time::Duration::from_secs(3600));
        if old && name.starts_with(".mint-install-") && name.ends_with(".tmp") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
#[path = "tests/output.rs"]
mod tests;
