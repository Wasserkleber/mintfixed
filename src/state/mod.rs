//! Persistent state models, profile resolution, and safe startup/loading.
//!
//! Keep the public `state::*` API stable; implementations live in focused modules.
pub mod config;
mod groups;
mod migration;
mod models;
mod operations;
mod persistence;
mod profiles;
mod validation;

pub use self::config::*;
pub use self::models::*;
pub use self::operations::{InstallEntry, SelectionError};
pub use self::persistence::State;

/// Persisted identity; independent of names, list positions and mod URLs.
pub fn new_id() -> String {
    format!("{:032x}", rand::random::<u128>())
}

use std::{
    collections::{BTreeMap, HashMap},
    ops::{Deref, DerefMut},
    path::PathBuf,
    sync::Arc,
};

use fs_err as fs;
use serde::{Deserialize, Serialize};
use snafu::prelude::*;

use crate::{
    Dirs,
    gui::GuiTheme,
    providers::{ModSpecification, ModStore},
};
use crate::{gui::SortBy, providers::ProviderError};
use mint_lib::{DRGInstallation, mod_info::MetaConfig};

fn default_true() -> bool {
    true
}

fn is_zero(value: &i32) -> bool {
    *value == 0
}

#[derive(Debug, Snafu)]
pub enum StateError {
    #[snafu(display("failed to deserialize user config"))]
    CfgDeserializationFailed { source: serde_json::Error },
    #[snafu(display("unsupported config version"))]
    UnsupportedCfgVersion,
    #[snafu(display("failed to read config.json"))]
    CfgReadFailed { source: std::io::Error },
    #[snafu(display("failed to save config"))]
    CfgSaveFailed { source: std::io::Error },
    #[snafu(display("failed to write configuration at {}: {source}", path.display()))]
    CfgWriteAtPathFailed {
        path: PathBuf,
        source: Box<StateError>,
    },
    #[snafu(display("failed to serialize user config"))]
    CfgSerializationFailed { source: serde_json::Error },
    #[snafu(transparent)]
    IoError { source: std::io::Error },
    #[snafu(transparent)]
    PersistError { source: tempfile::PersistError },
    #[snafu(transparent)]
    ProviderError { source: ProviderError },
    #[snafu(display("failed to deserialize mod data"))]
    ModDataDeserializationFailed { source: serde_json::Error },
    #[snafu(display("unsupported mod data version: {version}"))]
    UnsupportedModDataVersion { version: String },
    #[snafu(display("invalid mod data: {reason}"))]
    InvalidModData { reason: String },
    #[snafu(display("failed to deserialize legacy profiles"))]
    LegacyProfilesDeserializationFailed { source: serde_json::Error },
    #[snafu(display("invalid bundled mod.io preset: {reason}"))]
    InvalidBundledPreset { reason: String },
}
