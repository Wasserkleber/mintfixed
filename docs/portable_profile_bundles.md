# Portable profile ZIP bundles

In **Profile Management → Export Profile**, choose the source profile, then check **Include local PAK/ZIP files (portable ZIP)**, then click **Export profile**. Without the checkbox, choose JSON or TXT for an ordinary export. Portable ZIPs always use the full JSON profile definition to preserve group settings.
The ZIP contains `profile.json`, `manifest.json`, and local `mods/*.pak` and `mods/*.zip` files.
Remote mod.io links remain links. All referenced groups, members, priorities, disabled states,
member overrides, ordering, and other JSON settings are retained.

In **Profile Management → Import Profile**, load the resulting `.zip`. The preview shows the
profile name and bundled definition. Click **Import Profile** to verify and extract. If it
contains local mod archives and no valid **Settings → Local mods folder** is configured,
mintfixed shows an error instead of opening a folder picker. Extraction creates a new unique
subfolder under the configured folder; it does not overwrite previous imports. Local paths
are then updated before the profile and its groups are added to the active configuration.

You can still paste standard profile JSON or load an ordinary `.json` file. If the imported profile name already exists, the importer shows a warning and requires
a different name; profiles are never overwritten or automatically renamed.
Shared groups with conflicting names still receive unique names and are never overwritten.

Only PAK/ZIP local archives are bundled. Missing or unreadable local files fail export without
creating a partial ZIP. Portable ZIPs are validated for unsafe paths, duplicates, unlisted
entries, size limits and SHA-256 integrity before being imported. Mod files are never
installed automatically by importing a profile.

Tests: `cargo test profile_package_tests`, `cargo test profile_import`,
`cargo test group_package_tests`, `cargo check --all-targets`.

Portable profile ZIPs are saved automatically to `mintfixed exports` beside the mintfixed executable, just like group exports. No Save dialog opens. Repeated exports receive numbered filenames and never replace existing ZIPs.
