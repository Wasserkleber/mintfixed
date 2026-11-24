use super::*;

fn write_report(dir: &Path, name: &str, reason: &str, detail: &str) {
    std::fs::write(
        dir.join(name),
        format!("=== BPCrashHandler report ===\nreason  : {reason}\n\nassert  : {detail}\n"),
    )
    .unwrap();
}

#[test]
fn self_tests_are_ignored_and_latest_crash_is_shown() {
    let temp = tempfile::tempdir().unwrap();
    let reports = temp.path().join("reporter");
    std::fs::create_dir(&reports).unwrap();
    write_report(
        &reports,
        "bpcrash_20261107_090400_1_01_t1.txt",
        "load-time self test (armed)",
        "",
    );
    write_report(
        &reports,
        "bpcrash_20261107_090509_1_02_t1.txt",
        "first-chance exception #1",
        "older error",
    );
    write_report(
        &reports,
        "bpcrash_20261107_115333_2_03_t2.txt",
        "first-chance exception #1",
        "missing parent",
    );
    let seen = temp.path().join("ack.txt");
    let latest = find_latest_unseen_report(&reports, &seen).unwrap();
    assert!(latest.name.contains("115333"));
    assert_eq!(latest.detail.as_deref(), Some("missing parent"));
    assert_eq!(latest.timestamp, "2026-11-07 11:53:33");
}

#[test]
fn acknowledged_crash_does_not_reappear_but_new_one_does() {
    let temp = tempfile::tempdir().unwrap();
    let reports = temp.path().join("reporter");
    std::fs::create_dir(&reports).unwrap();
    let first = "bpcrash_20261107_090509_1_03_t1.txt";
    write_report(&reports, first, "first-chance exception #1", "error");
    let seen = temp.path().join("ack.txt");
    let crash = find_latest_unseen_report(&reports, &seen).unwrap();
    std::fs::write(&seen, crash.path.to_string_lossy().as_bytes()).unwrap();
    assert!(find_latest_unseen_report(&reports, &seen).is_none());
    write_report(
        &reports,
        "bpcrash_20261107_100000_2_03_t2.txt",
        "first-chance exception #1",
        "new error",
    );
    assert!(find_latest_unseen_report(&reports, &seen).is_some());
}

#[test]
fn newest_crash_behind_self_tests_is_found() {
    let temp = tempfile::tempdir().unwrap();
    let reports = temp.path().join("reporter");
    std::fs::create_dir(&reports).unwrap();
    write_report(
        &reports,
        "bpcrash_20261116_120000_1_01_t1.txt",
        "first-chance exception #1",
        "older crash",
    );
    write_report(
        &reports,
        "bpcrash_20261116_235903_2_03_t2.txt",
        "first-chance exception #1",
        "newest crash",
    );
    for day in 17..=22 {
        for (n, reason) in [
            (1, "load-time self test (armed, engine not resolved yet)"),
            (2, "load-time self test (armed)"),
        ] {
            let name = format!("bpcrash_202611{day}_000000_{day}_0{n}_t1.txt");
            write_report(&reports, &name, reason, "");
        }
    }
    std::fs::write(reports.join("bpcrash_status.txt"), "not a report").unwrap();
    let seen = temp.path().join("ack.txt");
    let latest = find_latest_unseen_report(&reports, &seen).unwrap();
    assert!(latest.name.contains("235903"));
    assert_eq!(latest.detail.as_deref(), Some("newest crash"));
}
