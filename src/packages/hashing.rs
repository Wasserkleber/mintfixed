//! Bounded streaming copy and SHA-256 verification.
use std::io::{Read, Write};
use sha2::{Digest, Sha256};

pub(crate) fn copy_and_hash<R: Read, W: Write>(
    input: &mut R,
    output: &mut W,
    limit: u64,
) -> Result<(u64, String), String> {
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = input
            .read(&mut buffer)
            .map_err(|e| format!("Cannot read mod file: {e}"))?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(count as u64)
            .ok_or("File size overflow")?;
        if total > limit {
            return Err("Bundled mod exceeds the archive size limit".into());
        }
        output
            .write_all(&buffer[..count])
            .map_err(|e| format!("Cannot write mod file: {e}"))?;
        hasher.update(&buffer[..count]);
    }
    Ok((total, format!("{:x}", hasher.finalize())))
}
