//! Fetch the game's PDB for the exact installed exe into the reporter's symbol cache.
//!
//! BP-Crash-Reporter looks for `mintfixed-bpcrash/symbols/<pdb>/<GUID><age>/<pdb>` and loads it
//! only when its GUID and age match the exe's CodeView (RSDS) record. There is one PDB per game
//! build (release and experimental differ, and every update needs a new one), and they run to
//! hundreds of MB, so they are fetched on demand, keyed by that record, rather than bundled with
//! the reporter. Nothing here is fatal: with no PDB the reporter prints module+RVA as before.
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use fs_err as fs;
use tracing::{info, warn};

/// Release assets named `<GUID><age>-<pdb>.zip`, one zipped PDB each. A 404 means no PDB was
/// published for this build.
const SYMBOLS_URL: &str =
    "https://github.com/Elytras/BP-Crash-Reporter/releases/download/drg-symbols";
const MAX_PDB_BYTES: u64 = 4 << 30;
/// Release and experimental, so switching branch does not download again.
const KEEP_BUILDS: usize = 2;

/// The PDB an exe was linked with: its file name and the symstore key, GUID then age in hex.
#[derive(Debug, PartialEq)]
pub(super) struct PdbId {
    pub name: String,
    pub key: String,
}

fn key(guid: &[u8], age: u32) -> String {
    let d1 = u32::from_le_bytes(guid[0..4].try_into().unwrap());
    let d2 = u16::from_le_bytes(guid[4..6].try_into().unwrap());
    let d3 = u16::from_le_bytes(guid[6..8].try_into().unwrap());
    format!(
        "{d1:08X}{d2:04X}{d3:04X}{}{age:X}",
        hex::encode_upper(&guid[8..16])
    )
}

fn u16_at(b: &[u8], o: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        b.get(o..o + 2).context("truncated")?.try_into()?,
    ))
}
fn u32_at(b: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        b.get(o..o + 4).context("truncated")?.try_into()?,
    ))
}
fn read_at(f: &mut File, offset: u64, len: usize) -> Result<Vec<u8>> {
    let mut buf = vec![0; len];
    f.seek(SeekFrom::Start(offset))?;
    f.read_exact(&mut buf)?;
    Ok(buf)
}

/// The exe's RSDS record, or None when it has no CodeView debug entry.
pub(super) fn exe_pdb_id(exe: &Path) -> Result<Option<PdbId>> {
    let mut f = File::open(exe).with_context(|| format!("cannot open {}", exe.display()))?;
    let mut h = vec![0; 4096];
    let n = f.read(&mut h)?;
    h.truncate(n);
    let pe = u32_at(&h, 0x3c)? as usize;
    if h.get(pe..pe + 4) != Some(b"PE\0\0") {
        bail!("{} is not a PE image", exe.display());
    }
    let sections = u16_at(&h, pe + 6)? as usize;
    let opt = pe + 24;
    let opt_size = u16_at(&h, pe + 20)? as usize;
    let dirs = opt + if u16_at(&h, opt)? == 0x20b { 112 } else { 96 };
    let (dbg_rva, dbg_size) = (u32_at(&h, dirs + 6 * 8)?, u32_at(&h, dirs + 6 * 8 + 4)?);
    let to_file = |rva: u32| -> Result<Option<u64>> {
        for s in 0..sections {
            let sh = opt + opt_size + s * 40;
            let (vsize, va, rsize, raw) = (
                u32_at(&h, sh + 8)?,
                u32_at(&h, sh + 12)?,
                u32_at(&h, sh + 16)?,
                u32_at(&h, sh + 20)?,
            );
            if rva >= va && rva < va + vsize.max(rsize) {
                return Ok(Some((raw + rva - va) as u64));
            }
        }
        Ok(None)
    };
    let Some(dbg_off) = to_file(dbg_rva)? else {
        return Ok(None);
    };
    let entries = read_at(&mut f, dbg_off, dbg_size as usize)?;
    for e in entries.as_chunks::<28>().0 {
        if u32_at(e, 12)? != 2 {
            continue; // IMAGE_DEBUG_TYPE_CODEVIEW
        }
        let cv = read_at(&mut f, u32_at(e, 24)? as u64, u32_at(e, 16)? as usize)?;
        if cv.len() < 25 || &cv[..4] != b"RSDS" {
            continue;
        }
        let path = String::from_utf8_lossy(&cv[24..]);
        let path = path.trim_end_matches('\0');
        // The RSDS name can be a full build path; the store wants the file name only.
        let name = path.rsplit(['\\', '/']).next().unwrap_or(path).to_string();
        return Ok(Some(PdbId {
            name,
            key: key(&cv[4..20], u32_at(&cv, 20)?),
        }));
    }
    Ok(None)
}

