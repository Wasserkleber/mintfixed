use super::*;
use std::io::Write;
#[test]
fn reuses_installed_file_list_without_opening_any_source_archive() {
    let selected = from_installed_files(
        Some("first.pak"),
        ["FSD/Content/Test/Foo.uasset", "FSD/Content/Test/Foo.uexp"],
    );
    assert_eq!(selected.kind, "zip");
    assert_eq!(selected.index.status, "complete");
    assert_eq!(selected.index.paks.len(), 1);
    assert_eq!(selected.index.paks[0].member.as_deref(), Some("first.pak"));
    assert!(
        selected.index.paks[0]
            .packages
            .contains(&"fsd/content/test/foo.uasset".to_owned())
    );
}

#[test]
fn rejected_install_path_never_claims_a_complete_index() {
    let result = from_installed_files(None, ["FSD/Content/Good.uasset", "bad\0file.uasset"]);
    assert_eq!(result.kind, "pak_or_unknown");
    assert_eq!(result.index.status, "partial");
    assert!(!result.index.paks[0].complete);
    assert_eq!(result.index.paks[0].packages.len(), 1);
}

#[test]
fn rejects_unreadable_and_truncated_paks() {
    assert!(read_pak_index(&mut std::io::Cursor::new(vec![0; 80])).is_err());
    assert_eq!(
        index_archive(Path::new("__missing_mintfixed_archive__.pak"))
            .index
            .status,
        "unreadable"
    );
}
#[test]
fn legacy_binary_false_positives_are_not_exported_as_packages() {
    // A forged, legacy-style index with one valid name and a false
    // positive whose length-prefixed payload contains embedded NUL bytes.
    let mut index = 0i32.to_le_bytes().to_vec(); // empty mount FString
    let valid = b"fsd/content/test/good.uasset\0";
    index.extend_from_slice(&(valid.len() as u32).to_le_bytes());
    index.extend_from_slice(valid);
    let bad = b"\0\0\0fsd/content/test/bad.uasset\0";
    index.extend_from_slice(&(bad.len() as u32).to_le_bytes());
    index.extend_from_slice(bad);
    let mut pak = index.clone();
    pak.extend_from_slice(&[0u8; 16]);
    pak.push(0); // PAK encrypted index flag
    pak.extend_from_slice(&[0xE1, 0x12, 0x6F, 0x5A]);
    pak.extend_from_slice(&9u32.to_le_bytes());
    pak.extend_from_slice(&0u64.to_le_bytes());
    pak.extend_from_slice(&(index.len() as u64).to_le_bytes());
    let indexed = read_pak_index(&mut std::io::Cursor::new(pak)).unwrap();
    assert!(!indexed.complete, "false-positive index cannot be complete");
    assert!(
        indexed
            .packages
            .iter()
            .any(|v| v == "fsd/content/test/good.uasset")
    );
    assert!(!indexed.packages.iter().any(|v| v.contains("bad.uasset")));
    assert!(indexed.packages.iter().all(|v| valid_package_name(v)));
}

