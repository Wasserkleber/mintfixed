use super::*;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModioMod {
    pub(super) name_id: String,
    pub(super) name: String,
    pub(super) latest_modfile: Option<u32>,
    pub(super) modfiles: Vec<ModioFile>,
    pub(super) tags: HashSet<String>,
}

impl ModioMod {
    pub(super) fn new(mod_: modio::mods::Mod, files: Vec<modio::files::File>) -> Self {
        Self {
            name_id: mod_.name_id,
            name: mod_.name,
            latest_modfile: mod_.modfile.map(|f| f.id),
            modfiles: files.into_iter().map(ModioFile::new).collect(),
            tags: mod_.tags.into_iter().map(|t| t.name).collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModioModResponse {
    pub(super) id: u32,
}

impl From<modio::mods::Mod> for ModioModResponse {
    fn from(value: modio::mods::Mod) -> Self {
        Self { id: value.id }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModioFile {
    pub(super) id: u32,
    pub(super) date_added: u64,
    pub(super) version: Option<String>,
    pub(super) changelog: Option<String>,
}
impl ModioFile {
    pub(super) fn new(file: modio::files::File) -> Self {
        Self {
            id: file.id,
            date_added: file.date_added,
            version: file.version,
            changelog: file.changelog,
        }
    }
}

pub(super) fn process_modio_tags(set: &HashSet<String>) -> ModioTags {
    let qol = set.contains("QoL");
    let gameplay = set.contains("Gameplay");
    let audio = set.contains("Audio");
    let visual = set.contains("Visual");
    let framework = set.contains("Framework");
    let required_status = if set.contains("RequiredByAll") {
        RequiredStatus::RequiredByAll
    } else {
        RequiredStatus::Optional
    };
    let approval_status = if set.contains("Verified") || set.contains("Auto-Verified") {
        ApprovalStatus::Verified
    } else if set.contains("Approved") {
        ApprovalStatus::Approved
    } else {
        ApprovalStatus::Sandbox
    };
    // Basic heuristic to collect all the tags which begin with a number, like `1.38`.
    let versions = set
        .iter()
        .filter(|i| i.starts_with(char::is_numeric))
        .cloned()
        .collect::<BTreeSet<String>>();

    ModioTags {
        qol,
        gameplay,
        audio,
        visual,
        framework,
        versions,
        required_status,
        approval_status,
    }
}