/// The symstore key a PDB file carries: the GUID from its info stream, the age from its DBI
/// stream (the age the exe's RSDS record repeats).
pub(super) fn pdb_key(pdb: &Path) -> Result<String> {
    let mut f = File::open(pdb).with_context(|| format!("cannot open {}", pdb.display()))?;
    let sb = read_at(&mut f, 0, 56)?;
    if &sb[..32] != b"Microsoft C/C++ MSF 7.00\r\n\x1aDS\0\0\0" {
        bail!("{} is not an MSF 7 PDB", pdb.display());
    }
    let bs = u32_at(&sb, 32)? as u64;
    let dir_bytes = u32_at(&sb, 44)? as usize;
    let map = read_at(
        &mut f,
        u32_at(&sb, 52)? as u64 * bs,
        dir_bytes.div_ceil(bs as usize) * 4,
    )?;
    let mut dir = Vec::with_capacity(dir_bytes);
    for b in map.as_chunks::<4>().0 {
        dir.extend(read_at(&mut f, u32_at(b, 0)? as u64 * bs, bs as usize)?);
    }
    let streams = u32_at(&dir, 0)? as usize;
    if streams < 4 {
        bail!("{} has no DBI stream", pdb.display());
    }
    // The first block of streams 1 (PDB info) and 3 (DBI); both headers fit in one block.
    let mut at = 4 + streams * 4;
    let mut first = [0u64; 4];
    for (s, slot) in first.iter_mut().enumerate() {
        let size = u32_at(&dir, 4 + s * 4)?;
        let blocks = if size == u32::MAX {
            0
        } else {
            (size as u64).div_ceil(bs) as usize
        };
        if blocks > 0 {
            *slot = u32_at(&dir, at)? as u64 * bs;
        }
        at += blocks * 4;
    }
    let info = read_at(&mut f, first[1], 28)?;
    let dbi = read_at(&mut f, first[3], 12)?;
    Ok(key(&info[12..28], u32_at(&dbi, 8)?))
}

/// Start a background fetch of `exe`'s PDB into `dir/symbols`. Install Mods must not wait on a
/// download of this size, and at most one runs at a time.
pub(super) fn spawn_fetch(dir: PathBuf, exe: PathBuf) {
    static BUSY: AtomicBool = AtomicBool::new(false);
    if BUSY.swap(true, Ordering::AcqRel) {
        return;
    }
    std::thread::spawn(move || {
        if let Err(err) = fetch(&dir, &exe) {
            warn!("BP-Crash-Reporter symbols: {err:#}");
        }
        BUSY.store(false, Ordering::Release);
    });
}

fn fetch(dir: &Path, exe: &Path) -> Result<()> {
    let Some(id) = exe_pdb_id(exe)? else {
        info!(
            "BP-Crash-Reporter symbols: {} names no PDB; nothing to fetch",
            exe.display()
        );
        return Ok(());
    };
    let store = dir.join("symbols").join(&id.name);
    let target = store.join(&id.key).join(&id.name);
    if !target.is_file() {
        download(&id, &target)?;
    }
    prune(&store, &id.key);
    Ok(())
}

fn download(id: &PdbId, target: &Path) -> Result<()> {
    let url = format!("{SYMBOLS_URL}/{}-{}.zip", id.key, id.name);
    info!("BP-Crash-Reporter symbols: fetching {url}");
    let response = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(60 * 60))
        .build()?
        .get(&url)
        .header(reqwest::header::USER_AGENT, "mintfixed-bpcrash-installer")
        .send()
        .context("symbol request failed")?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        info!(
            "BP-Crash-Reporter symbols: none published for {} {}",
            id.name, id.key
        );
        return Ok(());
    }
    let response = response
        .error_for_status()
        .context("symbol download failed")?;

    let folder = target.parent().context("symbol path has no folder")?;
    fs::create_dir_all(folder)?;
    let zip_part = folder.join("download.zip.part");
    let pdb_part = folder.join("download.pdb.part");
    let result = (|| -> Result<()> {
        let mut zip_file = fs::File::create(&zip_part)?;
        let copied = std::io::copy(&mut response.take(MAX_PDB_BYTES + 1), &mut zip_file)?;
        if copied > MAX_PDB_BYTES {
            bail!("symbol archive exceeds size limit");
        }
        let mut archive = zip::ZipArchive::new(fs::File::open(&zip_part)?)?;
        let mut entry = archive
            .by_name(&id.name)
            .with_context(|| format!("symbol archive has no {}", id.name))?;
        if entry.size() > MAX_PDB_BYTES {
            bail!("PDB exceeds size limit");
        }
        std::io::copy(
            &mut (&mut entry).take(MAX_PDB_BYTES + 1),
            &mut fs::File::create(&pdb_part)?,
        )?;
        // Never install a PDB for another build: the reporter would refuse it anyway.
        let got = pdb_key(&pdb_part)?;
        if got != id.key {
            bail!("downloaded PDB is {got}, the exe wants {}", id.key);
        }
        fs::rename(&pdb_part, target)?;
        Ok(())
    })();
    let _ = fs::remove_file(&zip_part);
    let _ = fs::remove_file(&pdb_part);
    result?;
    info!("BP-Crash-Reporter symbols: installed {}", target.display());
    Ok(())
}

