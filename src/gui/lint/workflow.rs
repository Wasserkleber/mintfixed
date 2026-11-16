//! Mod scan selection and focus filtering (no GUI widgets).
use std::collections::BTreeSet;

use crate::mod_lints::LintReport;
use crate::providers::ModSpecification;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::gui) enum LintScope {
    #[default]
    All,
    Selected,
    Focus,
}

impl LintScope {
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All enabled mods",
            Self::Selected => "Selected enabled mods",
            Self::Focus => "One mod vs all enabled mods",
        }
    }
}

pub(in crate::gui) struct LintRun {
    pub profile: String,
    pub scope: LintScope,
    pub mods: Vec<ModSpecification>,
    pub focus: Option<ModSpecification>,
    pub checks: Vec<String>,
}

pub(in crate::gui) fn choose_mods(
    available: &[ModSpecification],
    scope: LintScope,
    selected: &BTreeSet<ModSpecification>,
    focus: Option<&ModSpecification>,
) -> Result<(Vec<ModSpecification>, Option<ModSpecification>), &'static str> {
    // Ensure identical specs (e.g. a mod present in several groups) are not linted twice.
    let mut seen = BTreeSet::new();
    let available: Vec<_> = available
        .iter()
        .filter(|s| seen.insert((*s).clone()))
        .cloned()
        .collect();
    match scope {
        LintScope::All if !available.is_empty() => Ok((available, None)),
        LintScope::Selected => {
            let included: Vec<_> = available
                .into_iter()
                .filter(|s| selected.contains(s))
                .collect();
            if included.is_empty() {
                Err("Select at least one enabled mod")
            } else {
                Ok((included, None))
            }
        }
        LintScope::Focus => {
            let anchor = focus.ok_or("Choose a focus mod")?;
            if !available.contains(anchor) {
                return Err("Focus mod is no longer enabled in this profile");
            }
            if available.len() < 2 {
                return Err("At least two enabled mods are needed for comparison");
            }
            Ok((available, Some(anchor.clone())))
        }
        LintScope::All => Err("No enabled mods in this profile"),
    }
}

/// Focus mode compares the selected mod against every enabled peer, but suppresses
/// unrelated results for other mods. Peer names remain in conflict entries.
pub(in crate::gui) fn focus_only(report: &mut LintReport, anchor: &ModSpecification) {
    if let Some(entries) = &mut report.conflicting_mods {
        entries.retain(|_, modifiers| modifiers.contains(anchor));
    }
    macro_rules! keep_only_anchor {
        ($($field:ident),* $(,)?) => { $(
            if let Some(entries) = &mut report.$field {
                entries.retain(|mod_spec, _| mod_spec == anchor);
            }
        )* };
    }
    keep_only_anchor!(
        asset_register_bin_mods,
        shader_file_mods,
        outdated_pak_version_mods,
        non_asset_file_mods,
        split_asset_pairs_mods,
        unmodified_game_assets_mods,
    );
    if let Some(entries) = &mut report.empty_archive_mods {
        entries.retain(|s| s == anchor);
    }
    if let Some(entries) = &mut report.archive_with_only_non_pak_files_mods {
        entries.retain(|s| s == anchor);
    }
    if let Some(entries) = &mut report.archive_with_multiple_paks_mods {
        entries.retain(|s| s == anchor);
    }
}
