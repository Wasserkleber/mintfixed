//! Installation-time package index extraction for the optional crash-context
//! manifest. Never executes mods and never writes extracted ZIP members.
use serde::Serialize;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const MAX_PAK_BYTES: u64 = 512 * 1024 * 1024;
const MAX_INDEX_BYTES: u64 = 32 * 1024 * 1024;
const MAX_PAKS_PER_ZIP: usize = 32;
const MAX_ZIP_ENTRIES: usize = 16384;
const MAX_PACKAGE_NAMES: usize = 65536;

#[derive(Serialize)]
pub(super) struct IndexedPak {
    // None: direct archive .pak; Some: original member name in a ZIP/cache blob.
    member: Option<String>,
    packages: Vec<String>,
    complete: bool,
}

#[derive(Serialize)]
pub(super) struct ArchiveIndex {
    // Complete means all PAK members were indexed. Partial/unreadable must NOT
    // be interpreted as proof that a package is absent.
    status: &'static str,
    paks: Vec<IndexedPak>,
}

fn slice_at<'a>(data: &'a [u8], pos: &mut usize, length: usize) -> Result<&'a [u8], String> {
    let end = pos.checked_add(length).ok_or("index offset overflow")?;
    let got = data.get(*pos..end).ok_or("index truncated")?;
    *pos = end;
    Ok(got)
}
fn u32_at(data: &[u8], pos: &mut usize) -> Result<u32, String> {
    Ok(u32::from_le_bytes(
        slice_at(data, pos, 4)?
            .try_into()
            .map_err(|_| "short u32")?,
    ))
}
fn u64_at(data: &[u8], pos: &mut usize) -> Result<u64, String> {
    Ok(u64::from_le_bytes(
        slice_at(data, pos, 8)?
            .try_into()
            .map_err(|_| "short u64")?,
    ))
}
fn fstring(data: &[u8], pos: &mut usize) -> Result<String, String> {
    let len = i32::from_le_bytes(
        slice_at(data, pos, 4)?
            .try_into()
            .map_err(|_| "short FString")?,
    );
    if len == i32::MIN || len.unsigned_abs() > 4096 {
        return Err("FString length outside limits".into());
    }
    if len < 0 {
        let bytes = slice_at(data, pos, (len.unsigned_abs() as usize) * 2)?;
        let mut chars = Vec::new();
        for pair in bytes.chunks_exact(2) {
            let c = u16::from_le_bytes([pair[0], pair[1]]);
            if c != 0 {
                chars.push(c);
            }
        }
        return String::from_utf16(&chars).map_err(|_| "invalid UTF16 FString".into());
    }
    let bytes = slice_at(data, pos, len as usize)?;
    let without_nul = bytes.strip_suffix(&[0]).unwrap_or(bytes);
    String::from_utf8(without_nul.to_vec()).map_err(|_| "invalid UTF8 FString".into())
}
fn read_region<R: Read + Seek>(
    source: &mut R,
    size: u64,
    at: u64,
    count: u64,
) -> Result<Vec<u8>, String> {
    if count > MAX_INDEX_BYTES || at > size || count > size - at {
        return Err("PAK index range outside archive".into());
    }
    let mut result = vec![0u8; count as usize];
    source
        .seek(SeekFrom::Start(at))
        .map_err(|e| e.to_string())?;
    source.read_exact(&mut result).map_err(|e| e.to_string())?;
    Ok(result)
}

struct ParsedPakIndex {
    packages: Vec<String>,
    // A legacy index scanned heuristically may contain false-positive names.
    // Never claim completeness when any candidate was rejected.
    complete: bool,
}

fn valid_package_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 1024 && !name.chars().any(char::is_control)
}

