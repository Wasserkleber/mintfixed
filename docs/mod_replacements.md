# Bundled mod.io replacement rules

An optional configuration lets a custom mintfixed build **recommend one mod
instead of another** and recognize other mods as satisfying a dependency on
the original. For example, mintfixed can suggest **Unified UI (UUI)** when
someone adds **ModHub**, while treating an enabled UUI or EMM as satisfying a
ModHub dependency.

The **configuration** is embedded in the executable at build time. It does not
bundle mod files, download mods on startup, or silently change mods already in
a user's profiles.

## Configure replacements

1. Edit `assets/config/mod_replacements.json` **before building**. The shipped
   configuration contains the ModHub-to-UUI rule. For example:

   ```json
   {
     "enabled": true,
     "replacements": [
       {
         "enabled": true,
         "original": "https://mod.io/g/drg/m/mod-hub",
         "original_name": "ModHub",
         "replacement": "https://mod.io/g/drg/m/unified-ui",
         "replacement_name": "Unified UI",
         "alternatives": [
           "https://mod.io/g/drg/m/test"
         ],
         "description": "Unified UI replaces ModHub while being more performant.",
         "dependency_warning": "A mod settings menu (e.g. Unified UI) is needed. Click to add the recommended replacement."
       }
     ]
   }
   ```

2. Rebuild with `cargo build --release` (or your usual build script). As with
   [`preinstall.json`](preinstall.md), editing the JSON by itself **does not**
   update an existing executable. The rules apply to new actions after the
   rebuilt version is launched; unlike preinstall presets, replacement rules
   are **not limited to first-time setups**.
3. When a user **adds** a mod matching `original`, mintfixed asks whether to
   **Use [replacement] (recommended)**, **Keep [original]**, or **Cancel**.
   Choosing the replacement adds its mod.io URL through the normal add-mod
   flow. It is a recommendation, not a forced migration.
4. When another mod **requires** `original`, an **enabled** `replacement` or
   enabled mod listed in `alternatives` counts as satisfying that dependency
   for the missing-dependency warning. If none is enabled, the warning uses
   `dependency_warning`; clicking to add a missing dependency chooses the
   recommended replacement. Automatic dependency resolution avoids silently
   adding the configured original mod.

## JSON fields

| Field | Required | Meaning |
| --- | --- | --- |
| Top-level `enabled` | No | Set to `false` to disable **all** replacement rules; defaults to `true`. |
| `replacements` | No | Array of replacement rules; defaults to an empty list. |
| Rule `enabled` | No | Set to `false` to disable this rule only; defaults to `true`. |
| `original` | Yes | mod.io DRG URL of the mod to intercept or recognize as a dependency. |
| `original_name` | Yes | Display name for the original in the prompt and fallback warning; cannot be blank. |
| `replacement` | Yes | mod.io DRG URL recommended when the original is added or required. |
| `replacement_name` | Yes | Display name for the suggested mod; cannot be blank. |
| `alternatives` | No | Other mod.io DRG URLs that may **satisfy the original dependency** when enabled. They are **not** added, installed, or offered as extra buttons. Defaults to `[]`. |
| `description` | No | Optional explanation shown in the replacement prompt; defaults to an empty string. |
| `dependency_warning` | No | Optional custom text shown when the original dependency is missing. When empty, mintfixed shows a generic message with the original and recommended replacement names. |

### Replacement versus alternatives

With the example above, adding **ModHub** offers to add **Unified UI** instead.
When another mod lists **ModHub** as a dependency, any **enabled** instance of
ModHub, Unified UI, or EMM satisfies that check.

- **`replacement`**: the **one** mod actively recommended and used by the
  missing-dependency add action.
- **`alternatives`**: additional mods that satisfy the *original dependency*.
  They do **not** replace the recommendation or create more prompt options.
- The relationship is **one-way**: ModHub being installed does not satisfy a
  dependency that specifically requires Unified UI.

If there are no other accepted substitutes, omit `alternatives` or use
`"alternatives": []`.

## Additional rules and disabling

Add another object to the `replacements` array to define another independent
replacement. Set its `enabled` to `false` to turn it off without deleting it:

```json
{
  "enabled": true,
  "replacements": [
    {
      "enabled": false,
      "original": "https://mod.io/g/drg/m/old-mod",
      "original_name": "Old Mod",
      "replacement": "https://mod.io/g/drg/m/new-mod",
      "replacement_name": "New Mod",
      "alternatives": [],
      "description": "Use New Mod instead of Old Mod."
    }
  ]
}
```

This is a **configuration example**, not an assertion that those example mod
URLs exist. Setting top-level `"enabled": false` disables every rule; setting
`"replacements": []` removes every recommendation.

## Validation and limitations

- URLs must be DRG mod.io URLs under `https://mod.io/g/drg/m/`. Pinned modfile
  URLs are also accepted. Mod matching uses mod identity, so pinning a version
  does not bypass the rule.
- Each original mod may have only one rule. Duplicate or overlapping originals,
  replacement chains, self-replacements, invalid URLs, and unknown JSON fields
  are rejected.
- Rules are **directional**. Substitutes only satisfy dependencies on the
  original; an original does not automatically satisfy dependencies on its
  replacement.
- Only **enabled** entries in the active profile count toward dependency
  checks, including group members whose group/member settings enable them.
- An invalid embedded configuration makes the shipped-rule validation test
  fail. At runtime, mintfixed logs an error and uses **no replacement rules**,
  rather than risking incorrect mod substitutions.
- Replacement prompts happen when adding mods; existing profile entries are
  not rewritten by rebuilding or editing this JSON.
- JSON rules express *dependency-warning policy*, not an in-game compatibility
  guarantee. Check mod compatibility before marking one as an alternative.
- Do not put mod.io access tokens or other credentials in this JSON.
