//! Lets an integration keep the installed mods_P.pak when nothing that goes into it changed.
use std::io;

use rayon::prelude::*;
use sha2::{Digest, Sha256};

use super::*;

/// The pak entry holding the key the pak was built from (beside `meta`, outside any mounted content).
pub(super) const KEY_PATH: &str = "mint-key";

/// sha256 of everything mods_P.pak is built from: this MINT executable (its patches and integration code), the
/// meta (settings and mod list), the game pak's size and mtime, and every mod file's full contents.
pub(super) fn key(
    meta: &[u8],
    game_pak: &Path,
    mods: &[(ModInfo, PathBuf)],
) -> io::Result<[u8; 32]> {
    let file_hash = |path: &Path| -> io::Result<[u8; 32]> {
        let mut hash = Sha256::new();
        io::copy(
            &mut BufReader::with_capacity(1 << 20, fs::File::open(path)?),
            &mut hash,
        )?;
        Ok(hash.finalize().into())
    };
    let exe = file_hash(&std::env::current_exe()?)?;
    let mod_hashes = mods
        .par_iter()
        .map(|(_, path)| file_hash(path))
        .collect::<io::Result<Vec<_>>>()?;
    let game = fs::metadata(game_pak)?;

    let mut key = Sha256::new();
    key.update(exe);
    key.update((meta.len() as u64).to_le_bytes());
    key.update(meta);
    key.update(game.len().to_le_bytes());
    key.update(format!("{:?}", game.modified()?));
    for hash in mod_hashes {
        key.update(hash);
    }
    Ok(key.finalize().into())
}

/// The key stored in an installed mods_P.pak, if it has one.
pub(super) fn installed_key(mod_pak: &Path) -> Option<Vec<u8>> {
    let mut reader = BufReader::new(fs::File::open(mod_pak).ok()?);
    let pak = repak::PakBuilder::new().reader(&mut reader).ok()?;
    pak.get(KEY_PATH, &mut reader).ok()
}
