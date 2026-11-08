# Build the current checkout once, then publish the SAME binary twice:
# latest mintfixed tag name and mintfixed_experimental, each as EXE and ZIP.
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
    # The latest tag supplies a FILENAME only. Never check out that older
    # tag or build a second executable from it.
    $headCommit = @(& git rev-parse HEAD)
    Assert-Success 'Current commit lookup'
    Write-Host "Building current checkout ($($headCommit[0].Substring(0, 7))) once..."
    $env:CARGO_TARGET_DIR = $normalTarget
    Remove-Item Env:MINTFIXED_RELEASE_TAG -ErrorAction SilentlyContinue
    $compiledExe = Join-Path $normalTarget 'release\mint.exe'
    # Force Cargo to recreate the executable, while retaining cached dependencies.
    if (Test-Path -LiteralPath $compiledExe -PathType Leaf) {
        Remove-Item -LiteralPath $compiledExe -Force
    }
    & cargo build --release --locked
    Assert-Success 'Current checkout build'
    if (-not (Test-Path -LiteralPath $compiledExe -PathType Leaf)) {
        throw "Cargo did not create the expected executable: $compiledExe"
    }

    # Stage all four outputs before replacing the previous dist directory.
    New-Item -ItemType Directory -Path $releaseRoot -Force | Out-Null
    $stage = Join-Path $releaseRoot ('.mintfixed-dist-new-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $stage -Force | Out-Null
    Package-Exe $compiledExe $latestTag $stage
    Package-Exe $compiledExe 'mintfixed_experimental' $stage

    # Verify that the two distributable EXEs are byte-for-byte identical
    # to the newly compiled source executable (not a stale tagged build).
    $compiledHash = (Get-FileHash -LiteralPath $compiledExe -Algorithm SHA256).Hash
    foreach ($name in @($latestTag, 'mintfixed_experimental')) {
        $packagedExe = Join-Path $stage ($name + '.exe')
        $packagedHash = (Get-FileHash -LiteralPath $packagedExe -Algorithm SHA256).Hash
        if ($compiledHash -cne $packagedHash) {
            throw "Build verification failed: $packagedExe differs from $compiledExe"
        }
    }
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
