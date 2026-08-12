#!/bin/sh
# Installs Leteo on Linux or macOS.
#
#   curl -fsSL https://raw.githubusercontent.com/asanabrial/leteo/main/scripts/install.sh | sh
#
# Downloads the release archive for this machine, checks it against the
# published SHA-256 sums, and puts the binary somewhere on the path. Nothing is
# installed if the checksum does not match.
#
# Plain POSIX sh on purpose: this has to run before anything is installed, on
# whatever the machine happens to have.

set -eu

REPO="asanabrial/leteo"
# Where the binary goes. A user-owned directory by default, so no sudo.
INSTALL_DIR="${LETEO_INSTALL_DIR:-$HOME/.local/bin}"
VERSION="${LETEO_VERSION:-latest}"

say() { printf '%s\n' "$*"; }
fail() { printf 'error: %s\n' "$*" >&2; exit 1; }

need() {
    command -v "$1" >/dev/null 2>&1 || fail "this needs $1, which is not installed"
}

target_triple() {
    kernel="$(uname -s)"
    machine="$(uname -m)"
    case "$kernel" in
        Linux)  os="unknown-linux-gnu" ;;
        Darwin) os="apple-darwin" ;;
        # Git Bash, MSYS2 and Cygwin all run on Windows, where the release is a
        # zip and the PATH lives in the registry. Send them to the right script
        # rather than to a build they did not ask for.
        MINGW*|MSYS*|CYGWIN*)
            fail "on Windows, run instead:
    irm https://raw.githubusercontent.com/$REPO/main/scripts/install.ps1 | iex" ;;
        *) fail "no prebuilt Leteo for $kernel; build from source with 'cargo install --git https://github.com/$REPO'" ;;
    esac
    case "$machine" in
        x86_64|amd64)  arch="x86_64" ;;
        arm64|aarch64) arch="aarch64" ;;
        *) fail "no prebuilt Leteo for $machine; build from source with 'cargo install --git https://github.com/$REPO'" ;;
    esac
    printf '%s-%s' "$arch" "$os"
}

resolve_version() {
    if [ "$VERSION" != "latest" ]; then
        printf '%s' "$VERSION"
        return
    fi
    # The redirect from /releases/latest names the tag, which avoids depending
    # on the API and its rate limit.
    resolved=$(curl -fsSLI -o /dev/null -w '%{url_effective}' \
        "https://github.com/$REPO/releases/latest" 2>/dev/null | sed 's|.*/tag/||')
    [ -n "$resolved" ] || fail "could not work out the latest version; set LETEO_VERSION"
    printf '%s' "$resolved"
}

need curl
need tar
need uname

TARGET="$(target_triple)"
VERSION="$(resolve_version)"
PACKAGE="leteo-$VERSION-$TARGET"
ARCHIVE="$PACKAGE.tar.gz"
# Overridable so an internal mirror can serve the same layout, and so the
# verification path can be exercised without publishing anything.
BASE="${LETEO_BASE_URL:-https://github.com/$REPO/releases/download/$VERSION}"

say "Leteo $VERSION for $TARGET"

TEMP="$(mktemp -d)"
# Leave nothing behind, including on failure.
trap 'rm -rf "$TEMP"' EXIT INT TERM

say "  downloading"
curl -fsSL "$BASE/$ARCHIVE" -o "$TEMP/$ARCHIVE" \
    || fail "no release archive at $BASE/$ARCHIVE"
curl -fsSL "$BASE/SHA256SUMS" -o "$TEMP/SHA256SUMS" \
    || fail "no checksums published for $VERSION; refusing to install unverified"

say "  verifying"
expected=$(grep " $ARCHIVE\$" "$TEMP/SHA256SUMS" | awk '{print $1}')
[ -n "$expected" ] || fail "$ARCHIVE is not listed in SHA256SUMS"
if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$TEMP/$ARCHIVE" | awk '{print $1}')
elif command -v shasum >/dev/null 2>&1; then
    actual=$(shasum -a 256 "$TEMP/$ARCHIVE" | awk '{print $1}')
else
    fail "this needs sha256sum or shasum to verify the download"
fi
[ "$actual" = "$expected" ] || fail "checksum mismatch: expected $expected, got $actual"

say "  installing to $INSTALL_DIR"
tar -xzf "$TEMP/$ARCHIVE" -C "$TEMP"
mkdir -p "$INSTALL_DIR"
install -m 755 "$TEMP/$PACKAGE/leteo" "$INSTALL_DIR/leteo" 2>/dev/null \
    || { cp "$TEMP/$PACKAGE/leteo" "$INSTALL_DIR/leteo" && chmod 755 "$INSTALL_DIR/leteo"; }

# Beside the binary rather than fetched when it is wanted: removing a tool
# should not require being online. Unlike Windows it is only a convenience —
# `leteo uninstall` does the whole job here, including deleting its own binary,
# which Unix allows and Windows does not.
if [ -f "$TEMP/$PACKAGE/uninstall.sh" ]; then
    cp "$TEMP/$PACKAGE/uninstall.sh" "$INSTALL_DIR/uninstall.sh" 2>/dev/null && chmod 755 "$INSTALL_DIR/uninstall.sh"
fi

say ""
case ":$PATH:" in
    *":$INSTALL_DIR:"*)
        say "Installed. Run 'leteo setup' to configure your agents."
        say "To remove it: 'leteo uninstall'."
        ;;
    *)
        say "Installed, but $INSTALL_DIR is not on your PATH."
        say "Add this to your shell profile:"
        say ""
        say "    export PATH=\"\$PATH:$INSTALL_DIR\""
        say ""
        say "Then run 'leteo setup' to configure your agents."
        ;;
esac
