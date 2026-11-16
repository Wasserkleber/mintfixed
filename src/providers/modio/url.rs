use super::*;

static RE_MOD: OnceLock<regex::Regex> = OnceLock::new();
fn re_mod() -> &'static regex::Regex {
    RE_MOD.get_or_init(|| regex::Regex::new("^https://mod\\.io/g/drg/m/(?P<name_id>[^/#]+)/?(?:#(?:(?P<mod_id>\\d+)(?:/(?P<modfile_id>\\d+))?|[a-z]+))?$").unwrap())
}

pub(super) fn parse_url(url: &str) -> Result<ModIoModUrl<'_>, ProviderError> {
    let captures = re_mod().captures(url).context(InvalidUrlSnafu {
        url: url.to_string(),
    })?;

    Ok(ModIoModUrl {
        name_id: captures.name("name_id").unwrap().as_str(),
        mod_id: captures
            .name("mod_id")
            .and_then(|m| m.as_str().parse().ok()),
        modfile_id: captures
            .name("modfile_id")
            .and_then(|m| m.as_str().parse().ok()),
    })
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(super) struct ModIoModUrl<'n> {
    pub(super) name_id: &'n str,
    pub(super) mod_id: Option<u32>,
    pub(super) modfile_id: Option<u32>,
}

/// Validate bundled mod.io links with the provider's normal parser.
pub(crate) fn is_valid_modio_url(url: &str) -> bool {
    parse_url(url).is_ok()
}

/// A DRG mod.io mod and its exact file. A pin carries both IDs even offline.
pub(crate) fn pinned_file(url: &str) -> Option<(u32, u32)> {
    let parsed = parse_url(url).ok()?;
    Some((parsed.mod_id?, parsed.modfile_id?))
}

pub(crate) fn cached_file(url: &str, cache: &ProviderCache) -> Option<(u32, u32)> {
    let parsed = parse_url(url).ok()?;
    if let (Some(id), Some(file)) = (parsed.mod_id, parsed.modfile_id) {
        return Some((id, file));
    }
    let cache = cache.read().unwrap();
    let cache = cache.get::<ModioCache>(MODIO_PROVIDER_ID)?;
    let id = parsed
        .mod_id
        .or_else(|| cache.mod_id_map.get(parsed.name_id).copied())?;
    Some((id, cache.mods.get(&id)?.latest_modfile?))
}

pub(super) fn format_spec(name_id: &str, mod_id: u32, file_id: Option<u32>) -> ModSpecification {
    ModSpecification::new(if let Some(file_id) = file_id {
        format!("https://mod.io/g/drg/m/{name_id}#{mod_id}/{file_id}")
    } else {
        format!("https://mod.io/g/drg/m/{name_id}#{mod_id}")
    })
}
