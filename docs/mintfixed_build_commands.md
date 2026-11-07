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
3. Builds the **actual source at that tag** from a separate, reusable Git worktree. It does not change your current branch.
4. Builds the **current checkout** as experimental, including any local code changes.
5. Packages each executable into a ZIP containing only that executable.
6. Replaces the previous `target\release\dist` distribution files only after both builds and both ZIPs succeed.

### Output files

For example, if the newest tag is `mintfixed_1.4`:

```text
target\release\dist\
├── mintfixed_1.4.exe
├── mintfixed_1.4.zip
├── mintfixed_experimental.exe
└── mintfixed_experimental.zip
```

The tagged `.exe` and `.zip` come from the **tagged release code**. The experimental pair comes from **your current checkout**, even when it is ahead of that tag. Renaming an experimental executable is not a release build.

To view the artifacts:

```powershell
Get-ChildItem .\target\release\dist\
```

To check the embedded version in the experimental build:

```powershell
.\target\release\dist\mintfixed_experimental.exe --version
```

To build **offline**, using tags already present locally:

```powershell
.\build.cmd -NoFetch
```

### Build locations

| Location | Purpose |
| --- | --- |
| `target\release\dist\` | Four finished distributables; regenerated each successful run |
| `target\release\mint.exe` | Cargo's experimental build output (not an export artifact) |
| `target\mintfixed-stable-src\` | Reusable detached worktree checked out at the highest matching version tag |
| `target\mintfixed-stable-build\release\mint.exe` | Cargo's tagged build output |

The stable worktree must have no local changes; the script stops rather than overwriting them. The two Cargo target directories help retain incremental build caches. Do not manually edit the stable worktree.

## Check the current checkout and tags

```powershell
git status -sb
git log -1 --oneline
git tag --points-at HEAD
git tag --list 'mintfixed_*' --sort=-version:refname
git fetch origin --tags
```

You **do not** need to check out a tag to build its release artifacts: `build.cmd` does that in its separate worktree. You can stay on `beta` while building both versions.

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

Run `build.cmd` to build the tagged release and your experimental checkout together:

```powershell
.\build.cmd
```

> **Important:** The local builder chooses the *highest matching version-numbered tag in your locally available Git tags*, not necessarily the most recently created tag or a tag pointing at your current `HEAD`. Review `git tag --list 'mintfixed_*' --sort=-version:refname` if the release filename is unexpected.

## Publish on GitHub

Create/publish a release for the intended tag at [Mintfixed GitHub Releases](https://github.com/Wasserkleber/mintfixed/releases/new). Mark prereleases appropriately. Upload the tagged ZIP from `target\release\dist\` when manually distributing a release. Use `mintfixed_experimental.zip` for test builds, not as an official tagged release.

The GitHub Actions release workflow also uses `scripts/build_release_and_experimental.ps1` to package its already-compiled Windows executable. The legacy `build_tagged.ps1` has been removed; CI uses the packaging-only mode described below.

## GitHub Actions packaging mode (not for normal local builds)

The GitHub Actions workflow first checks out the published tag and compiles the executable, then invokes the same script **without rebuilding**:

```powershell
.\scripts\build_release_and_experimental.ps1 -NoBuild -Tag mintfixed_1.4 -OutputDirectory dist
```

This checks that the tag points to `HEAD`, that the checkout is clean, and that `mint.exe --version` exactly matches the requested tag. It copies the already-compiled binary to `dist\mintfixed_1.4.exe`. GitHub Actions then creates its published Windows ZIP and uploads the artifacts. This CI-only call produces the tagged EXE, **not** all four local build artifacts.

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
