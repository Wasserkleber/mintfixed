//! Shared dependency-expansion policy used by ModStore and mod.io cache refresh.
//! A request is attempted at most once per resolution pass; cached/resolved
//! canonical specs are also excluded.
use std::collections::HashSet;

use super::{ModInfo, ModSpecification};

pub(crate) fn pending_dependencies<'a>(
    infos: impl IntoIterator<Item = &'a ModInfo>,
    resolved: &HashSet<ModSpecification>,
    attempted: &HashSet<ModSpecification>,
) -> HashSet<ModSpecification> {
    infos
        .into_iter()
        .flat_map(|info| info.suggested_dependencies.iter())
        .filter(|spec| !resolved.contains(*spec) && !attempted.contains(*spec))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::ModResolution;
    use super::*;

    fn info(url: &str, dependencies: &[&str]) -> ModInfo {
        ModInfo {
            provider: "http",
            name: url.to_owned(),
            spec: ModSpecification::new(url.to_owned()),
            versions: vec![],
            resolution: ModResolution::resolvable(url.to_owned().into()),
            suggested_require: false,
            suggested_dependencies: dependencies
                .iter()
                .map(|v| ModSpecification::new((*v).to_owned()))
                .collect(),
            modio_tags: None,
            modio_id: None,
        }
    }

    #[test]
    fn cyclic_dependencies_are_not_reattempted() {
        let a = info("https://example.test/a", &["https://example.test/b"]);
        let b = info("https://example.test/b", &["https://example.test/a"]);
        let resolved = HashSet::from([a.spec.clone(), b.spec.clone()]);
        let attempted = resolved.clone();
        assert!(pending_dependencies([&a, &b], &resolved, &attempted).is_empty());
    }

    #[test]
    fn only_unseen_dependencies_are_queued_once() {
        let a = info(
            "https://example.test/a",
            &["https://example.test/b", "https://example.test/c"],
        );
        let b = info("https://example.test/b", &["https://example.test/c"]);
        let resolved = HashSet::from([a.spec.clone()]);
        let attempted = HashSet::from([a.spec.clone(), b.spec.clone()]);
        let result = pending_dependencies([&a, &b], &resolved, &attempted);
        assert_eq!(
            result,
            HashSet::from([ModSpecification::new("https://example.test/c".to_owned())])
        );
    }
}
