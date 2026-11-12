#!/bin/sh
# Installs skald from its GitHub Release on macOS and Linux: downloads the release's archive for this
# machine's OS and architecture and puts `skald` in ~/.local/bin, or $SKALD_INSTALL_DIR, on the PATH
# of new shells. script/release-installers sets the release tag of a release's copy.
#   curl -fsSL https://github.com/skald-so/skald/releases/download/<tag>/install.sh | sh
# SKALD_DOWNLOAD_URL: where the release's files are instead of its GitHub Release.
set -eu

tag='@RELEASE_TAG@'

fail() {
  echo "install.sh: $*" >&2
  exit 1
}

case $tag in
  v[0-9]*) ;;
  *) fail "this is the template of the installer; run install.sh of a release: https://github.com/skald-so/skald/releases" ;;
esac

arch=$(uname -m)
case $arch in
  x86_64 | amd64) arch=x86_64 ;;
  arm64 | aarch64) arch=aarch64 ;;
  *) fail "no skald for the architecture $arch" ;;
esac
os=$(uname -s)
case $os in
  Darwin)
    # Under Rosetta 2, uname reports x86_64 on Apple silicon.
    if [ "$arch" = x86_64 ] && [ "$(sysctl -n hw.optional.arm64 2>/dev/null || true)" = 1 ]; then
      arch=aarch64
    fi
    target=$arch-apple-darwin
    ;;
  Linux) target=$arch-unknown-linux-gnu ;;
  MINGW* | MSYS* | CYGWIN*) fail "on Windows, run install.ps1 of the release in PowerShell" ;;
  *) fail "no skald for $os" ;;
esac
archive=skald-client-$target.tar.gz
base=${SKALD_DOWNLOAD_URL:-https://github.com/skald-so/skald/releases/download/$tag}

# download <url> <file>: HTTPS only, unless SKALD_DOWNLOAD_URL names another scheme.
download() {
  if command -v curl >/dev/null 2>&1; then
    case $1 in
      https://*) curl --proto '=https' --tlsv1.2 -fsSL "$1" -o "$2" ;;
      *) curl -fsSL "$1" -o "$2" ;;
    esac
  elif command -v wget >/dev/null 2>&1; then
    case $1 in
      https://*) wget --https-only -q -O "$2" "$1" ;;
      *) wget -q -O "$2" "$1" ;;
    esac
  else
    fail "curl or wget is needed to download skald"
  fi
}

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
download "$base/$archive" "$tmp/$archive" || fail "cannot download $base/$archive"
tar -xzf "$tmp/$archive" -C "$tmp" || fail "cannot unpack $archive"

dir=${SKALD_INSTALL_DIR:-$HOME/.local/bin}
mkdir -p "$dir"
# A new file moved over the old one: a running skald keeps its own.
cp "$tmp/skald-client-$target/skald" "$dir/.skald.new"
chmod 755 "$dir/.skald.new"
mv -f "$dir/.skald.new" "$dir/skald"
echo "install.sh: installed skald $tag in $dir"

case :$PATH: in
  *:"$dir":*) exit 0 ;;
esac
# New shells find skald: sh and bash login shells read ~/.profile, unless bash has a
# ~/.bash_profile; interactive bash reads ~/.bashrc; every zsh reads .zshenv.
line="export PATH=\"$dir:\$PATH\""
add_line() {
  if [ ! -f "$1" ] || ! grep -qxF "$line" "$1"; then
    printf '\n# skald\n%s\n' "$line" >>"$1"
    echo "install.sh: added $dir to PATH in $1"
  fi
}
add_line "$HOME/.profile"
if [ -f "$HOME/.bash_profile" ]; then add_line "$HOME/.bash_profile"; fi
if command -v bash >/dev/null 2>&1; then add_line "$HOME/.bashrc"; fi
if command -v zsh >/dev/null 2>&1; then add_line "${ZDOTDIR:-$HOME}/.zshenv"; fi
echo "install.sh: open a new shell to run skald, or run: $line"