fn parse_legacy_v8_index(
    index: &[u8],
    start: usize,
    mount: &str,
    pak_data_end: u64,
) -> Result<Vec<String>, String> {
    // UE4 PAK v8: entry count followed by inline FString filename and
    // FPakEntry metadata. Never search opaque binary metadata for filenames.
    let mut pos = start;
    let count = u32_at(index, &mut pos)? as usize;
    if count == 0 || count > MAX_PACKAGE_NAMES {
        return Err("invalid UE4 v8 entry count".into());
    }
    let mut names = Vec::with_capacity(count);
    for _ in 0..count {
        let filename = fstring(index, &mut pos)?;
        let full_name = format!("{mount}{filename}").to_ascii_lowercase();
        if !valid_package_name(&full_name) {
            return Err("invalid UE4 v8 filename".into());
        }

        let offset = u64_at(index, &mut pos)?;
        let stored_size = u64_at(index, &mut pos)?;
        let unpacked_size = u64_at(index, &mut pos)?;
        let compression_method = u32_at(index, &mut pos)?;
        let _sha1 = slice_at(index, &mut pos, 20)?;
        // Only verify the uncompressed v8 layout here. More complex v8
        // variants fall back to heuristic discovery marked partial.
        if compression_method != 0 {
            return Err("compressed UE4 v8 layout requires another parser".into());
        }
        let encrypted = slice_at(index, &mut pos, 1)?[0];
        let block_size = u32_at(index, &mut pos)?;
        if encrypted > 1 || block_size != 0 || stored_size != unpacked_size {
            return Err("unsupported UE4 v8 entry layout".into());
        }
        let end = offset
            .checked_add(stored_size)
            .ok_or("UE4 v8 entry size overflow")?;
        if end > pak_data_end {
            return Err("UE4 v8 entry extends into index".into());
        }
        names.push(full_name);
    }
    if pos != index.len() {
        return Err("UE4 v8 index has unparsed trailing data".into());
    }
    Ok(names)
}

fn read_pak_index<R: Read + Seek>(reader: &mut R) -> Result<ParsedPakIndex, String> {
    let size = reader.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
    if size < 36 || size > MAX_PAK_BYTES {
        return Err("PAK outside supported size".into());
    }
    let tail_start = size.saturating_sub(300);
    let tail = read_region(reader, size, tail_start, size - tail_start)?;
    // UE4 PAK footer magic, little-endian, preceded by encrypted-index flag.
    let magic = [0xE1, 0x12, 0x6F, 0x5A];
    let at = (1..=tail.len().saturating_sub(4))
        .rev()
        .find(|&at| tail.get(at..at + 4) == Some(magic.as_slice()))
        .ok_or("PAK footer not found")?;
    if tail[at - 1] != 0 {
        return Err("encrypted PAK index".into());
    }
    let mut fp = at + 4;
    let version = u32_at(&tail, &mut fp)?;
    let index_at = u64_at(&tail, &mut fp)?;
    let index_size = u64_at(&tail, &mut fp)?;
    let index = read_region(reader, size, index_at, index_size)?;
    let mut p = 0;
    let mount = fstring(&index, &mut p)?.replace('\\', "/");
    // Remove the original ../../../ prefix without changing case of filenames.
    let mut mount = mount.as_str();
    while let Some(rest) = mount.strip_prefix("../") {
        mount = rest;
    }
    let mut paths = Vec::new();
    let mut complete = true;
    if version < 10 {
        // Prefer an exact UE4 v8 directory walk. This avoids false names
        // found by scanning binary FPakEntry fields. The conservative
        // fallback keeps any discoveries but does not claim completeness.
        if version == 8 {
            if let Ok(mut exact_names) = parse_legacy_v8_index(&index, p, mount, index_at) {
                exact_names.sort_unstable();
                exact_names.dedup();
                return Ok(ParsedPakIndex {
                    packages: exact_names,
                    complete: true,
                });
            }
            complete = false;
        }
        // Legacy PAKs do not have a separate full-directory index. Instead,
        // the primary index contains inline length-prefixed filenames.
        for i in p..index.len().saturating_sub(4) {
            let n = index[i] as usize;
            if n < 5 || index.get(i + 1..i + 4) != Some(&[0u8, 0, 0][..]) {
                continue;
            }
            let Some(candidate) = index.get(i + 4..i + 4 + n) else {
                continue;
            };
            if !candidate.ends_with(&[0])
                || !candidate.contains(&b'.')
                || !candidate.iter().all(|b| *b == 0 || (32..=126).contains(b))
            {
                continue;
            }
            // Legacy scan may match binary length/offset records. Reject NULs
            // rather than incorporating them into a JSON package name.
            if candidate[..n - 1].contains(&0) {
                complete = false;
                continue;
            }
            let name = String::from_utf8_lossy(&candidate[..n - 1]);
            paths.push(format!("{mount}{name}").to_ascii_lowercase());
            if paths.len() >= MAX_PACKAGE_NAMES {
                return Err("too many PAK filenames".into());
            }
        }
    } else {
        let entries = u32_at(&index, &mut p)?;
        if entries > MAX_PACKAGE_NAMES as u32 {
            return Err("too many PAK entries".into());
        }
        let _seed = u64_at(&index, &mut p)?;
        let hash_index = u32_at(&index, &mut p)?;
        if hash_index > 1 {
            return Err("unsupported hash-index flag".into());
        }
        if hash_index == 1 {
            slice_at(&index, &mut p, 36)?;
        }
        let dir_index = u32_at(&index, &mut p)?;
        if dir_index != 1 {
            return Err("missing PAK full directory index".into());
        }
        let fdi_at = u64_at(&index, &mut p)?;
        let fdi_size = u64_at(&index, &mut p)?;
        let fdi = read_region(reader, size, fdi_at, fdi_size)?;
        let mut d = 0;
        let folders = u32_at(&fdi, &mut d)?;
        if folders > MAX_PACKAGE_NAMES as u32 {
            return Err("too many PAK directories".into());
        }
        for _ in 0..folders {
            let folder = fstring(&fdi, &mut d)?.replace('\\', "/");
            let folder = folder.trim_start_matches('/');
            let files = u32_at(&fdi, &mut d)?;
            if files > MAX_PACKAGE_NAMES as u32
                || paths.len() + (files as usize) > MAX_PACKAGE_NAMES
            {
                return Err("too many indexed PAK filenames".into());
            }
            for _ in 0..files {
                let filename = fstring(&fdi, &mut d)?;
                let _entry = u32_at(&fdi, &mut d)?;
                paths.push(format!("{mount}{folder}{filename}").to_ascii_lowercase());
            }
        }
    }
    let before = paths.len();
    paths.retain(|path| valid_package_name(path));
    if before != paths.len() {
        complete = false;
    }
    paths.sort_unstable();
    paths.dedup();
    if paths.is_empty() {
        return Err("empty/unsupported PAK index".into());
    }
    Ok(ParsedPakIndex {
        packages: paths,
        complete,
    })
}

