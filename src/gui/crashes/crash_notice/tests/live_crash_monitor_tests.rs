use super::*;
use std::collections::HashSet;

fn report(dir: &Path, name: &str, reason: &str) {
    std::fs::write(
        dir.join(name),
        format!("=== BPCrashHandler report ===\nreason  : {reason}\n\nassert  : test error\n"),
    )
    .unwrap();
}

#[test]
fn watches_only_new_crashes_and_ignores_self_tests() {
    let dir = tempfile::tempdir().unwrap();
    let seen = dir.path().join("ack.txt");
    report(
        dir.path(),
        "bpcrash_20261111_110000_1_01_t1.txt",
        "first-chance exception #1",
    );
    let mut known: HashSet<String> = report_paths(dir.path())
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert!(find_new_report(dir.path(), &seen, &mut known).is_none());

    report(
        dir.path(),
        "bpcrash_20261111_110500_1_02_t1.txt",
        "load-time self test (armed)",
    );
    assert!(find_new_report(dir.path(), &seen, &mut known).is_none());
    report(
        dir.path(),
        "bpcrash_20261111_110600_1_03_t1.txt",
        "first-chance exception #1",
    );
    let found = find_new_report(dir.path(), &seen, &mut known).unwrap();
    assert!(found.name.contains("110600"));
    assert_eq!(found.detail.as_deref(), Some("test error"));
    assert!(find_new_report(dir.path(), &seen, &mut known).is_none());
}

#[test]
fn incomplete_reports_are_retried() {
    let dir = tempfile::tempdir().unwrap();
    let seen = dir.path().join("ack.txt");
    let mut known = HashSet::new();
    let name = "bpcrash_20261111_120000_1_01_t1.txt";
    std::fs::write(dir.path().join(name), "=== BPCrashHandler report ===\n").unwrap();
    assert!(find_new_report(dir.path(), &seen, &mut known).is_none());
    assert!(!known.contains(name));
    report(dir.path(), name, "UE fatal crash");
    assert!(find_new_report(dir.path(), &seen, &mut known).is_some());
}
