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

#[cfg(test)]
#[path = "tests/output.rs"]
mod tests;
