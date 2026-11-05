# Runs a dependency-free Cargo smoke test of version stamping and incremental
# reruns. Does NOT build DRG/mintfixed itself. Requires Git and Rust stable.
$ErrorActionPreference = 'Stop'
$source = (Resolve-Path (Join-Path $PSScriptRoot '..\mint_lib\build_version.rs')).Path
$dir = Join-Path ([IO.Path]::GetTempPath()) ("mintfixed-version-check-" + [guid]::NewGuid().ToString('N'))
$originalTarget = $env:CARGO_TARGET_DIR

function Write-Utf8([string] $path, [string] $content) {
    [IO.File]::WriteAllText($path, $content, [System.Text.UTF8Encoding]::new($false))
}
function Run-Git([string[]] $commandArgs) {
    & git -C $dir @commandArgs | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "Git failed: $($commandArgs -join ' ')" }
}
function Check-Title([string] $expectation) {
    $actual = (& cargo +stable run --quiet) -join "`n"
    if ($LASTEXITCODE -ne 0) { throw 'Cargo build failed' }
    if ($actual -ne $expectation) { throw "Expected '$expectation', got '$actual'" }
    Write-Host "OK: $actual"
}

try {
    New-Item -ItemType Directory -Force -Path (Join-Path $dir 'src') | Out-Null
    Copy-Item $source (Join-Path $dir 'build_version.rs')
    Write-Utf8 -path (Join-Path $dir 'Cargo.toml') -content @'
[package]
name = "mintfixed-version-check"
version = "0.2.10"
edition = "2024"
build = "build.rs"
'@
    Write-Utf8 -path (Join-Path $dir 'build.rs') -content @'
mod build_version;
use std::{env, path::PathBuf};
fn main() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    build_version::emit_rerun_directives(&dir);
    let info = build_version::resolve(&dir, env!("CARGO_PKG_VERSION"), None).unwrap();
    build_version::write_generated(&PathBuf::from(env::var("OUT_DIR").unwrap()), &info).unwrap();
}
'@
    Write-Utf8 -path (Join-Path $dir 'src/main.rs') -content @'
#[allow(dead_code)]
mod info { include!(concat!(env!("OUT_DIR"), "/mintfixed_version.rs")); }
fn main() { println!("{}", info::DISPLAY_VERSION); }
'@
    Write-Utf8 -path (Join-Path $dir '.gitignore') -content 'target/'
    Run-Git -commandArgs @('init', '-q')
    Run-Git -commandArgs @('config', 'user.name', 'Version Check')
    Run-Git -commandArgs @('config', 'user.email', 'version-check@mintfixed.invalid')
    Run-Git -commandArgs @('add', '.')
    Run-Git -commandArgs @('commit', '-qm', 'initial')
    $env:CARGO_TARGET_DIR = Join-Path $dir 'target'
    Push-Location $dir
    try {
        $head = (& git rev-parse --short=7 HEAD).Trim()
        Check-Title "mintfixed dev ($head)"
        Run-Git -commandArgs @('tag', 'mintfixed_1.3')
        Check-Title 'mintfixed v1.3' # New tag on the same HEAD, no cargo clean
        Write-Utf8 -path (Join-Path $dir 'src/main.rs') -content @'
#[allow(dead_code)]
mod info { include!(concat!(env!("OUT_DIR"), "/mintfixed_version.rs")); }
fn main() { println!("{}", info::DISPLAY_VERSION); } // changed
'@
        Check-Title "mintfixed v1.3 ($head, modified)"
        Run-Git -commandArgs @('add', 'src/main.rs')
        Run-Git -commandArgs @('commit', '-qm', 'modify source')
        $head = (& git rev-parse --short=7 HEAD).Trim()
        Check-Title "mintfixed v1.3 +1 commits ($head)"
        # Ref updates through packed-refs and subsequent loose tag creation.
        Run-Git -commandArgs @('pack-refs', '--all', '--prune')
        Check-Title "mintfixed v1.3 +1 commits ($head)"
        Run-Git -commandArgs @('tag', 'mintfixed_1.4-beta.1')
        Check-Title 'mintfixed v1.4-beta.1'
        # A new commit on another branch must not leak its tag into the main branch.
        $main = (& git branch --show-current).Trim()
        Run-Git -commandArgs @('switch', '-q', '-c', 'version-check-side')
        Write-Utf8 -path (Join-Path $dir 'src/main.rs') -content @'
#[allow(dead_code)]
mod info { include!(concat!(env!("OUT_DIR"), "/mintfixed_version.rs")); }
fn main() { println!("{}", info::DISPLAY_VERSION); } // side branch
'@
        Run-Git -commandArgs @('add', 'src/main.rs')
        Run-Git -commandArgs @('commit', '-qm', 'side branch')
        Run-Git -commandArgs @('tag', 'mintfixed_9.9')
        Check-Title 'mintfixed v9.9'
        Run-Git -commandArgs @('switch', '-q', $main)
        Check-Title 'mintfixed v1.4-beta.1'
    }
    finally { Pop-Location }
    Write-Host 'Incremental Cargo version stamping checks passed.'
}
finally {
    $env:CARGO_TARGET_DIR = $originalTarget
    Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
}