/// Keep the current build's PDB and the newest other one; delete the rest.
fn prune(store: &Path, current: &str) {
    let Ok(entries) = fs::read_dir(store) else {
        return;
    };
    let mut others: Vec<_> = entries
        .flatten()
        .filter(|e| e.file_name() != current && e.path().is_dir())
        .map(|e| (e.metadata().and_then(|m| m.modified()).ok(), e.path()))
        .collect();
    others.sort_by_key(|o| std::cmp::Reverse(o.0));
    for (_, path) in others.into_iter().skip(KEEP_BUILDS - 1) {
        match fs::remove_dir_all(&path) {
            Ok(()) => info!("BP-Crash-Reporter symbols: removed old {}", path.display()),
            Err(err) => warn!(
                "BP-Crash-Reporter symbols: could not remove {}: {err}",
                path.display()
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round trip on this test binary: its RSDS record must name a PDB beside it whose own GUID
    /// and age give the same key. MSVC targets only, where the linker writes both.
    #[test]
    #[cfg(all(windows, target_env = "msvc"))]
    fn exe_and_pdb_keys_agree() {
        let exe = std::env::current_exe().unwrap();
        let id = exe_pdb_id(&exe)
            .unwrap()
            .expect("test binary has an RSDS record");
        assert!(id.key.len() >= 33, "32 GUID digits and an age");
        let pdb = exe.with_file_name(&id.name);
        assert_eq!(pdb_key(&pdb).unwrap(), id.key);
    }

    /// By hand, on a real game build: `BPCRASH_EXE=<exe> BPCRASH_PDB=<pdb> cargo test -- --ignored`.
    /// Prints both keys; passes only when they match.
    #[test]
    #[ignore]
    fn game_exe_and_pdb_keys_agree() {
        let exe = PathBuf::from(std::env::var("BPCRASH_EXE").unwrap());
        let pdb = PathBuf::from(std::env::var("BPCRASH_PDB").unwrap());
        let id = exe_pdb_id(&exe).unwrap().unwrap();
        let got = pdb_key(&pdb).unwrap();
        println!("exe {} {}  pdb {got}", id.name, id.key);
        assert_eq!(got, id.key);
    }

    #[test]
    fn key_matches_symstore_layout() {
        // GUID 7F157EDF-1E03-4F00-80E1-6E4DFF522DFA age 1, as DRG CL-166125's RSDS stores it.
        let guid = [
            0xDF, 0x7E, 0x15, 0x7F, 0x03, 0x1E, 0x00, 0x4F, 0x80, 0xE1, 0x6E, 0x4D, 0xFF, 0x52,
            0x2D, 0xFA,
        ];
        assert_eq!(key(&guid, 1), "7F157EDF1E034F0080E16E4DFF522DFA1");
        assert_eq!(key(&guid, 0x1A), "7F157EDF1E034F0080E16E4DFF522DFA1A");
    }

    #[test]
    fn non_pdb_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("x.pdb");
        fs::write(&fake, vec![0u8; 128]).unwrap();
        assert!(pdb_key(&fake).is_err());
    }

    #[test]
    fn prune_keeps_current_and_newest_other() {
        let dir = tempfile::tempdir().unwrap();
        for (i, k) in ["OLD1", "OLD2", "CUR"].iter().enumerate() {
            fs::create_dir(dir.path().join(k)).unwrap();
            std::thread::sleep(Duration::from_millis(20 * (i as u64 + 1)));
        }
        prune(dir.path(), "CUR");
        assert!(dir.path().join("CUR").is_dir());
        assert!(dir.path().join("OLD2").is_dir());
        assert!(!dir.path().join("OLD1").exists());
    }
}
