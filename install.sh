#!/bin/sh
#
# Runspace installer.
#
#   curl -fsSL https://raw.githubusercontent.com/h4nxxlmao/runspace/main/install.sh | sh
#
# Prebuilt release assets are expected to be named:
#   runspace-x86_64-unknown-linux-gnu
#   runspace-aarch64-unknown-linux-gnu
#
# Override the source repository with RUNSPACE_REPO=owner/repo, pin a tag with
# RUNSPACE_VERSION=v0.1.0, or choose a directory with RUNSPACE_INSTALL_DIR.

set -eu

REPO="${RUNSPACE_REPO:-h4nxxlmao/runspace}"
VERSION="${RUNSPACE_VERSION:-latest}"
INSTALL_DIR="${RUNSPACE_INSTALL_DIR:-$HOME/.local/bin}"

say() {
    printf '%s\n' "$*"
}

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

need() {
    command -v "$1" >/dev/null 2>&1 || die "$1 is required"
}

case "$(uname -s 2>/dev/null || true)" in
    Linux) ;;
    *) die "This installer supports Linux. On Windows use: irm https://raw.githubusercontent.com/h4nxxlmao/runspace/main/install.ps1 | iex" ;;
esac

ARCH="$(uname -m 2>/dev/null || true)"
case "$ARCH" in
    x86_64 | amd64) TARGET="x86_64-unknown-linux-gnu" ;;
    aarch64 | arm64) TARGET="aarch64-unknown-linux-gnu" ;;
    *) die "unsupported architecture: $ARCH" ;;
esac

need mkdir
need mktemp
need uname

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/runspace-install.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT

if [ "$VERSION" = "latest" ]; then
    URL="https://github.com/$REPO/releases/latest/download/runspace-$TARGET"
else
    URL="https://github.com/$REPO/releases/download/$VERSION/runspace-$TARGET"
fi

download() {
    src="$1"
    dst="$2"
    rm -f "$dst"
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$src" -o "$dst"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO "$dst" "$src"
    else
        return 1
    fi
}

mkdir -p "$TMP_DIR"

if download "$URL" "$TMP_DIR/runspace" \
    && chmod +x "$TMP_DIR/runspace" \
    && "$TMP_DIR/runspace" --version >/dev/null 2>&1; then
    say "Downloaded runspace for $TARGET"
else
    rm -f "$TMP_DIR/runspace"
    say "No working prebuilt binary at $URL"
    say "Falling back to a source build with cargo..."
    need cargo
    cargo install --git "https://github.com/$REPO.git" runspace --locked --root "$TMP_DIR/cargo"
    cp "$TMP_DIR/cargo/bin/runspace" "$TMP_DIR/runspace"
    chmod +x "$TMP_DIR/runspace"
fi

mkdir -p "$INSTALL_DIR"
mv -f "$TMP_DIR/runspace" "$INSTALL_DIR/runspace"

say ""
say "Installed $INSTALL_DIR/runspace"
"$INSTALL_DIR/runspace" --version

case ":${PATH:-}:" in
    *":$INSTALL_DIR:"*) ;;
    *)
        say ""
        say "Add this directory to your PATH:"
        say "  export PATH=\"$INSTALL_DIR:\$PATH\""
        ;;
esac

say ""
say "Runspace needs the X11 tools. On Debian/Ubuntu:"
say "  sudo apt-get install -y xvfb xserver-xephyr xdotool openbox x11-xserver-utils"
say "Then run: runspace doctor"
