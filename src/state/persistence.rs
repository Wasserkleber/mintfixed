use super::*;

pub struct State {
    pub dirs: Dirs,
    pub config: ConfigWrapper<VersionAnnotatedConfig>,
    pub mod_data: ConfigWrapper<VersionAnnotatedModData>,
    pub store: Arc<ModStore>,
}

impl State {
    /// Known version conflicts, including disabled profile entries.
    pub fn profile_version_warnings(&self, profile: &str) -> Vec<String> {
        crate::mod_identity::ProfileIssues::collect(&self.mod_data, profile, &self.store).versions
    }

    pub fn init(dirs: Dirs) -> Result<Self, StateError> {
        let config_path = dirs.config_dir.join("config.json");

        let config = read_config_or_default(&config_path)?;
        let config = ConfigWrapper::<VersionAnnotatedConfig>::new(&config_path, config);
        config.save().unwrap();

        let legacy_mod_profiles_path = dirs.config_dir.join("profiles.json");
        let mod_data_path = dirs.config_dir.join("mod_data.json");
        // Apply embedded defaults only for a genuinely new mod list.
        let fresh = !mod_data_path.exists() && !legacy_mod_profiles_path.exists();
        let mut mod_data = read_mod_data_or_default(&mod_data_path, legacy_mod_profiles_path)?;
        if fresh {
            match &mut mod_data {
                VersionAnnotatedModData::V0_1_0(data) | VersionAnnotatedModData::V0_3_0(data) => {
                    crate::preinstall::apply(data)
                        .map_err(|reason| StateError::InvalidBundledPreset { reason })?;
                }
                VersionAnnotatedModData::V0_0_0(_) => unreachable!("mod data should be upgraded"),
            }
        }
        if fresh {
            config::write_atomic(&mod_data_path, &mod_data)?;
        }
        let mod_data = ConfigWrapper::<VersionAnnotatedModData>::new(mod_data_path, mod_data);
        // The loader commits migrations before creating an auto-saving wrapper.

        let store = ModStore::new(&dirs.cache_dir, &config.provider_parameters)?.into();

        Ok(Self {
            dirs,
            config,
            mod_data,
            store,
        })
    }
}

fn read_config_or_default(config_path: &PathBuf) -> Result<VersionAnnotatedConfig, StateError> {
    Ok(match fs::read(config_path) {
        Ok(buf) => {
            let config = serde_json::from_slice::<MaybeVersionedConfig>(&buf)
                .context(CfgDeserializationFailedSnafu)?;
            match config {
                MaybeVersionedConfig::Versioned(v) => match v {
                    VersionAnnotatedConfig::V0_0_0(v) => VersionAnnotatedConfig::V0_0_0(v),
                    VersionAnnotatedConfig::Unsupported => UnsupportedCfgVersionSnafu.fail()?,
                },
                MaybeVersionedConfig::Legacy(legacy) => {
                    VersionAnnotatedConfig::V0_0_0(Config_v0_0_0 {
                        provider_parameters: legacy.provider_parameters,
                        drg_pak_path: legacy.drg_pak_path,
                        ..Default::default()
                    })
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => VersionAnnotatedConfig::default(),
        Err(e) => Err(e)?,
    })
}

fn read_mod_data_or_default(
    mod_data_path: &std::path::Path,
    legacy_mod_profiles_path: PathBuf,
) -> Result<VersionAnnotatedModData, StateError> {
    migration::load(mod_data_path, &legacy_mod_profiles_path)
}
