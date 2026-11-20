//! A mod file's `.pak` bytes, opened once and shared by every thread that reads entries from it.
use std::io::{Read, SeekFrom};
use std::sync::Arc;

use super::*;

/// A plain `.pak` stays on disk and is read with positioned reads; a zip's first `.pak` is inflated into
/// memory once (mod.io serves zips). Cloning is cheap: each worker takes its own `reader()` and seek position.
#[derive(Clone)]
pub(crate) enum PakSource {
    Memory { bytes: Arc<[u8]>, member: Arc<str> },
    File(Arc<std::fs::File>, u64),
}

impl PakSource {
    pub(crate) fn open(mut file: fs::File) -> Result<Self, IntegrationError> {
        let Ok(mut archive) = zip::ZipArchive::new(&mut file) else {
            let len = file.metadata()?.len();
            return Ok(Self::File(Arc::new(file.into_parts().0), len));
        };
        for i in 0..archive.len() {
            let mut entry = archive
                .by_index(i)
                .map_err(|_| IntegrationError::GenericError {
                    msg: "failed to extract file in zip archive".to_string(),
                })?;
            let is_pak = entry
                .enclosed_name()
                .is_some_and(|p| p.extension() == Some(std::ffi::OsStr::new("pak")));
            if entry.is_file() && is_pak {
                let mut buf = Vec::with_capacity(entry.size() as usize);
                entry.read_to_end(&mut buf)?;
                return Ok(Self::Memory {
                    bytes: buf.into(),
                    member: Arc::from(entry.name()),
                });
            }
        }
        GenericSnafu {
            msg: "zip archive does not contain pak",
        }
        .fail()
    }

    // Remember the ZIP member actually chosen by integration. Arc keeps cloning cheap.
    pub(crate) fn selected_member(&self) -> Option<&str> {
        match self {
            Self::Memory { member, .. } => Some(member.as_ref()),
            Self::File(..) => None,
        }
    }

    pub(crate) fn reader(&self) -> SourceReader {
        SourceReader {
            source: self.clone(),
            pos: 0,
        }
    }

    fn len(&self) -> u64 {
        match self {
            Self::Memory { bytes, .. } => bytes.len() as u64,
            Self::File(_, len) => *len,
        }
    }
}

pub(crate) struct SourceReader {
    source: PakSource,
    pos: u64,
}

impl Read for SourceReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = match &self.source {
            PakSource::Memory { bytes, .. } => {
                let rest = bytes.get(self.pos as usize..).unwrap_or_default();
                let n = rest.len().min(buf.len());
                buf[..n].copy_from_slice(&rest[..n]);
                n
            }
            #[cfg(windows)]
            PakSource::File(file, _) => {
                std::os::windows::fs::FileExt::seek_read(&**file, buf, self.pos)?
            }
            #[cfg(unix)]
            PakSource::File(file, _) => {
                std::os::unix::fs::FileExt::read_at(&**file, buf, self.pos)?
            }
        };
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for SourceReader {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let pos = match to {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::End(d) => self.source.len().checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        };
        self.pos = pos.ok_or_else(|| std::io::Error::other("seek before the start of the pak"))?;
        Ok(self.pos)
    }
}

#[cfg(test)]
mod selected_member_tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn zip_source_remembers_only_the_first_installed_member() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let mut writer = zip::ZipWriter::new(temp.reopen().unwrap());
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("first.pak", options.clone()).unwrap();
        writer.write_all(b"first source").unwrap();
        writer.start_file("second.pak", options).unwrap();
        writer.write_all(b"second source").unwrap();
        writer.finish().unwrap();

        let source = PakSource::open(fs_err::File::open(temp.path()).unwrap()).unwrap();
        assert_eq!(source.selected_member(), Some("first.pak"));
        let mut payload = Vec::new();
        source.reader().read_to_end(&mut payload).unwrap();
        assert_eq!(payload, b"first source");
    }
}
