#[cfg(test)]
mod lint_workflow_tests {
    use super::super::{LintRun, LintScope, choose_mods, focus_only, save_txt};
    use crate::mod_lints::LintReport;
    use crate::providers::ModSpecification;
    use std::collections::BTreeSet;
    use std::fs;
    fn spec(name: &str) -> ModSpecification {
        ModSpecification::new(name.to_owned())
    }

    #[test]
    fn scope_selection_and_focus() {
        let a = spec("a");
        let b = spec("b");
        let c = spec("c");
        let available = vec![a.clone(), b.clone(), c.clone(), a.clone()];
        let selected = BTreeSet::from([b.clone()]);
        let (mods, focus) = choose_mods(&available, LintScope::All, &selected, None).unwrap();
        assert_eq!(mods.len(), 3);
        assert!(focus.is_none());
        let (mods, _) = choose_mods(&available, LintScope::Selected, &selected, None).unwrap();
        assert_eq!(mods, vec![b.clone()]);
        assert!(choose_mods(&available, LintScope::Selected, &BTreeSet::new(), None).is_err());
        let (mods, focus) = choose_mods(&available, LintScope::Focus, &selected, Some(&a)).unwrap();
        assert_eq!(mods.len(), 3);
        assert_eq!(focus, Some(a.clone()));
        assert!(
            choose_mods(
                &available,
                LintScope::Focus,
                &selected,
                Some(&spec("missing"))
            )
            .is_err()
        );
    }

    #[test]
    fn focus_retains_only_related_conflicts_and_mod_issues() {
        use indexmap::IndexSet;
        let a = spec("a");
        let b = spec("b");
        let c = spec("c");
        let mut report = LintReport::default();
        report.conflicting_mods = Some(
            [
                (
                    "a.uasset".to_owned(),
                    IndexSet::from_iter([a.clone(), b.clone()]),
                ),
                (
                    "c.uasset".to_owned(),
                    IndexSet::from_iter([b.clone(), c.clone()]),
                ),
            ]
            .into(),
        );
        report.empty_archive_mods = Some(BTreeSet::from([a.clone(), c.clone()]));
        focus_only(&mut report, &a);
        assert_eq!(report.conflicting_mods.unwrap().len(), 1);
        assert_eq!(report.empty_archive_mods.unwrap(), BTreeSet::from([a]));
    }

    #[test]
    fn text_report_is_written_even_without_findings() {
        let dir = tempfile::tempdir().unwrap();
        let run = LintRun {
            profile: "test profile".into(),
            scope: LintScope::All,
            mods: vec![spec("a")],
            focus: None,
            checks: vec!["conflicting".into()],
        };
        let report = LintReport::default();
        let path1 = save_txt(dir.path(), &report, &run).unwrap();
        let path2 = save_txt(dir.path(), &report, &run).unwrap();
        assert_ne!(path1, path2);
        let text = fs::read_to_string(path1).unwrap();
        assert!(text.contains("Profile: test profile"));
        assert!(text.contains("SUMMARY: 0 entries"));
    }
}
