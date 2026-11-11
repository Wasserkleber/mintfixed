# mintfixed

**An actively maintained, feature-rich fork of [MINT](https://github.com/trumank/mint) for Deep Rock Galactic.**

Mintfixed is an external mod manager that allows you to download, organize, and install mods independently of Deep Rock Galactic's built-in mod integration.

Originally forked from MINT after its development stopped, Mintfixed introduces new features, performance optimizations, compatibility fixes, and stability improvements.

Supports both **Steam** and **Microsoft Store** versions of Deep Rock Galactic.

## Download & Installation

- **[Download the Latest Release](https://github.com/Wasserkleber/mintfixed/releases/latest)**
- **[Mintfixed Installation & Usage Guide](https://mod.io/g/drg/r/mint-installation-usage-guide-mintfixed)**

## Major Features

- **Mod Groups** — Organize mods into collapsible groups with drag-and-drop, group toggles, and import/export functionality.
- **97% Lower Background CPU Usage** — Significantly reduced idle CPU usage through GUI performance optimizations.
- **No `-disablemodding` Required** — Removes the need for the additional Steam launch parameter.
- **Custom In-Game Mod Menu** — Adds a dedicated in-game interface for Mintfixed's mod integration.
- **Integrated Crash Reporter** — Automatically generates detailed crash reports to help diagnose game crashes and mod compatibility issues.
- **Bitfix Support** — Supports Bitfix DLL integration and Lua scripts for advanced game modifications beyond traditional `.pak` mods.

## Additional Improvements Over MINT

### Mod Management

- Improved profile management, including renaming, duplication, import, and export.
- Bulk importing of local `.pak` and `.zip` mods.
- Improved searching, sorting, and drag-and-drop functionality.
- Dedicated native DLL management.
- Offline mod support.

### Compatibility & Stability

- Updated mod.io integration and improved download reliability.
- Blueprint compatibility checks to prevent certain crashes.
- Improved mod packaging and Oodle compression support.
- Configuration backups and improved migration handling.
- Additional diagnostic logging and error reporting.

### Quality of Life

- Improved and modernized user interface.
- More intuitive mod and group management.
- Better error messages and notifications.
- Improved group and profile import/export workflows.
- Numerous bug fixes and performance improvements.

## Issues & Feedback

Found a bug, experienced a crash, or have a feature suggestion?

**[Report an Issue on GitHub](https://github.com/Wasserkleber/mintfixed/issues)**

Please include relevant logs, reproduction steps, and affected mods whenever possible.

## Credits

Mintfixed builds upon the work of several open-source projects:

- **[trumank/MINT](https://github.com/trumank/mint)** — Original external mod manager and integration system for Deep Rock Galactic, developed by Truman and contributors.
- **[trumank/bitfix](https://github.com/trumank/bitfix)** — Lua-scriptable runtime binary patcher developed by Truman.
- **[Elytras/BP-Crash-Reporter](https://github.com/Elytras/BP-Crash-Reporter)** — Unreal Engine 4 crash reporting tool with detailed Blueprint diagnostics, developed by Elytras.

Special thanks to the original developers and contributors of these projects. Mintfixed builds upon their work with continued maintenance, additional features, compatibility updates, and performance improvements.