pub(super) fn index_archive(path: &Path, kind: &str) -> ArchiveIndex {
    let mut result = ArchiveIndex {
        status: "unreadable",
        paks: Vec::new(),
    };
    if kind == "zip" {
        if std::fs::metadata(path)
            .map(|m| m.len() > 1024 * 1024 * 1024)
            .unwrap_or(true)
        {
            return result;
        }
        let Ok(file) = File::open(path) else {
            return result;
        };
        let Ok(mut zip) = zip::ZipArchive::new(file) else {
            return result;
        };
        if zip.len() > MAX_ZIP_ENTRIES {
            return result;
        }
        let mut attempted = 0usize;
        let mut failed = false;
        for i in 0..zip.len() {
            let Ok(member) = zip.by_index(i) else {
                failed = true;
                continue;
            };
            let member_name = member.name().replace('\\', "/");
            if !member_name.to_ascii_lowercase().ends_with(".pak") {
                continue;
            }
            attempted += 1;
            if attempted > MAX_PAKS_PER_ZIP {
                failed = true;
                break;
            }
            let mut record = IndexedPak {
                member: Some(member_name),
                packages: Vec::new(),
                complete: false,
            };
            if member.size() <= MAX_PAK_BYTES {
                let mut bytes = Vec::new();
                if member
                    .take(MAX_PAK_BYTES + 1)
                    .read_to_end(&mut bytes)
                    .is_ok()
                    && bytes.len() as u64 <= MAX_PAK_BYTES
                {
                    if let Ok(index) = read_pak_index(&mut std::io::Cursor::new(bytes)) {
                        record.packages = index.packages;
                        record.complete = index.complete;
                    }
                }
            }
            if !record.complete {
                failed = true;
            }
            result.paks.push(record);
        }
        if attempted > 0 {
            result.status = if failed { "partial" } else { "complete" };
        }
    } else if kind == "pak_or_unknown" {
        if let Ok(mut file) = File::open(path) {
            let indexed = read_pak_index(&mut file);
            let (packages, complete) = match indexed {
                Ok(index) => (index.packages, index.complete),
                Err(_) => (Vec::new(), false),
            };
            result.status = if complete {
                "complete"
            } else if packages.is_empty() {
                "unreadable"
            } else {
                "partial"
            };
            result.paks.push(IndexedPak {
                member: None,
                packages,
                complete,
            });
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unreadable_and_truncated_paks() {
        assert!(read_pak_index(&mut std::io::Cursor::new(vec![0; 80])).is_err());
        assert_eq!(
            index_archive(
                Path::new("__missing_mintfixed_archive__.pak"),
                "pak_or_unknown"
            )
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
    fn rejects_invalid_fstrings() {
        let mut p = 0;
        assert!(fstring(&[0xff, 0xff, 0xff, 0x7f], &mut p).is_err());
    }
}
