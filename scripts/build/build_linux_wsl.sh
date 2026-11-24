#!/usr/bin/env bash
# Native Linux release builds invoked by Windows build.cmd through WSL.
# Arguments are Windows paths for the repository and staging directory, then tag.
set -euo pipefail

if [[ $# -ne 3 ]]; then
    echo "Usage: build_linux_wsl.sh <windows-repository-path> <windows-stage-path> <tag>" >&2
    exit 2
fi

repo="$(wslpath -u "$1")"
stage="$(wslpath -u "$2")"
tag="$3"
if [[ ! "$tag" =~ ^mintfixed_[0-9]+\.[0-9]+(\.[0-9]+)?(-[A-Za-z0-9][A-Za-z0-9.-]*)?$ ]]; then
    echo "Invalid mintfixed release tag: $tag" >&2
    exit 2
fi
if [[ ! -f "$repo/Cargo.toml" || ! -d "$stage" ]]; then
    echo "Invalid repository or staging directory: $repo / $stage" >&2
    exit 2
fi

# rustup may not be on PATH when WSL launches a non-login shell.
if [[ -f "$HOME/.cargo/env" ]]; then
    # shellcheck disable=SC1091
    source "$HOME/.cargo/env"
fi
if ! command -v cargo >/dev/null 2>&1; then
    echo "Cargo is missing in WSL. Install Rust with rustup first." >&2
    exit 1
fi
if ! command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
    echo "MinGW-w64 is missing in WSL. Install gcc-mingw-w64-x86-64." >&2
    exit 1
fi

# Keep Linux build artifacts on the Linux filesystem rather than mixing them
# with Windows target/release (also substantially faster under WSL).
export CARGO_TARGET_DIR="$HOME/mintfixed-target"
export CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc
export AR_x86_64_pc_windows_gnu=x86_64-w64-mingw32-ar
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc
unset MINTFIXED_RELEASE_TAG || true

cd "$repo"
cargo build --release --locked --target x86_64-unknown-linux-gnu

binary="$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/mint"
if [[ ! -s "$binary" ]]; then
    echo "Linux build produced no executable: $binary" >&2
    exit 1
fi
if [[ "$(od -An -tx1 -N4 "$binary" | tr -d '[:space:]')" != '7f454c46' ]]; then
    echo "Build output is not an ELF binary: $binary" >&2
    exit 1
fi

package_linux() {
    local name="$1"
    local raw="$stage/$name"
    local archive="$stage/$name.tar.gz"
    cp "$binary" "$raw"
    # Unlike a Windows-created ZIP, tar retains the Linux executable bit.
    tar -czf "$archive" --transform="s|^mint$|$name|" -C "$(dirname "$binary")" mint
    if [[ ! -s "$raw" || ! -s "$archive" ]]; then
        echo "Linux packaging failed for: $name" >&2
        exit 1
    fi
    if [[ "$(tar -tzf "$archive")" != "$name" ]]; then
        echo "Unexpected Linux archive contents: $archive" >&2
        exit 1
    fi
    echo "Packaged Linux build: $name (+ .tar.gz)"
}

package_linux "${tag}-linux-x86_64"
package_linux 'mintfixed_experimental-linux-x86_64'
