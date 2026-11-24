use std::path::PathBuf;

use crate::providers::*;

mod cache_cleanup;
mod cache_updates;
mod fetch;
mod local_files;
mod modio_status;
mod registry;
mod resolution;

pub struct ModStore {
    cache_path: PathBuf,
    providers: Providers,
    cache: ProviderCache,
    blob_cache: BlobCache,
    local_refresh: std::sync::Mutex<()>,
    local_files: RwLock<(u64, HashMap<String, [u8; 32]>)>,
}
