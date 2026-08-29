#!/bin/sh
# tokenloom installer — https://github.com/danewalker/tokenloom
#
#   curl -fsSL https://raw.githubusercontent.com/danewalker/tokenloom/main/install.sh | sh
#
# Options (env or flags):
#   TOKENLOOM_VERSION=v0.1.0   pin a release (default: latest)
#   TOKENLOOM_INSTALL_DIR=...  install directory (default: /usr/local/bin if
#                              writable, else $HOME/.local/bin)
#   --with-dsh-plugin          also install the DeepSeek Harness search plugin
#   --prefix DIR               stage under DIR/bin instead of a system path
set -eu

REPO="danewalker/tokenloom"
VERSION="${TOKENLOOM_VERSION:-latest}"
WITH_DSH_PLUGIN=0
PREFIX=""
while [ $# -gt 0 ]; do
	case "$1" in
	--with-dsh-plugin) WITH_DSH_PLUGIN=1 ;;
	--prefix)
		PREFIX="${2:?--prefix needs a value}"
		shift
		;;
	--help | -h)
		sed -n '2,10p' "$0"
		exit 0
		;;
	*)
		echo "tokenloom-install: unknown option: $1" >&2
		exit 1
		;;
	esac
	shift
done

say() { printf '%s\n' "$*" >&2; }
bail() { say "tokenloom-install: $*"; exit 1; }

# ── fetch helper (curl or wget) ──────────────────────────────────────────────
fetch() {
	_url="$1"
	_out="$2"
	if command -v curl >/dev/null 2>&1; then
		curl -fsSL -o "$_out" "$_url" || return 1
	elif command -v wget >/dev/null 2>&1; then
		wget -qO "$_out" "$_url" || return 1
	else
		bail "need curl or wget to download"
	fi
}

# ── resolve version ──────────────────────────────────────────────────────────
if [ "$VERSION" = "latest" ]; then
	VERSION=$(fetch "https://api.github.com/repos/$REPO/releases/latest" - | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n1)
	[ -n "$VERSION" ] || bail "could not resolve the latest release (set TOKENLOOM_VERSION=vX.Y.Z to pin one)"
fi
say "tokenloom-install: installing $VERSION"

# ── detect platform ──────────────────────────────────────────────────────────
OS=$(uname -s)
ARCH=$(uname -m)
case "$OS" in
Linux)
	case "$ARCH" in
	x86_64 | amd64) TARGET="x86_64-unknown-linux-musl" ;;
	aarch64 | arm64) TARGET="aarch64-unknown-linux-musl" ;;
	*) bail "unsupported architecture: $ARCH" ;;
	esac
	EXT="tar.gz"
	;;
Darwin)
	case "$ARCH" in
	arm64) TARGET="aarch64-apple-darwin" ;;
	x86_64) TARGET="x86_64-apple-darwin" ;;
	*) bail "unsupported architecture: $ARCH" ;;
	esac
	EXT="tar.gz"
	;;
MINGW* | MSYS* | CYGWIN* | Windows_NT)
	TARGET="x86_64-pc-windows-msvc"
	EXT="zip"
	;;
*)
	bail "unsupported OS: $OS"
	;;
esac

# ── download + verify ────────────────────────────────────────────────────────
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
BASE="https://github.com/$REPO/releases/download/$VERSION"
ARCHIVE="tokenloom-$VERSION-$TARGET.$EXT"
say "tokenloom-install: downloading $ARCHIVE"
fetch "$BASE/$ARCHIVE" "$TMP/$ARCHIVE" || bail "download failed — does release $VERSION exist?"
fetch "$BASE/$ARCHIVE.sha256" "$TMP/$ARCHIVE.sha256" || say "tokenloom-install: (no checksum published; skipping verification)"
if [ -s "$TMP/$ARCHIVE.sha256" ]; then
	(
		cd "$TMP"
		if command -v sha256sum >/dev/null 2>&1; then
			echo "$(cat "$ARCHIVE.sha256")" | sed "s| .*$|  $ARCHIVE|" | sha256sum -c - >/dev/null 2>&1 || bail "checksum mismatch for $ARCHIVE"
		elif command -v shasum >/dev/null 2>&1; then
			echo "$(cat "$ARCHIVE.sha256")" | sed "s| .*$|  $ARCHIVE|" | shasum -a 256 -c - >/dev/null 2>&1 || bail "checksum mismatch for $ARCHIVE"
		fi
	)
	say "tokenloom-install: checksum OK"
fi

# ── extract ──────────────────────────────────────────────────────────────────
mkdir -p "$TMP/out"
case "$EXT" in
tar.gz) tar -xzf "$TMP/$ARCHIVE" -C "$TMP/out" ;;
zip)
	if command -v unzip >/dev/null 2>&1; then
		unzip -q "$TMP/$ARCHIVE" -d "$TMP/out"
	else
		bail "need unzip for the Windows archive"
	fi
	;;
esac

# ── install the binary ───────────────────────────────────────────────────────
BIN_NAME="tokenloom"
[ -f "$TMP/out/$BIN_NAME" ] && BIN_SRC="$TMP/out/$BIN_NAME"
[ -f "$TMP/out/$BIN_NAME.exe" ] && BIN_SRC="$TMP/out/$BIN_NAME.exe"
[ -n "${BIN_SRC:-}" ] || bail "archive layout unexpected — no tokenloom binary found"

if [ -n "$PREFIX" ]; then
	INSTALL_DIR="$PREFIX/bin"
else
	INSTALL_DIR="${TOKENLOOM_INSTALL_DIR:-}"
	if [ -z "$INSTALL_DIR" ]; then
		if [ -w /usr/local/bin ] || [ "$(id -u)" = "0" ]; then
			INSTALL_DIR="/usr/local/bin"
		else
			INSTALL_DIR="$HOME/.local/bin"
		fi
	fi
fi
mkdir -p "$INSTALL_DIR"
mv "$BIN_SRC" "$INSTALL_DIR/$BIN_NAME"
chmod +x "$INSTALL_DIR/$BIN_NAME"
say "tokenloom-install: installed $INSTALL_DIR/$BIN_NAME"

case ":$PATH:" in
*":$INSTALL_DIR:"*) ;;
*) say "tokenloom-install: NOTE — add to PATH:  export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
esac

# ── optional: DeepSeek Harness search plugin ─────────────────────────────────
if [ "$WITH_DSH_PLUGIN" = "1" ]; then
	PLUGIN_SRC="$TMP/out/integrations/dsh-plugin"
	PLUGIN_DST="${DSH_PROFILES:-$HOME/.dsh}/profiles/node_modules/@dane/dsh-web-search-tokenloom"
	if [ -d "$PLUGIN_SRC" ]; then
		mkdir -p "$(dirname "$PLUGIN_DST")"
		rm -rf "$PLUGIN_DST"
		cp -R "$PLUGIN_SRC" "$PLUGIN_DST"
		say "tokenloom-install: DSH plugin → $PLUGIN_DST"
		say "  next: add the cordis patch entry from the README (DeepSeek Harness section)"
	else
		say "tokenloom-install: (plugin files not in this archive; install from the repo instead)"
	fi
fi

"$INSTALL_DIR/$BIN_NAME" --version 2>/dev/null && say "tokenloom-install: done — weave the web into your context window 🧵"
