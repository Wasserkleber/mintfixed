# Portable group packages (ZIP)

In **Group Management → Export groups**, choose **JSON** (keeps all group/member settings)
or **TXT** (one mod URL/path per line, defaults on reimport). Enable **Include local PAK/ZIP
files** to produce a single portable `.zip` in `mintfixed exports` beside the executable.
Without the checkbox, the existing standalone JSON and TXT exports remain unchanged.

Packages contain `manifest.json`, `group.json` or `group.txt`, plus a `mods/` folder
with local `.pak` and `.zip` files. Remote mod.io URLs remain URLs. Each included mod
is assigned a unique filename and a SHA-256 checksum in the manifest. A package export
fails (without leaving a partial ZIP) if a referenced local mod cannot be read.

In **Group Management → Import groups**, choose the ZIP. The group is previewed and can
be renamed, as with JSON/TXT import. If **Settings → local mods folder** is configured,
mintfixed automatically uses it without showing a folder picker. If the setting
is empty or invalid, clicking Import group shows a concise error requesting a
valid local mods folder in Settings. New mods go directly into the configured mods folder. Files with identical
contents are reused; filename conflicts get a numbered suffix such as
`CustomMod (2).pak`. Existing files are never overwritten. The importer verifies
all SHA-256 checksums, then imports the group and updates its paths. If the
configured folder is missing or inaccessible, import displays an error and keeps
the group pending; change Settings to correct the folder and retry. Existing groups
and existing mod files are never overwritten; cancellation does not import anything.

Only this package format is accepted. Extraction rejects traversal paths, symlinks,
duplicate/unlisted entries and files exceeding the limits (1000 files / 8 GiB total).
ZIP-packed `.pak`/`.zip` payloads use stored mode, so archives cannot force decompression
of massive mod contents. Arbitrary ZIP files containing mod binaries are not accepted as
full group packages. Imported archives are not automatically installed or executed.

The configured Settings local-mod-folder search/relink system remains available for
older JSON/TXT imports with absolute paths that do not exist on the new computer.

**Testing:** `cargo test group_package_tests` and `cargo check --all-targets`.
Try one JSON+PAKs and one TXT+PAKs round-trip; verify that JSON preserves priorities,
enabled states and descriptions, TXT restores default values, and mod.io URLs are
not copied into the ZIP. Test two local PAKs with identical basenames, a missing
PAK, an invalid ZIP, an edited ZIP entry, Cancel and repeated imports.

## Import error when local files are missing

When importing a group from JSON, TXT, or a portable ZIP, press **Import group**
as usual. If there are missing **external** PAK/ZIP paths, or bundled local files
need extraction, and Settings → Local mods folder is not configured to an
existing directory, import is rejected with a simple error: "Cannot import
this group: configure a valid local mods folder in Settings first."

No warning is added to the preview, and the Import group button stays enabled.
The selected group and its settings are not modified by the rejected import.
After configuring a valid folder, try Import group again; unresolved files
continue through the normal search and relocation warnings. Local files
*bundled inside a portable ZIP* require a destination even though they
do not count as missing external paths. Remote-only ZIPs need no extraction.
