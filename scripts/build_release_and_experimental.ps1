# Build two genuine source versions of mintfixed (latest release tag and current
# checkout), and package each one as an EXE plus a ZIP containing that EXE.
# Usage from repository root: .\build.cmd
# Offline (use locally fetched tags): .\build.cmd -NoFetch
param(
    [switch] $NoFetch,
    [switch] $NoBuild,
    [string] $Tag = '',
    [string] $OutputDirectory = ''
)

$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$releaseRoot = Join-Path $repo 'target\release'
$dist = Join-Path $releaseRoot 'dist'
# A persistent, ignored worktree + Cargo target preserve incremental builds.
$stableTree = Join-Path $repo 'target\mintfixed-stable-src'
$stableTarget = Join-Path $repo 'target\mintfixed-stable-build'
$normalTarget = Join-Path $repo 'target'
$originalTarget = $env:CARGO_TARGET_DIR
$originalReleaseTag = $env:MINTFIXED_RELEASE_TAG
$stage = $null

function Assert-Success([string] $description) {
    if ($LASTEXITCODE -ne 0) {
        throw "$description failed (exit code $LASTEXITCODE)"
    }
}

function Package-Exe([string] $source, [string] $baseName, [string] $folder) {
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
        throw "Expected build output was not found: $source"
    }
    $exe = Join-Path $folder ($baseName + '.exe')
    $zip = Join-Path $folder ($baseName + '.zip')
    Copy-Item -LiteralPath $source -Destination $exe -Force
    # The archive contains only the correctly named EXE at its root.
    Compress-Archive -LiteralPath $exe -DestinationPath $zip -CompressionLevel Optimal -Force
    if (-not (Test-Path -LiteralPath $zip -PathType Leaf)) {
        throw "Failed to create ZIP: $zip"
    }
}

