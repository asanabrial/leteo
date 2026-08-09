#!/bin/sh
# Removes Leteo from Linux and macOS, completely.
#
#   sh uninstall.sh
#
# There is no registry here and nothing for a system settings panel to list, so
# unlike Windows this script is a convenience rather than a requirement:
# `leteo uninstall` does all of it, including deleting its own binary, because
# unlinking a running executable is allowed on Unix. This exists for the case
# where the binary is already gone or will not run.

set -eu

INSTALL_DIR="${LETEO_INSTALL_DIR:-$HOME/.local/bin}"
DATA_DIR="${LETEO_DATA_DIR:-$HOME/.leteo}"
BINARY="$INSTALL_DIR/leteo"

YES=0
DRY_RUN=0
for arg in "$@"; do
    case "$arg" in
        -y|--yes) YES=1 ;;
        -n|--dry-run) DRY_RUN=1 ;;
        *) echo "unknown option: $arg" >&2; exit 2 ;;
    esac
done

say() { printf '%s\n' "$1"; }

# Counted before anything goes, so the number on screen is the number that is
# about to be destroyed rather than an estimate.
memories="unknown"
if [ -x "$BINARY" ]; then
    counted=$("$BINARY" stats --json 2>/dev/null | sed -n 's/.*"total_observations"[ ]*:[ ]*\([0-9]*\).*/\1/p' | head -n 1)
    [ -n "$counted" ] && memories="$counted"
fi

say "Leteo will be removed from this machine:"
say ""
say "  every agent it was configured in  (MCP server, hooks, memory protocol)"
say "  $DATA_DIR"
say "      $memories memories, settings, and any backups kept beside them"
say "  $BINARY"
say ""
# The boundary worth stating: a `.leteo/` inside a repository is project data,
# usually committed and often somebody else's too. Searching the filesystem for
# those and deleting them would take files out of version control.
say "  Not touched: any .leteo/ folder inside a repository. Those are project"
say "  files, usually committed to git and shared with the rest of a team."
say ""

if [ "$DRY_RUN" -eq 1 ]; then
    say "Nothing was removed (--dry-run)."
    exit 0
fi

if [ "$YES" -eq 0 ]; then
    printf 'Remove all of it? This cannot be undone [y/N] '
    read -r answer
    case "$answer" in
        y|Y) ;;
        *) say "Nothing was removed."; exit 0 ;;
    esac
fi

# The agents and the data first, while the binary that knows where they are is
# still here. It resolves twelve agents' config files and strips the MCP server,
# the hooks and the memory-protocol block from each; doing that here by hand
# would be a second, worse copy of the same knowledge.
if [ -x "$BINARY" ]; then
    say "  removing agent configuration and memories"
    "$BINARY" uninstall --yes || say "  leteo uninstall failed; carrying on with the files"
else
    say "  no binary to ask, removing the data directory directly"
fi

# Only when the binary could not do it. `leteo uninstall` removes its own files
# and leaves anything it did not create; this is the fallback for a store whose
# binary is already gone, so it names the same files rather than reaching for
# `rm -rf` on a path `LETEO_DATA_DIR` may point anywhere.
if [ -d "$DATA_DIR" ] && [ ! -x "$BINARY" ]; then
    say "  removing Leteo's files from $DATA_DIR"
    rm -f "$DATA_DIR"/leteo.db* "$DATA_DIR"/store.db* \
          "$DATA_DIR/settings.json" "$DATA_DIR/cloud.json"
    rm -rf "$DATA_DIR/hooks" "$DATA_DIR"/backup-*
    # Only if that emptied it. A note somebody filed beside the store keeps the
    # directory, and is reported rather than taken along with it.
    if [ -z "$(ls -A "$DATA_DIR" 2>/dev/null)" ]; then
        rmdir "$DATA_DIR"
    else
        say "  $DATA_DIR was kept: it holds files Leteo did not put there"
    fi
fi

# The two files the installer wrote, by name. Never the directory: the default
# is `~/.local/bin`, which is shared with every other tool somebody installed.
if [ -e "$BINARY" ]; then
    say "  removing $BINARY"
    rm -f "$BINARY"
fi
rm -f "$INSTALL_DIR/uninstall.sh"

say ""
say "Leteo is gone."
# The installer never edits a shell profile — it prints the line and leaves the
# choice — so there is nothing here to undo. Said out loud because a PATH entry
# somebody added by hand is the one trace that can outlive this, and silence
# about it would look like the uninstall having missed something.
case ":$PATH:" in
    *":$INSTALL_DIR:"*)
        say ""
        say "If you added $INSTALL_DIR to your shell profile by hand, that line"
        say "is still there. Nothing else put it on your PATH."
        ;;
esac
