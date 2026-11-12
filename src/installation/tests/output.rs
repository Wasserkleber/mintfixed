use super::*;
use std::io::{BufWriter, Write};

#[test]
fn unfinished_install_keeps_previous_archive_and_removes_temporary_file() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("mods_P.pak");
    std::fs::write(&target, b"previous archive").unwrap();
    {
        let mut output = InstallOutput::new(&target).unwrap();
        output.file().write_all(b"incomplete archive").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"previous archive");
    }
    assert_eq!(std::fs::read(&target).unwrap(), b"previous archive");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn successful_install_replaces_archive_only_after_commit() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("mods_P.pak");
    std::fs::write(&target, b"previous archive").unwrap();
    let mut output = InstallOutput::new(&target).unwrap();
    {
        let mut writer = BufWriter::new(output.file());
        writer.write_all(b"complete archive").unwrap();
        writer.flush().unwrap();
    }
    assert_eq!(std::fs::read(&target).unwrap(), b"previous archive");
    output.commit().unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"complete archive");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn failed_commit_cleans_temporary_file_without_removing_destination() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("mods_P.pak");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("sentinel"), b"untouched").unwrap();
    let mut output = InstallOutput::new(&target).unwrap();
    output.file().write_all(b"archive").unwrap();
    assert!(output.commit().is_err());
    assert_eq!(
        std::fs::read(target.join("sentinel")).unwrap(),
        b"untouched"
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn first_install_creates_archive_on_commit() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("mods_P.pak");
    let mut output = InstallOutput::new(&target).unwrap();
    output.file().write_all(b"first archive").unwrap();
    assert!(!target.exists());
    output.commit().unwrap();
    assert_eq!(std::fs::read(target).unwrap(), b"first archive");
}

#[cfg(windows)]
#[test]
fn locked_archive_is_preserved_when_replacement_is_denied() {
    use std::os::windows::fs::OpenOptionsExt;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("mods_P.pak");
    std::fs::write(&target, b"previous archive").unwrap();
    let mut output = InstallOutput::new(&target).unwrap();
    output.file().write_all(b"new archive").unwrap();
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&target)
        .unwrap();
    assert!(output.commit().is_err());
    drop(lock);
    assert_eq!(std::fs::read(&target).unwrap(), b"previous archive");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}