#[test]
fn rejects_manifest_package_names_with_binary_characters() {
    assert!(valid_package_name("fsd/content/test/good.uasset"));
    assert!(!valid_package_name("\0\0fsd/content/test/bad.uasset"));
    assert!(!valid_package_name(""));
    assert!(!valid_package_name(&"x".repeat(1025)));
    assert!(!valid_package_name("fsd/content\n/test.uasset"));
}
#[test]
fn structured_v8_parser_ignores_binary_filename_false_positives() {
    let mut index = Vec::new();
    index.extend_from_slice(&10i32.to_le_bytes());
    index.extend_from_slice(b"../../../\0");
    index.extend_from_slice(&2u32.to_le_bytes());
    for (i, filename) in [
        "FSD/Content/Test/First.uasset",
        "FSD/Content/Test/Second.uexp",
    ]
    .iter()
    .enumerate()
    {
        let name = format!("{filename}\0");
        index.extend_from_slice(&(name.len() as i32).to_le_bytes());
        index.extend_from_slice(name.as_bytes());
        index.extend_from_slice(&(i as u64 * 16).to_le_bytes());
        index.extend_from_slice(&0u64.to_le_bytes()); // stored size
        index.extend_from_slice(&0u64.to_le_bytes()); // uncompressed size
        index.extend_from_slice(&0u32.to_le_bytes()); // no compression
        let mut hash = [0u8; 20];
        if i == 0 {
            // Opaque SHA-1 bytes that look like a malformed FString to
            // the old heuristic: length + embedded-NUL fake name.
            let fake = b"\0\0foo.x\0";
            hash[..4].copy_from_slice(&(fake.len() as u32).to_le_bytes());
            hash[4..4 + fake.len()].copy_from_slice(fake);
        }
        index.extend_from_slice(&hash);
        index.push(0); // unencrypted
        index.extend_from_slice(&0u32.to_le_bytes()); // block size
    }
    let mut pak = vec![0u8; 32];
    let index_at = pak.len() as u64;
    pak.extend_from_slice(&index);
    pak.push(0); // unencrypted index
    pak.extend_from_slice(&[0xE1, 0x12, 0x6F, 0x5A]);
    pak.extend_from_slice(&8u32.to_le_bytes());
    pak.extend_from_slice(&index_at.to_le_bytes());
    pak.extend_from_slice(&(index.len() as u64).to_le_bytes());

    let parsed = read_pak_index(&mut std::io::Cursor::new(pak)).unwrap();
    assert!(parsed.complete);
    assert_eq!(parsed.packages.len(), 2);
    assert!(
        parsed
            .packages
            .contains(&"fsd/content/test/first.uasset".to_string())
    );
    assert!(
        parsed
            .packages
            .contains(&"fsd/content/test/second.uexp".to_string())
    );
    assert!(parsed.packages.iter().all(|name| valid_package_name(name)));
}

#[test]
fn zip_manifest_indexes_only_the_first_installable_member() {
    fn legacy_pak(filename: &str) -> Vec<u8> {
        let mut index = 0i32.to_le_bytes().to_vec();
        let name = format!("{filename}\0");
        index.extend_from_slice(&(name.len() as u32).to_le_bytes());
        index.extend_from_slice(name.as_bytes());
        let mut pak = index.clone();
        pak.extend_from_slice(&[0u8; 16]);
        pak.push(0);
        pak.extend_from_slice(&[0xE1, 0x12, 0x6F, 0x5A]);
        pak.extend_from_slice(&9u32.to_le_bytes());
        pak.extend_from_slice(&0u64.to_le_bytes());
        pak.extend_from_slice(&(index.len() as u64).to_le_bytes());
        pak
    }
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("two_paks.zip");
    let file = File::create(&path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, filename) in [
        ("nope.PAK", "FSD/Content/Test/Excluded0.uasset"),
        ("nested/first.pak", "FSD/Content/Test/Included.uasset"),
        ("nested/second.pak", "FSD/Content/Test/Excluded1.uasset"),
    ] {
        zip.start_file(name, options).unwrap();
        zip.write_all(&legacy_pak(filename)).unwrap();
    }
    zip.finish().unwrap();
    let indexed = index_archive(&path);
    assert_eq!(indexed.kind, "zip");
    let indexed = indexed.index;
    assert_eq!(indexed.status, "complete");
    assert_eq!(
        indexed.paks.len(),
        1,
        "must not attribute uninstalled ZIP members"
    );
    assert_eq!(indexed.paks[0].member.as_deref(), Some("nested/first.pak"));
    assert_eq!(
        indexed.paks[0].packages,
        vec!["fsd/content/test/included.uasset".to_string()]
    );
}

#[test]
fn rejects_invalid_fstrings() {
    let mut p = 0;
    assert!(fstring(&[0xff, 0xff, 0xff, 0x7f], &mut p).is_err());
}
