# Mintfixed — build and release commands

This guide covers the **Windows build workflow** for the `beta` branch. Run the commands in PowerShell from the repository root:

```powershell
cd C:\Users\Wasserkleber\mint
```

## Quick build: release + experimental

```powershell
.\build.cmd
```

This is the recommended local build command. `build.cmd` starts `scripts/build_release_and_experimental.ps1` with a process-local PowerShell execution-policy override; no machine-wide policy change is needed.

The builder:

1. Fetches tags from `origin` (unless you use `-NoFetch`).
2. Finds the highest version-numbered tag matching `mintfixed_X.Y`, `mintfixed_X.Y.Z`, or a supported prerelease suffix (for example `mintfixed_1.4-beta.1`).
3. Builds the **current checkout once**, including any local code changes. The tag is used for naming, not for selecting the code to build.
4. Packages the same Windows executable under release-labeled and experimental names, each with a matching ZIP.
5. Builds and packages Linux x86_64 through WSL too (unless you pass `-SkipLinux`).
6. Replaces the previous `target\release\dist` files only after packaging succeeds.

### Output files

For example, if the newest tag is `mintfixed_1.4`:

```text
target\release\dist\
├── mintfixed_1.4-windows-x86_64.exe
├── mintfixed_1.4-windows-x86_64.zip
├── mintfixed_experimental-windows-x86_64.exe
├── mintfixed_experimental-windows-x86_64.zip
├── mintfixed_1.4-linux-x86_64
├── mintfixed_1.4-linux-x86_64.tar.gz
├── mintfixed_experimental-linux-x86_64
└── mintfixed_experimental-linux-x86_64.tar.gz
```

Both Windows variants contain the **same build of your current checkout**; the latest tag supplies only the release-labeled filenames. Linux variants likewise contain the same current-checkout Linux build. For an official release, build and publish from the actual tagged commit.

To view the artifacts:

```powershell
Get-ChildItem .\target\release\dist\
```

To check the embedded version in the experimental build:

```powershell
.\target\release\dist\mintfixed_experimental-windows-x86_64.exe --version
```

To build **offline**, using tags already present locally:

```powershell
.\build.cmd -NoFetch
```

### Build locations

| Location | Purpose |
| --- | --- |
| `target\release\dist\` | Eight finished distributables (four with `-SkipLinux`); regenerated each successful run |
| `target\release\mint.exe` | Cargo's current-checkout Windows build output (not an export artifact) |
| WSL `~/mintfixed-target/x86_64-unknown-linux-gnu/release/mint` | Cargo's current-checkout Linux build output |

The script uses the current checkout for both release-labeled and experimental artifacts. Windows and WSL use separate Cargo target directories to preserve build caches.

## Check the current checkout and tags

```powershell
git status -sb
git log -1 --oneline
git tag --points-at HEAD
git tag --list 'mintfixed_*' --sort=-version:refname
git fetch origin --tags
```

You can stay on `beta` while producing local builds. These are builds of your **current checkout**, regardless of the tag used in their filenames.

## Prepare and tag a release

Review the changes, and commit only what you intend to publish:

```powershell
git status
git diff
git diff --cached
# Stage the files you actually want to commit:
git add <paths>
git commit -m "Prepare mintfixed v1.4"
git push origin beta
```

Replace `<paths>` with actual file or directory paths. Run `git diff --cached` before committing. A released tag should point at a reviewed, committed version, not uncommitted local edits.

Create and push an **annotated tag** for a stable release:

```powershell
git tag -a mintfixed_1.4 -m "mintfixed v1.4"
git push origin mintfixed_1.4
```

Or create a prerelease tag (use this **instead** of the stable tag):

```powershell
git tag -a mintfixed_1.4-beta.1 -m "mintfixed v1.4 Beta 1"
git push origin mintfixed_1.4-beta.1
```

Run `build.cmd` to package your current checkout under release-labeled and experimental filenames:

```powershell
.\build.cmd
```

> **Important:** The local builder chooses the *highest matching version-numbered tag in your locally available Git tags*, not necessarily the most recently created tag or a tag pointing at your current `HEAD`. Review `git tag --list 'mintfixed_*' --sort=-version:refname` if the release filename is unexpected.

## Publish on GitHub

Create/publish a release for the intended tag at [Mintfixed GitHub Releases](https://github.com/Wasserkleber/mintfixed/releases/new). Mark prereleases appropriately. Upload the tagged ZIP from `target\release\dist\` when manually distributing a release. Use `mintfixed_experimental-windows-x86_64.zip` for Windows test builds, not as an official tagged release.

The GitHub Actions release workflow also uses `scripts/build_release_and_experimental.ps1` to package its already-compiled Windows executable. The legacy `build_tagged.ps1` has been removed; CI uses the packaging-only mode described below.

## GitHub Actions packaging mode (not for normal local builds)

The GitHub Actions workflow first checks out the published tag and compiles the executable, then invokes the same script **without rebuilding**:

```powershell
.\scripts\build_release_and_experimental.ps1 -NoBuild -Tag mintfixed_1.4 -OutputDirectory dist
```

This checks that the tag points to `HEAD`, that the checkout is clean, and that `mint.exe --version` exactly matches the requested tag. It copies the already-compiled binary to `dist\mintfixed_1.4-windows-x86_64.exe`. GitHub Actions then creates its published Windows ZIP and uploads the artifacts. This CI-only call produces the tagged EXE, **not** all four local build artifacts.

`-NoBuild`, `-Tag` and `-OutputDirectory` are CI packaging arguments; they do **not** produce the experimental build. For ordinary local development and export, use only `build.cmd`.

## Optional: verify version-number stamping

`scripts/verify_version_rebuild.ps1` is **not** a second Mintfixed builder. It is a standalone smoke test for `mint_lib/build_version.rs`. It creates a small temporary Rust project and checks that the embedded version string changes correctly after adding tags, modifying files, making commits, and switching branches. It does not build Mintfixed itself, and the normal `build.cmd` workflow does not invoke it.

To run this version-stamping check separately (requires Git and the Rust **stable** toolchain):

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\verify_version_rebuild.ps1
```

Keep this test script even though the old `build_tagged.ps1` has been removed.

## Troubleshooting

| Symptom | What to check |
| --- | --- |
| `build.cmd` not found | Run it from the repository root. Apply the dual-build patch if it has not been added yet. |
| No `mintfixed_...` tag found | Run `git fetch origin --tags`, or create a correctly formatted release tag. |
| Git fetch fails or you are offline | Use `.\build.cmd -NoFetch` if you already have local tags. |
| Tagged worktree reports local changes | Inspect `target\mintfixed-stable-src\`; do not force-remove changes without checking them. |
| An `.exe` is in use / copying fails | Close the running copy of Mintfixed or any process holding the distribution file, then rerun. |
| Rust compilation fails | Check the compiler output and active toolchain with `rustup show active-toolchain`; this project specifies a toolchain in `rust-toolchain.toml`. |
| Versioned EXE is not in `target\release\` | The **exported files are in `target\release\dist\`**, not directly in `target\release\`. |

You normally do **not** need `cargo clean` when building a new tag: the script manages separate build directories automatically.
