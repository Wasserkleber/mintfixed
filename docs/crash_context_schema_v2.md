# crash_context.json — mintfixed producer contract v2

Written after a successful mod installation to
`<FSD>/Binaries/<platform>/mintfixed-bpcrash/crash_context.json`.
Written atomically; the reporter reads it read-only after the game exits.

The existing v1 keys (`producer`, `generated_unix_ms`,
`captured_after_successful_install`, `installed_mods`, archive fingerprints,
`installed_pak`, etc.) remain. `schema_version` is now 2.

New per-mod key:

```json
"archive_index": {
  "status": "complete",
  "paks": [
    {
      "member": "mods/packed.pak",
      "complete": true,
      "packages": [
        "fsd/content/weaponsntools/_crosshairs/newcrosshairs/hud_cronewgrenadelauncher.uasset"
      ]
    }
  ]
}
```

`member` is `null` for direct .pak archives and a ZIP member filename for ZIP
source archives; it is data, not an extraction destination. `packages` holds
case-folded **raw PAK index file paths** including extensions; the reporter
normalizes them to package keys using its shared `PackageKey` logic.

`status` is `complete`, `partial`, or `unreadable`. A complete result requires
at least one indexed PAK with `complete:true`; partial/unreadable indicates
coverage gaps. Only `complete` supports negative evidence. A source archive
listed in the manifest may not contribute all its packages to the final
`mods_P.pak`: Blueprint compatibility guards and conflicts can exclude entries.
Consequently **source match != loaded asset or proof of crash causation**.

Constraints: exactly the first installable PAK member per ZIP; max 512 MiB uncompressed PAK; max
16384 ZIP entries; max 65536 names per PAK. Encrypted, compressed-index, or
unsupported PAKs are incomplete/unreadable, not silently considered empty.

Do not export auth tokens, unrelated profiles, or entire mod configuration files.
The reporter still cannot prove which mod won a conflicting merged package.
The publication contract is a two-project change; roll out producer before
consumer. v1 consumers are not forward compatible with v2; v2 reporter rejects
v1 and reports the status explicitly.

## Source-index reuse and archive classification

During a real `mods_P.pak` rebuild, `integrate/mod_inputs.rs::ModPak::open`
uses repak to obtain the selected member and its file names. The producer
constructs `archive_index` directly from that already-opened `ModPak.files` list,
with no second PAK directory parsing and no separate ZIP signature sniffing.
`PakSource::selected_member()` carries the actual first ZIP member chosen by
the integration reader. Direct PAK sources set `member:null`.

The unchanged-installation fast path does not open `ModPak`s. In that path,
the crash-context exporter falls back to a **single** ZIP central-directory
probe and PAK index read when needed, to publish a new manifest. This does not
change the installed `mods_P.pak`. The producer reports indexes as partial or
unreadable if a fallback cannot establish complete coverage. The 512 MiB PAK
and 16,384 ZIP-entry limits apply to fallback indexing, not the file list
already accepted by the integration reader. The reporter never performs source
ZIP detection or decompression.

## Attribution safety and lifecycle

The producer records **only the first installable ZIP PAK**, matching
`integrate/pak_reader.rs::PakSource::open`: regular file, enclosed ZIP path,
case-sensitive `.pak` extension. Later PAK members are not merged and must not
appear in attribution. `member` identifies the selected source archive member.

The fingerprint of `installed_pak` (`mods_P.pak`) includes path, `exists`,
`byte_size`, and `modified_unix_ms`. The reporter compares its actual path,
size, and millisecond modification time with the live installed PAK before
attribution. A missing or changed installed PAK results in `stale_context`, not
mod-blame. This is metadata validation, **not cryptographic binding** of a
crash to its installation; malicious or accidental same-size/same-mtime
replacement cannot be detected.

The current snapshot is replaced after a successful installation and removed
on uninstall. New installations no longer write `crash_context_history/`:
previous historical files (if any) are left untouched for manual management.
Source archive SHA-256 fields are no longer generated; no consumer was
verifying them. Absent source archive fingerprints use `null` for `byte_size`
and `modified_unix_ms` when `exists=false`.
