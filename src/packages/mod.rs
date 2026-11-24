//! Shared portable-package infrastructure, independent of the GUI.
mod archive;
mod extraction;
mod filenames;
pub(crate) mod group;
pub(crate) mod group_document;
mod hashing;
pub(crate) mod profile;
pub(crate) mod profile_document;

pub(crate) use archive::{
    ArchiveKind, IncludedFile, MAX_DEFINITION, MAX_FILES, MAX_MANIFEST, MAX_TOTAL, Manifest,
    PREFIX, bounded_text, validate_entries, write_package,
};
pub(crate) use extraction::extract_mod_files;
#[cfg(test)]
pub(crate) use filenames::{bundle_filename, original_file_name, unique_destination_name};
pub(crate) use filenames::{
    local_archive, safe_archive_path, safe_component, unique_bundle_filename,
};
pub(crate) use hashing::copy_and_hash;
