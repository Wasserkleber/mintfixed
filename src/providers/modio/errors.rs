use super::{ProviderError, Snafu, modio};

/// Decides whether a mod.io response means "this mod is gone" (deleted,
/// hidden or made private), as opposed to a problem with the request or the
/// account. 404 and 410 always mean the resource is gone. mod.io also answers
/// 403 for a mod it no longer shows, but 403 can mean missing API permissions
/// too, so a 403 only counts when mod.io's own message says the mod was not
/// found or was deleted ("The mod ID you have included in your request could
/// not be found."). Everything else, including auth and network errors, stays
/// a real error so nothing is ever dropped on a transient failure.
pub(super) fn unavailable_modio_status(status: Option<u16>, message: &str) -> bool {
    match status {
        Some(404 | 410) => true,
        Some(403) => {
            let message = message.to_ascii_lowercase();
            message.contains("could not be found") || message.contains("has been deleted")
        }
        _ => false,
    }
}

fn unavailable_modio_response(error: &modio::Error) -> bool {
    // The display text carries mod.io's error message after the status line.
    unavailable_modio_status(
        error.status().map(|status| status.as_u16()),
        &error.to_string(),
    )
}

/// Only suppress errors representing an unavailable *specific mod*, never
/// OAuth errors, rate limits, network failures, or general API failures.
pub(crate) fn is_unavailable_error(error: &ProviderError) -> bool {
    match error {
        ProviderError::NoModsForNameId { .. } => true,
        ProviderError::DrgModioError { source } => match source {
            DrgModioError::FetchModFilesFailed { source, .. }
            | DrgModioError::FetchModFailed { source, .. }
            | DrgModioError::FetchModFileFailed { source, .. }
            | DrgModioError::FetchDependenciesFailed { source, .. } => {
                unavailable_modio_response(source)
            }
            DrgModioError::GenericError { msg } => {
                *msg == "mod.io returned no mod for the requested ID"
                    || *msg == "mod.io returned no mod file for the requested ID"
            }
            _ => false,
        },
        _ => false,
    }
}

#[derive(Debug, Snafu)]
#[snafu(visibility(pub(super)))]
pub enum DrgModioError {
    #[snafu(display("missing OAuth token"))]
    MissingOauthToken,
    #[snafu(display("mod.io error: {source}"))]
    GenericModioError { source: modio::Error },
    #[snafu(display("failed to perform basic mod.io probe: {source}"))]
    CheckFailed { source: modio::Error },
    #[snafu(display("failed to fetch mod files for <{url}> (mod_id = {mod_id}): {source}"))]
    FetchModFilesFailed {
        source: modio::Error,
        url: String,
        mod_id: u32,
    },
    #[snafu(display(
        "failed to fetch mod file {modfile_id} for <{url}> (mod_id = {mod_id}): {source}"
    ))]
    FetchModFileFailed {
        source: modio::Error,
        url: String,
        mod_id: u32,
        modfile_id: u32,
    },
    #[snafu(display("failed to fetch mod <{url}> (mod_id = {mod_id}): {source}"))]
    FetchModFailed {
        source: modio::Error,
        url: String,
        mod_id: u32,
    },
    #[snafu(display("failed to fetch dependencies for mod <{url}> (mod_id = {mod_id}): {source}"))]
    FetchDependenciesFailed {
        source: modio::Error,
        url: String,
        mod_id: u32,
    },
    #[snafu(display("encountered mod.io-related error: {msg}"))]
    GenericError { msg: &'static str },
}

impl DrgModioError {
    pub fn opt_mod_id(&self) -> Option<u32> {
        match self {
            DrgModioError::FetchModFilesFailed { mod_id, .. }
            | DrgModioError::FetchModFileFailed { mod_id, .. }
            | DrgModioError::FetchModFailed { mod_id, .. }
            | DrgModioError::FetchDependenciesFailed { mod_id, .. } => Some(*mod_id),
            _ => None,
        }
    }
}
