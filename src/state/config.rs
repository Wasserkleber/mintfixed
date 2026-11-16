use std::io::Write;
use std::path::Path;

use serde::de::DeserializeOwned;

use super::*;

pub trait ConfigTrait: std::fmt::Debug + Default + Serialize + DeserializeOwned {}
impl<T> ConfigTrait for T where T: std::fmt::Debug + Default + Serialize + DeserializeOwned {}

/// Wrapper around an object that is written to a file when dropped
#[derive(Debug)]
pub struct ConfigWrapper<C: ConfigTrait> {
    save_revision: std::sync::atomic::AtomicU64,
    path: Option<PathBuf>,
    config: C,
}

impl<C: ConfigTrait> ConfigWrapper<C> {
    /// Creates a new file backed config object that is automatically saved when dropped
    pub fn new<P: AsRef<Path>>(path: P, config: C) -> Self {
        Self {
            config,
            save_revision: Default::default(),
            path: Some(path.as_ref().to_path_buf()),
        }
    }

    /// Create wrapper that lives only in memory and has no file backing
    pub fn memory(config: C) -> Self {
        Self {
            save_revision: Default::default(),
            config,
            path: None,
        }
    }

    /// Try our best to ensure that the config written is complete to protect against partial
    /// or broken config writes if the tool crashes or is killed.
    ///
    /// This is achieved, best-effort, by writing to a temporary file then replacing the target file
    /// with the temporary file.
    ///
    /// See <https://stackoverflow.com/questions/70362352/atomic-file-create-write>.
    pub fn save(&self) -> Result<(), StateError> {
        if let Some(final_path) = &self.path {
            write_atomic(final_path, &self.config)?;
        }
        self.save_revision
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    pub fn revision(&self) -> u64 {
        self.save_revision
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl<C: ConfigTrait> std::ops::Deref for ConfigWrapper<C> {
    type Target = C;
    fn deref(&self) -> &Self::Target {
        &self.config
    }
}

impl<C: ConfigTrait> std::ops::DerefMut for ConfigWrapper<C> {
    fn deref_mut(&mut self) -> &mut C {
        &mut self.config
    }
}

impl<C: ConfigTrait> Drop for ConfigWrapper<C> {
    fn drop(&mut self) {
        self.save().unwrap();
    }
}

/// Sync the complete temporary file before atomically replacing the destination.
pub(super) fn write_atomic(path: &Path, value: &impl Serialize) -> Result<(), StateError> {
    let mut file = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
    file.write_all(&serde_json::to_vec_pretty(value).context(CfgSerializationFailedSnafu)?)
        .context(CfgSaveFailedSnafu)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}

#[obake::versioned]
#[obake(version("0.0.0"))]
#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub provider_parameters: HashMap<String, HashMap<String, String>>,
    pub drg_pak_path: Option<PathBuf>,
    /// When configured, newly discovered local mods are added to the active profile.
    #[serde(default)]
    pub local_mod_folder: Option<PathBuf>,
    pub gui_theme: Option<GuiTheme>,
    pub sorting_config: Option<SortingConfig>,
    // notag's config "0.1.0" adds these two; kept (unused here) so a notag install gets its settings back.
    #[serde(default = "default_true")]
    pub confirm_deletion: bool,
    #[serde(default = "default_true")]
    pub show_changelog: bool,
    /// Install and autolaunch the pinned BP-Crash-Reporter on integrate.
    #[serde(default = "default_true")]
    pub bundled_crash_reporter: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SortingConfig {
    pub sort_category: SortBy,
    pub is_ascending: bool,
}

impl Default for SortingConfig {
    fn default() -> Self {
        Self {
            sort_category: SortBy::Enabled,
            is_ascending: true,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "version")]
pub enum VersionAnnotatedConfig {
    // notag's "0.1.0" is this layout plus the two flags above; read it too. We still write our own "0.0.0".
    #[serde(rename = "0.0.0", alias = "0.1.0")]
    V0_0_0(Config!["0.0.0"]),
    #[serde(other)]
    Unsupported,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MaybeVersionedConfig {
    Versioned(VersionAnnotatedConfig),
    Legacy(Config!["0.0.0"]),
}

impl Default for MaybeVersionedConfig {
    fn default() -> Self {
        MaybeVersionedConfig::Versioned(Default::default())
    }
}

impl Default for VersionAnnotatedConfig {
    fn default() -> Self {
        VersionAnnotatedConfig::V0_0_0(Default::default())
    }
}

impl Deref for VersionAnnotatedConfig {
    type Target = Config!["0.0.0"];

    fn deref(&self) -> &Self::Target {
        match self {
            VersionAnnotatedConfig::V0_0_0(cfg) => cfg,
            VersionAnnotatedConfig::Unsupported => unreachable!(),
        }
    }
}

impl DerefMut for VersionAnnotatedConfig {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            VersionAnnotatedConfig::V0_0_0(cfg) => cfg,
            VersionAnnotatedConfig::Unsupported => unreachable!(),
        }
    }
}

impl Default for Config!["0.0.0"] {
    fn default() -> Self {
        Self {
            provider_parameters: Default::default(),
            drg_pak_path: DRGInstallation::find()
                .as_ref()
                .map(DRGInstallation::main_pak),
            local_mod_folder: None,
            gui_theme: Some(GuiTheme::Dark),
            sorting_config: None,
            confirm_deletion: true,
            show_changelog: true,
            bundled_crash_reporter: true,
        }
    }
}

impl From<&VersionAnnotatedConfig> for MetaConfig {
    fn from(value: &VersionAnnotatedConfig) -> Self {
        MetaConfig {
            bundled_crash_reporter: value.bundled_crash_reporter,
        }
    }
}
