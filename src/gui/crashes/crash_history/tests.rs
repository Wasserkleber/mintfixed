use super::*;

fn write_report(dir: &Path, name: &str, reason: &str) {
    fs::write(
        dir.join(name),
        format!(
            "=== BPCrashHandler report ===\nreason  : {reason}\n\n--- native context ---\n"
        ),
    )
    .unwrap();
}

#[test]
fn crash_history_excludes_self_tests_and_sorts_newest_first() {
    let tmp = tempfile::tempdir().unwrap();
    write_report(
        tmp.path(),
        "bpcrash_20261107_090400_1_01_t1.txt",
        "load-time self test (armed)",
    );
    write_report(
        tmp.path(),
        "bpcrash_20261107_090509_1_03_t1.txt",
        "first-chance exception #1",
    );
    write_report(
        tmp.path(),
        "bpcrash_20261107_115333_2_03_t2.txt",
        "UE fatal crash",
    );
    fs::write(tmp.path().join("bpcrash_status.txt"), "loaded").unwrap();
    let scan = scan_reports(tmp.path(), &ReportCache::new()).unwrap();
    assert_eq!(scan.cache.len(), 3);
    let reports = scan.reports;
    assert_eq!(reports.len(), 2);
    assert_eq!(reports[0].timestamp, "2026-11-07 11:53:33");
    assert!(reports[1].name.contains("090509"));
}

#[test]
fn refresh_reuses_classifications_and_handles_changed_and_deleted_files() {
    let tmp = tempfile::tempdir().unwrap();
    let old = "bpcrash_20261107_090400_1_01_t1.txt";
    let changed = "bpcrash_20261107_090509_1_03_t1.txt";
    let unchanged = "bpcrash_20261107_115333_2_03_t2.txt";
    write_report(tmp.path(), old, "UE fatal crash");
    write_report(tmp.path(), changed, "load-time self test");
    write_report(tmp.path(), unchanged, "load-time self test");
    let mut first = scan_reports(tmp.path(), &ReportCache::new()).unwrap();
    assert_eq!(first.reports.len(), 1);
    assert_eq!(first.cache.len(), 3);

    // Substitute a known classification to prove that unchanged files
    // use their cached value rather than opening and parsing the header.
    first
        .cache
        .get_mut(&tmp.path().join(unchanged))
        .unwrap()
        .kind = ReportKind::Crash;
    fs::remove_file(tmp.path().join(old)).unwrap();
    write_report(tmp.path(), changed, "UE fatal crash");
    let next = scan_reports(tmp.path(), &first.cache).unwrap();
    assert_eq!(next.reports.len(), 2);
    assert_eq!(next.reports[0].name, unchanged);
    assert_eq!(next.reports[1].name, changed);
    assert!(!next.cache.contains_key(&tmp.path().join(old)));
}

#[test]
fn incomplete_reports_are_retried() {
    let tmp = tempfile::tempdir().unwrap();
    let name = "bpcrash_20261107_090400_1_01_t1.txt";
    fs::write(tmp.path().join(name), "=== BPCrashHandler report ===\n").unwrap();
    let first = scan_reports(tmp.path(), &ReportCache::new()).unwrap();
    assert!(first.cache.is_empty());
    assert!(first.reports.is_empty());
    write_report(tmp.path(), name, "UE fatal crash");
    let next = scan_reports(tmp.path(), &first.cache).unwrap();
    assert_eq!(next.reports.len(), 1);
    assert_eq!(next.cache.len(), 1);
}

fn age_report(path: &Path) {
    fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_secs(120))
        .unwrap();
}

