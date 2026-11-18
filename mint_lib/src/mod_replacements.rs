//! Build-time, mod.io replacement/alternative rules. Like assets/preinstall.json,
//! changes to assets/mod_replacements.json take effect after rebuilding mintfixed.
use crate::mod_info::ModSpecification;
use serde::Deserialize;
use std::{collections::HashSet, sync::OnceLock};

const EMBEDDED_RULES: &str = include_str!("../../assets/mod_replacements.json");

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplacementRule {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub original: String,
    pub original_name: String,
    pub replacement: String,
    pub replacement_name: String,
    #[serde(default)]
    pub alternatives: Vec<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub dependency_warning: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    replacements: Vec<ReplacementRule>,
}
fn default_true() -> bool {
    true
}

fn same_mod(left: &str, right: &str) -> bool {
    ModSpecification::new(left.into()).matches_identity(&ModSpecification::new(right.into()))
}
fn valid_modio(url: &str) -> bool {
    // Require canonical DRG mod.io URLs, optionally pinned. Do not accept
    // non-mod.io inputs, other games, URL prefixes, or URL query parameters.
    let Some(rest) = url.strip_prefix("https://mod.io/g/drg/m/") else {
        return false;
    };
    let (slug, version) = rest
        .split_once('#')
        .map_or((rest, None), |(s, v)| (s, Some(v)));
    !slug.is_empty()
        && !slug.contains('/')
        && !slug.contains('?')
        && version.is_none_or(|v| !v.is_empty() && !v.contains('?'))
}
fn parse_json(raw: &str) -> Result<Vec<ReplacementRule>, String> {
    let mut config: Config = serde_json::from_str(raw).map_err(|error| error.to_string())?;
    if !config.enabled {
        return Ok(Vec::new());
    }
    config.replacements.retain(|rule| rule.enabled);
    let mut seen = HashSet::new();
    for rule in &config.replacements {
        if !valid_modio(&rule.original) || !valid_modio(&rule.replacement) {
            return Err(format!(
                "Invalid DRG mod.io URL in replacement rule for {}",
                rule.original
            ));
        }
        if rule.original_name.trim().is_empty() || rule.replacement_name.trim().is_empty() {
            return Err("Replacement rule names cannot be empty".into());
        }
        if same_mod(&rule.original, &rule.replacement) {
            return Err(format!("A mod cannot replace itself: {}", rule.original));
        }
        if !seen.insert(rule.original.clone()) {
            return Err(format!("Duplicate replacement source: {}", rule.original));
        }
        for alt in &rule.alternatives {
            if !valid_modio(alt) || same_mod(alt, &rule.original) {
                return Err(format!("Invalid alternative for {}: {alt}", rule.original));
            }
        }
    }
    // A chain or duplicate source would cause repeated prompts / ambiguity.
    for rule in &config.replacements {
        for other in &config.replacements {
            if !std::ptr::eq(rule, other) && same_mod(&rule.original, &other.original) {
                return Err(format!(
                    "Overlapping replacement sources: {} and {}",
                    rule.original, other.original
                ));
            }
            if same_mod(&rule.replacement, &other.original) {
                return Err(format!(
                    "Replacement chains are unsupported: {} -> {}",
                    rule.original, rule.replacement
                ));
            }
        }
    }
    Ok(config.replacements)
}

pub fn rules() -> &'static [ReplacementRule] {
    static RULES: OnceLock<Vec<ReplacementRule>> = OnceLock::new();
    RULES
        .get_or_init(|| {
            parse_json(EMBEDDED_RULES).unwrap_or_else(|error| {
                tracing::error!("Invalid embedded mod_replacements.json: {error}");
                Vec::new() // Fail closed rather than changing user mod selections.
            })
        })
        .as_slice()
}

/// Exact matching (including pinned releases) for configured source mods.
pub fn rule_for_original(spec: &ModSpecification) -> Option<&'static ReplacementRule> {
    rules()
        .iter()
        .find(|rule| same_mod(&spec.url, &rule.original))
}

/// Alternatives only satisfy the original dependency in the forward direction.
pub fn satisfies_alternative(installed: &ModSpecification, required: &ModSpecification) -> bool {
    rule_for_original(required).is_some_and(|rule| {
        same_mod(&installed.url, &rule.replacement)
            || rule
                .alternatives
                .iter()
                .any(|alt| same_mod(&installed.url, alt))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shipped_rules_are_valid() {
        parse_json(EMBEDDED_RULES).unwrap();
    }
    #[test]
    fn default_rule_is_forward_only_and_pins_match() {
        let installed = ModSpecification::new("https://mod.io/g/drg/m/unified-ui#123/456".into());
        let required = ModSpecification::new("https://mod.io/g/drg/m/mod-hub#567/891".into());
        if rule_for_original(&required).is_some() {
            assert!(satisfies_alternative(&installed, &required));
            assert!(!satisfies_alternative(&required, &installed));
            let emm = ModSpecification::new("https://mod.io/g/drg/m/emm".into());
            assert!(satisfies_alternative(&emm, &required));
        }
    }
    #[test]
    fn disabled_rules_are_empty_and_unknown_fields_are_rejected() {
        assert!(
            parse_json(r#"{"enabled":false,"replacements":[]}"#)
                .unwrap()
                .is_empty()
        );
        assert!(parse_json(r#"{"replacements":[{"enabled":false,"original":"https://mod.io/g/drg/m/a","original_name":"A","replacement":"https://mod.io/g/drg/m/b","replacement_name":"B"}]}"#).unwrap().is_empty());
        assert!(parse_json(r#"{"enabled":true,"replacements":[],"typo":1}"#).is_err());
    }
    #[test]
    fn custom_rules_are_data_driven() {
        let custom = r#"{"replacements":[{"original":"https://mod.io/g/drg/m/old", "original_name":"Old", "replacement":"https://mod.io/g/drg/m/new", "replacement_name":"New"}]}"#;
        let rules = parse_json(custom).unwrap();
        assert_eq!(rules[0].replacement_name, "New");
        assert!(same_mod(
            &rules[0].original,
            "https://mod.io/g/drg/m/old#5/12"
        ));
    }
}
