# UI asset and renderer notices

## Material Symbols

The icons in `assets/icons/*.svg` are Google Material Symbols Rounded,
outlined, weight 400, optical size 20, licensed under Apache-2.0.

Source: https://github.com/google/material-design-icons

Revision: `737e3324305806514d7909874fa1818ae1808232`

Upstream path: `symbols/web/<name>/materialsymbolsrounded/<name>_20px.svg`

Included symbols: add, content_copy, dark_mode, delete, desktop_windows,
drag_handle, error, file_copy, folder, language, light_mode, settings, warning.

Modification: an explicit white fill was added for runtime theme tinting.
The original vector geometry is unchanged. Each file identifies the change.
The full Apache-2.0 license is in `assets/icons/LICENSE`.

## SVG renderer

The icons are embedded in the executable and rendered locally with resvg.
Rasterized textures are cached at the current display scale; SVG files remain
the editable sources. No network request is needed to display icons.

resvg and usvg are used under their MIT license option. Text rendering,
system font discovery, and raster image decoding features are disabled.
License and copyright texts for packages introduced or updated by this
renderer dependency are in `assets/icons/DEPENDENCY_LICENSES.txt`.
These are supplementary notices for the UI changes, not an inventory of all
existing application dependencies.

These notices and license texts are also embedded in the application under
Settings > Third-party notices.
