# Bundled mod.io presets

An optional preset lets a custom mintfixed build ship with predefined individual
mods and groups. The **configuration** is embedded in the executable; the actual
mod files are NOT bundled or downloaded on startup. This is not an offline
mod bundle.

1. Edit `assets/preinstall.json` before building. The default `items: []` does
   nothing. For example, replace the URLs below with your actual mod.io links:

   ```json
   {
     "profile": "Starter",
     "items": [
       {
         "type": "group",
         "name": "Utilities",
         "enabled": true,
         "mods": [
           {"url": "https://mod.io/g/drg/m/your-first-mod", "enabled": true, "priority": 5},
           {"url": "https://mod.io/g/drg/m/your-second-mod", "enabled": false}
         ]
       },
       {"type": "mod", "url": "https://mod.io/g/drg/m/your-standalone-mod", "enabled": true}
     ]
   }
   ```

2. Rebuild with `cargo build --release` (or your tagged build script). The
   resulting executable contains the preset. A ZIP containing that executable
   contains the preset as well.
3. On a *fresh installation* (neither `mod_data.json` nor legacy
   `profiles.json` exists), mintfixed creates the named profile and groups in the
   given order, without downloading or installing anything. Users must configure
   their mod.io Personal Access Token and press **Install mods** themselves to
   download and install the enabled mods for the selected profile.

Other rules:

- Top-level `"enabled": false` disables all bundled groups and individual mods.
  It defaults to `true`, and only affects first-time setups.
- Only mod.io DRG mod URLs are accepted, including pinned modfile URLs.
- `enabled` defaults to `true`, `priority` to `0`, and `required` to `false`.
- Each URL may occur only once in the preset. Group names must be unique.
- Nothing is downloaded on first launch, even when some entries are enabled.
- Only the normal **Install mods** action installs the selected profile's mods.
- Existing users' profiles are never overwritten. Rebuilding with a different
  preset does *not* reapply it to existing users.
- No `preinstall.pending` marker is created. If an older test build left one
  behind, it has no effect on the revised build.
- Do not put a mod.io access token in the preset or the build.

## Descriptions and info icons

Optionally add `"description"` to **each standalone mod**, **each group**, or
**each mod inside a group** in `assets/preinstall.json`. For example:

```json
{
  "enabled": true,
  "profile": "Starter",
  "items": [
    {"type":"group", "name":"Utilities", "description":"Optional quality-of-life tools",
     "mods":[{"url":"https://mod.io/g/drg/m/example",
              "description":"Adds optional utility features"}]},
    {"type":"mod", "url":"https://mod.io/g/drg/m/example-two",
     "description":"Standalone performance improvement"}
  ]
}
```

On a fresh installation, mintfixed saves descriptions alongside the mod/group
entries in `mod_data.json`. Hover the **ⓘ** icon on a mod row or group heading
to read its description. Descriptions are optional; older installs keep working.
No provider cache changes are required. No mods download until **Install mods**
is pressed after configuring a mod.io token.

Rebuilding the preset does not update previously seeded descriptions. To repeat
the first-run test, reset the isolated test appdata directory.