Push-Location $repo
try {
    # GitHub Actions compiles the exact tag before packaging. In this mode,
    # reuse the compiled binary instead of rebuilding both local variants.
    if ($NoBuild) {
        if (-not $Tag -or $Tag -notmatch '^(?:mintfixed_[0-9]+\.[0-9]+(?:\.[0-9]+)?(?:-[A-Za-z0-9][A-Za-z0-9.-]*)?|v0\.2\.10-mintfixed-[0-9]+)$') {
            throw '-NoBuild requires a valid, explicit -Tag (mintfixed_X.Y or legacy tag).'
        }
        $tagCommit = @(& git rev-parse --verify "refs/tags/${Tag}^{commit}" 2>$null)
        Assert-Success 'Release tag lookup'
        $headCommit = @(& git rev-parse HEAD)
        Assert-Success 'Current commit lookup'
        if ($tagCommit.Count -ne 1 -or $headCommit.Count -ne 1 -or
            $tagCommit[0].Trim() -cne $headCommit[0].Trim()) {
            throw "Release tag $Tag does not point at HEAD."
        }
        $changes = @(& git status --porcelain=v1 --untracked-files=all)
        Assert-Success 'Release worktree status'
        if ($changes.Count -gt 0) {
            throw 'Release checkout has uncommitted or untracked files.'
        }
        $binary = Join-Path $normalTarget 'release\mint.exe'
        if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
            throw "Missing previously compiled executable: $binary"
        }
        $versionLines = @(& $binary --version)
        Assert-Success 'Reading executable version'
        $actualVersion = if ($versionLines.Count -eq 1) { ([string]$versionLines[0]).Trim() } else { '' }
        $expectedVersion = if ($Tag -match '^mintfixed_(.+)$') {
            'mintfixed v' + $Matches[1]
        } else {
            'mintfixed ' + $Tag
        }
        if ($actualVersion -cne $expectedVersion) {
            throw "Compiled version mismatch: expected '$expectedVersion', got '$actualVersion'."
        }
        $destinationDirectory = if ($OutputDirectory) {
            if ([IO.Path]::IsPathRooted($OutputDirectory)) { $OutputDirectory }
            else { Join-Path $repo $OutputDirectory }
        } else { $dist }
        New-Item -ItemType Directory -Path $destinationDirectory -Force | Out-Null
        $destination = Join-Path $destinationDirectory ($Tag + '.exe')
        Copy-Item -LiteralPath $binary -Destination $destination -Force
        Write-Host "Verified release executable: $destination" -ForegroundColor Green
        return
    }
    if ($Tag -or $OutputDirectory) {
        throw '-Tag and -OutputDirectory are only supported together with -NoBuild.'
    }

    & git rev-parse --is-inside-work-tree | Out-Null
    Assert-Success 'Repository check'
    & cargo --version | Out-Null
    Assert-Success 'Cargo check'

    if (-not $NoFetch) {
        Write-Host 'Fetching the latest tags from origin...'
        & git fetch --tags origin
        Assert-Success 'git fetch --tags origin'
    }

    # Version sorting chooses mintfixed_1.10 before mintfixed_1.9. Ignore
    # historical upstream/legacy tags that aren't mintfixed release versions.
    $tagCandidates = @(& git tag --list 'mintfixed_*' --sort=-version:refname)
    Assert-Success 'Tag lookup'
    $latestTag = $tagCandidates |
        Where-Object { $_ -match '^mintfixed_[0-9]+\.[0-9]+(?:\.[0-9]+)?(?:-[a-zA-Z0-9][a-zA-Z0-9.-]*)?$' } |
        Select-Object -First 1
    if (-not $latestTag) {
        throw 'No mintfixed_X.Y[.Z] Git tag found. Fetch or create a release tag first.'
    }
    Write-Host "Latest release tag: $latestTag"

    # Keep the tagged source out of the user's current beta checkout. Reuse
    # the same worktree so a second run does not recompile everything.
    if (Test-Path -LiteralPath $stableTree) {
        $worktreeRoot = @(& git -C $stableTree rev-parse --show-toplevel)
        Assert-Success 'Existing tagged worktree check'
        if (-not $worktreeRoot -or
            [IO.Path]::GetFullPath($worktreeRoot[0]).TrimEnd('\') -ine
            [IO.Path]::GetFullPath($stableTree).TrimEnd('\')) {
            throw "Refusing to modify an unexpected directory: $stableTree"
        }
        $changes = @(& git -C $stableTree status --porcelain --untracked-files=all)
        Assert-Success 'Tagged worktree status check'
        if ($changes.Count -gt 0) {
            throw "Tagged build worktree has local changes: $stableTree (inspect it before continuing)"
        }
        & git -C $stableTree checkout --detach "refs/tags/$latestTag"
        Assert-Success 'Switching tagged build worktree'
    } else {
        New-Item -ItemType Directory -Path $normalTarget -Force | Out-Null
        & git worktree add --detach $stableTree "refs/tags/$latestTag"
        Assert-Success 'Creating tagged build worktree'
    }

    $tagCommit = @(& git -C $stableTree rev-parse HEAD)
    Assert-Success 'Tagged commit lookup'
    Write-Host "Building release source $latestTag ($($tagCommit[0].Substring(0,7)))..."
    $env:CARGO_TARGET_DIR = $stableTarget
    $env:MINTFIXED_RELEASE_TAG = $latestTag
    Push-Location $stableTree
    try {
        & cargo build --release
        Assert-Success "Release build ($latestTag)"
    } finally {
        Pop-Location
    }
    $releaseExe = Join-Path $stableTarget 'release\mint.exe'

    # Current checkout is *not* given a release tag; its own compile-time
    # version metadata continues to report commit distance / modifications.
    Write-Host 'Building experimental source from the current checkout...'
    $env:CARGO_TARGET_DIR = $normalTarget
    Remove-Item Env:MINTFIXED_RELEASE_TAG -ErrorAction SilentlyContinue
    & cargo build --release
    Assert-Success 'Experimental build'
    $experimentalExe = Join-Path $normalTarget 'release\mint.exe'

    # Package into a staging directory, then replace the four previous
    # distributables as a group only after both builds have succeeded.
    New-Item -ItemType Directory -Path $releaseRoot -Force | Out-Null
    $stage = Join-Path $releaseRoot ('.mintfixed-dist-new-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $stage -Force | Out-Null
    Package-Exe $releaseExe $latestTag $stage
    Package-Exe $experimentalExe 'mintfixed_experimental' $stage

    $oldDist = Join-Path $releaseRoot ('.mintfixed-dist-old-' + [guid]::NewGuid().ToString('N'))
    $hadOldDist = Test-Path -LiteralPath $dist
    if ($hadOldDist) {
        Move-Item -LiteralPath $dist -Destination $oldDist
    }
    try {
        Move-Item -LiteralPath $stage -Destination $dist
        $stage = $null
    } catch {
        if ($hadOldDist -and -not (Test-Path -LiteralPath $dist)) {
            Move-Item -LiteralPath $oldDist -Destination $dist
        }
        throw
    }
    if ($hadOldDist) {
        Remove-Item -LiteralPath $oldDist -Recurse -Force
    }

    Write-Host ''
    Write-Host 'Build complete. Distributables:' -ForegroundColor Green
    Get-ChildItem -LiteralPath $dist -File | Sort-Object Name |
        ForEach-Object { Write-Host ('  ' + $_.FullName) }
} finally {
    $env:CARGO_TARGET_DIR = $originalTarget
    $env:MINTFIXED_RELEASE_TAG = $originalReleaseTag
    if ($stage -and (Test-Path -LiteralPath $stage)) {
        Remove-Item -LiteralPath $stage -Recurse -Force
    }
    Pop-Location
}
