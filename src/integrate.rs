mod blueprint_guard;
mod bundle_writer;

use self::bundle_writer::ModBundleWriter;
mod asset_paths;
mod assets;
mod orchestration;
mod pak_reader;
mod patches;
mod uninstall;

use self::asset_paths::supported_registry_asset_path;
use self::assets::{collect_dir_files, format_soft_class};
pub use self::orchestration::integrate;
pub(crate) use self::orchestration::integrate_with_report;
pub(crate) use self::pak_reader::get_pak_from_data;
use self::patches::{
    hook_pcb, patch, patch_modding_tab, patch_modding_tab_item, patch_server_list_entry,
};
pub use self::uninstall::uninstall;

use std::collections::{HashMap, HashSet};
use std::io::{BufReader, BufWriter, Cursor, ErrorKind, Read, Seek, Write};
use std::path::{Path, PathBuf};

use fs_err as fs;

use serde::Deserialize;
use snafu::{Whatever, prelude::*};
use tracing::{info, warn};
use uasset_utils::asset_registry::{AssetRegistry, Readable as _, Writable as _};
use uasset_utils::paths::{PakPath, PakPathComponentTrait};
use uasset_utils::splice::{
    AssetVersion, TrackedStatement, extract_tracked_statements, inject_tracked_statements, walk,
};
use unreal_asset::AssetBuilder;
use unreal_asset::engine_version::EngineVersion;

use crate::mod_lints::LintError;
use crate::providers::{ModInfo, ProviderError, ReadSeek};
use mint_lib::DRGInstallation;
use mint_lib::mod_info::MetaConfig;

use unreal_asset::{
    Asset,
    exports::ExportBaseTrait,
    flags::EObjectFlags,
    kismet::{
        EExprToken, ExByteConst, ExCallMath, ExLet, ExLetObj, ExLocalVariable, ExRotationConst,
        ExSelf, ExSoftObjectConst, ExStringConst, ExVectorConst, FieldPath, KismetPropertyPointer,
    },
    kismet::{ExFalse, KismetExpression},
    types::PackageIndex,
    types::vector::Vector,
};

static INTEGRATION_DIR: include_dir::Dir<'_> =
    include_dir::include_dir!("$CARGO_MANIFEST_DIR/assets/integration");

#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum IntegrationError {
    #[snafu(display("unable to determine DRG installation at provided path {}", path.display()))]
    DrgInstallationNotFound { path: PathBuf },
    #[snafu(transparent)]
    IoError { source: std::io::Error },
    #[snafu(transparent)]
    RepakError { source: repak::Error },
    #[snafu(transparent)]
    UnrealAssetError { source: unreal_asset::Error },
    #[snafu(display("mod {:?}: I/O error encountered during its processing", mod_info.name))]
    CtxtIoError {
        source: std::io::Error,
        mod_info: Box<ModInfo>,
    },
    #[snafu(display("mod {:?}: repak error encountered during its processing", mod_info.name))]
    CtxtRepakError {
        source: repak::Error,
        mod_info: Box<ModInfo>,
    },
    #[snafu(display(
        "mod {:?}: modfile {} contains unexpected prefix",
        mod_info.name,
        modfile_path
    ))]
    ModfileInvalidPrefix {
        mod_info: Box<ModInfo>,
        modfile_path: String,
    },
    #[snafu(display(
        "mod {:?}: invalid PAK asset path '{}' (mount point '{}'). The asset cannot be mapped to an Unreal game path. DRG game assets normally belong under 'FSD/Content/...' (e.g. 'FSD/Content/Enemies/...'). The mod may have been packed with an incorrect mount point; repackage it or contact the author. This is not a missing DRG installation path.",
        mod_info.name,
        asset_path,
        mount_point,
    ))]
    ModfileInvalidAssetPath {
        mod_info: Box<ModInfo>,
        asset_path: String,
        mount_point: String,
    },
    #[snafu(display(
        "mod {:?}: failed to integrate: {source}",
        mod_info.name,
    ))]
    CtxtGenericError {
        source: Box<dyn std::error::Error + Send + Sync>,
        mod_info: Box<ModInfo>,
    },
    #[snafu(transparent)]
    ProviderError { source: ProviderError },
    #[snafu(display("integration error: {msg}"))]
    GenericError { msg: String },
    #[snafu(transparent)]
    JoinError { source: tokio::task::JoinError },
    #[snafu(transparent)]
    LintError { source: LintError },
    #[snafu(display("self update failed: {source:?}"))]
    SelfUpdateFailed {
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

impl IntegrationError {
    pub fn opt_mod_id(&self) -> Option<u32> {
        match self {
            IntegrationError::CtxtIoError { mod_info, .. }
            | IntegrationError::CtxtRepakError { mod_info, .. }
            | IntegrationError::CtxtGenericError { mod_info, .. }
            | IntegrationError::ModfileInvalidPrefix { mod_info, .. }
            | IntegrationError::ModfileInvalidAssetPath { mod_info, .. } => mod_info.modio_id,
            IntegrationError::ProviderError { source } => source.opt_mod_id(),
            _ => None,
        }
    }
}
