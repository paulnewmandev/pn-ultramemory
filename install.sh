#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# Purpose: install the pn-ultramemory release for this machine in one command, checked against its
#          published SHA-256, into a folder on your PATH.
# Usage:   curl -fsSL https://raw.githubusercontent.com/paulnewmandev/pn-ultramemory/main/install.sh | sh
#
# Settings, all optional, as environment variables:
#   PN_ULTRAMEMORY_VERSION   a release tag such as v1.1.0 (default: the latest release)
#   PN_ULTRAMEMORY_BIN_DIR   where to put the binary     (default: $HOME/.local/bin)
#
# What it does, and nothing else: download one archive and its checksum from the GitHub releases of
# this repository, refuse to continue if they do not match, and copy one binary into the folder
# above. It needs no root, changes no shell profile and sends nothing anywhere: it only tells you the
# line to add when the folder is not on your PATH.
#
# Windows is not covered here: download the .zip from the releases page instead.

set -eu

repo="paulnewmandev/pn-ultramemory"
version="${PN_ULTRAMEMORY_VERSION:-latest}"
bin_dir="${PN_ULTRAMEMORY_BIN_DIR:-$HOME/.local/bin}"

say() {
    printf 'pn-ultramemory: %s\n' "$1"
}

fail() {
    printf 'pn-ultramemory: %s\n' "$1" >&2
    exit 1
}

# Downloads a URL to a file with whichever of curl or wget is present.
download() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$1" -o "$2"
    elif command -v wget >/dev/null 2>&1; then
        wget -q "$1" -O "$2"
    else
        fail "neither curl nor wget is installed; install one of them and run this again"
    fi
}

# Prints the SHA-256 of a file with whichever tool the system has.
sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        fail "no SHA-256 tool (sha256sum or shasum) found, so the download cannot be checked"
    fi
}

os=$(uname -s)
arch=$(uname -m)
case "$os/$arch" in
    Darwin/arm64 | Darwin/aarch64) target="aarch64-apple-darwin" ;;
    Darwin/x86_64) target="x86_64-apple-darwin" ;;
    Linux/x86_64 | Linux/amd64) target="x86_64-unknown-linux-gnu" ;;
    *)
        fail "there is no release built for $os/$arch; build it from source with \`cargo build --release\` (see https://github.com/$repo#start-in-two-minutes)"
        ;;
esac

if [ "$version" = "latest" ]; then
    base="https://github.com/$repo/releases/latest/download"
else
    base="https://github.com/$repo/releases/download/$version"
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT INT TERM

say "downloading the $version release for $target"
download "$base/pn-ultramemory-$target.tar.gz" "$work/archive.tar.gz" ||
    fail "could not download $base/pn-ultramemory-$target.tar.gz; check the version and your connection"
download "$base/pn-ultramemory-$target.sha256" "$work/archive.sha256" ||
    fail "could not download the checksum of the release, so it cannot be checked; try again"

expected=$(awk '{print $1; exit}' "$work/archive.sha256")
actual=$(sha256_of "$work/archive.tar.gz")
if [ -z "$expected" ] || [ "$expected" != "$actual" ]; then
    fail "the download does not match its published SHA-256, so nothing was installed; try again, and report it at https://github.com/$repo/issues if it keeps happening"
fi

tar xzf "$work/archive.tar.gz" -C "$work"
mkdir -p "$bin_dir"
cp "$work/pn-ultramemory-$target/pn-ultramemory" "$bin_dir/pn-ultramemory"
chmod 755 "$bin_dir/pn-ultramemory"

installed=$("$bin_dir/pn-ultramemory" --version) ||
    fail "the binary was copied to $bin_dir but does not run on this machine; build it from source with \`cargo build --release\`"
say "installed $installed at $bin_dir/pn-ultramemory"

case ":$PATH:" in
    *":$bin_dir:"*) ;;
    *)
        say "$bin_dir is not on your PATH yet; add this line to your shell profile (~/.zshrc or ~/.bashrc) and open a new terminal:"
        # The single quotes are deliberate: `$PATH` must be printed as written, for the profile.
        # shellcheck disable=SC2016
        printf '\n    export PATH="%s:$PATH"\n\n' "$bin_dir"
        ;;
esac

cat <<'NEXT'
Next, from the root of your project:

    pn-ultramemory index                            build the graph
    pn-ultramemory install --agents claude-code     connect your agent (or cursor, codex, gemini, …), then restart it
    pn-ultramemory brain                            see it as a brain, in your browser
    pn-ultramemory doctor                           check that everything is in place
NEXT
