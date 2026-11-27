# Local Proton crash attribution integration

This patch preserves the pinned, checksum-verified injection executable and the
existing `--once` hook. Install Mods creates `bpcrash-proton.sh` next to it.

Until the matching upstream reporter changes ship in a verified release, build
and stage the patched native watcher and a separate patched triage executable:

- `bpcrash-watch` (native Linux ELF, executable)
- `bpcrash-triage.exe` (Windows PE, built from the matching upstream patch)

Copy both into `FSD/Binaries/Win64/mintfixed-bpcrash/` on Linux. Do not replace
`bpcrash.exe`: the installer verifies that file against its release marker.

Set the Steam launch option to:

```text
bash "/absolute/path/to/FSD/Binaries/Win64/mintfixed-bpcrash/bpcrash-proton.sh" %command%
```

Keep any existing options as appropriate. This is opt-in; Mintfixed does not
silently edit Steam settings. The wrapper preserves the original launch command
and exit code, uses the same Proton launcher/runtime for finite post-exit triage,
and respects the disabled marker. A missing local helper falls back to normal
launch. The GUI does not need to be open.

After updating Mintfixed, reinstall mods to refresh the manifest and hook.
The session marker now has three lines: Wine PID, Windows game executable,
and Windows report directory. Use the matching patched native watcher; the
old experimental two-line marker is no longer accepted by the new watcher.
Only Linux-produced merged PAK fingerprints receive the optional relative path.
Old reporters continue to parse the v2 manifest but may reject Linux paths.

This is a local development integration, not an automatic upstream release
upgrade. Verify real Proton launch commands, timestamp equality and Steam
relaunch behavior before shipping. The wrapper requires a recognizable Proton
`run` or `waitforexitandrun` command in Steam's expanded launch arguments.
