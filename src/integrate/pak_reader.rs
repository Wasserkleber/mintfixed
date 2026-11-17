// Mechanical source extraction from src/integrate.rs; no algorithm changes.
use super::*;

pub(crate) fn get_pak_from_data(
    mut data: Box<dyn ReadSeek>,
) -> Result<Box<dyn ReadSeek>, IntegrationError> {
    if let Ok(mut archive) = zip::ZipArchive::new(&mut data) {
        (0..archive.len())
            .map(|i| -> Result<Option<Box<dyn ReadSeek>>, IntegrationError> {
                let mut file = archive
                    .by_index(i)
                    .map_err(|_| IntegrationError::GenericError {
                        msg: "failed to extract file in zip archive".to_string(),
                    })?;
                match file.enclosed_name() {
                    Some(p) => {
                        if file.is_file() && p.extension() == Some(std::ffi::OsStr::new("pak")) {
                            let mut buf = vec![];
                            file.read_to_end(&mut buf)?;
                            Ok(Some(Box::new(Cursor::new(buf))))
                        } else {
                            Ok(None)
                        }
                    }
                    None => Ok(None),
                }
            })
            .find_map(Result::transpose)
            .context(GenericSnafu {
                msg: "zip archive does not contain pak",
            })?
    } else {
        data.rewind()?;
        Ok(data)
    }
}
