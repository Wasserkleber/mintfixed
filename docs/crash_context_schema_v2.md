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

Constraints: max 32 nested PAKs per ZIP; max 512 MiB uncompressed PAK; max
16384 ZIP entries; max 65536 names per PAK. Encrypted, compressed-index, or
unsupported PAKs are incomplete/unreadable, not silently considered empty.

Do not export auth tokens, unrelated profiles, or entire mod configuration files.
The reporter still cannot prove which mod won a conflicting merged package.
The publication contract is a two-project change; roll out producer before
consumer. v1 consumers are not forward compatible with v2; v2 reporter rejects
v1 and reports the status explicitly.
