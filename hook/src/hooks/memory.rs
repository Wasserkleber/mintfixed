// Mechanical extraction from hook/src/hooks/mod.rs; detour behavior is unchanged.
use anyhow::Result;
use std::ffi::c_void;
use windows::Win32::System::Memory::{PAGE_EXECUTE_READWRITE, VirtualProtect};

pub(super) unsafe fn patch_mem(address: *mut u8, patch: impl AsRef<[u8]>) -> Result<()> {
    unsafe {
        let patch = patch.as_ref();
        let patch_mem = std::slice::from_raw_parts_mut(address, patch.len());

        let mut old = Default::default();
        VirtualProtect(
            patch_mem.as_ptr() as *const c_void,
            patch_mem.len(),
            PAGE_EXECUTE_READWRITE,
            &mut old,
        )?;

        patch_mem.copy_from_slice(patch);

        VirtualProtect(
            patch_mem.as_ptr() as *const c_void,
            patch_mem.len(),
            old,
            &mut old,
        )?;
    }

    Ok(())
}