#[test]
fn retention_keeps_fifty_crashes_and_two_self_tests_separately() {
    let tmp = tempfile::tempdir().unwrap();
    // All self-tests are newer than the crashes, simulating many clean
    // game starts. They must not evict any of the newest fifty crashes.
    for n in 0..55 {
        for (minute, reason) in [("00", "UE fatal crash"), ("01", "load-time self test")] {
            let name = format!("bpcrash_20261107_09{minute}{n:02}_1_01_t1.txt");
            write_report(tmp.path(), &name, reason);
            age_report(&tmp.path().join(name));
        }
    }
    let unrelated = tmp.path().join("bpcrash_status.txt");
    let wrong_header = tmp.path().join("bpcrash_20261101_090000_1_01_t1.txt");
    let incomplete = tmp.path().join("bpcrash_20261102_090000_1_01_t1.txt");
    let subdir = tmp.path().join("bpcrash_20261103_090000_1_01_t1.txt");
    let unknown_name = "bpcrash_20261104_090000_1_01_t1.txt";
    let unknown = tmp.path().join(unknown_name);
    fs::write(&unrelated, "status").unwrap();
    fs::write(&wrong_header, "not a crash report").unwrap();
    fs::write(&incomplete, "=== BPCrashHandler report ===\n").unwrap();
    fs::create_dir(&subdir).unwrap();
    write_report(tmp.path(), unknown_name, "diagnostic snapshot");
    age_report(&unknown);

    let scan = scan_reports(tmp.path(), &ReportCache::new()).unwrap();
    assert!(scan.cleanup_warning.is_none());
    assert_eq!(scan.cache.len(), 52);
    assert_eq!(scan.reports.len(), 50);
    for n in 0..55 {
        let crash = format!("bpcrash_20261107_0900{n:02}_1_01_t1.txt");
        let self_test = format!("bpcrash_20261107_0901{n:02}_1_01_t1.txt");
        assert_eq!(tmp.path().join(crash).exists(), n >= 5);
        assert_eq!(tmp.path().join(self_test).exists(), n >= 53);
    }
    assert!(unrelated.exists() && wrong_header.exists() && incomplete.exists());
    assert!(subdir.is_dir() && unknown.exists());
    let again = scan_reports(tmp.path(), &scan.cache).unwrap();
    assert_eq!(again.cache.len(), 52);
    assert_eq!(again.reports.len(), 50);
}

#[test]
fn self_test_cleanup_preserves_small_crash_history() {
    let tmp = tempfile::tempdir().unwrap();
    let crash = "bpcrash_20261107_090000_1_01_t1.txt";
    write_report(tmp.path(), crash, "first-chance exception #1");
    age_report(&tmp.path().join(crash));
    for n in 0..55 {
        let name = format!("bpcrash_20261108_0900{n:02}_1_01_t1.txt");
        write_report(tmp.path(), &name, "load-time self-test (armed)");
        age_report(&tmp.path().join(name));
    }
    let scan = scan_reports(tmp.path(), &ReportCache::new()).unwrap();
    assert!(scan.cleanup_warning.is_none());
    assert_eq!(scan.reports.len(), 1);
    assert_eq!(scan.reports[0].name, crash);
    assert_eq!(scan.cache.len(), 3);
    assert!(tmp.path().join(crash).exists());
    assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 3);
}

#[test]
fn pruning_defers_recent_or_changed_reports() {
    let tmp = tempfile::tempdir().unwrap();
    let name = "bpcrash_20261107_090400_1_01_t1.txt";
    write_report(tmp.path(), name, "UE fatal crash");
    let path = tmp.path().join(name);
    let current = file_stamp(&fs::metadata(&path).unwrap());
    assert!(!prune_report(&path, current).unwrap());
    assert!(path.exists());
    let old_time = SystemTime::now() - Duration::from_secs(120);
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(old_time)
        .unwrap();
    let old = file_stamp(&fs::metadata(&path).unwrap());
    write_report(tmp.path(), name, "another UE fatal crash");
    assert!(!prune_report(&path, old).unwrap());
    assert!(path.exists());
}

#[test]
fn malformed_timestamps_are_excluded() {
    assert!(report_timestamp("bpcrash_bad.txt").is_none());
    assert_eq!(
        report_timestamp("bpcrash_20261107_115333_2_03_t2.txt").as_deref(),
        Some("2026-11-07 11:53:33")
    );
}
