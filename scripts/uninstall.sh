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
# The model's files by name, once. tests/model_names.rs checks this line against
# MODEL_FILES in src/semantic/mod.rs, which a shell script cannot read.
MODEL_FILES="config.json model.safetensors tokenizer.json.gz"
# Everything `leteo uninstall` owns in the data directory, by name.
# tests/uninstall_files.rs checks this line against DATA_DIR_FILES in
# src/setup/removal/mod.rs. A `leteo.db*` glob would take a
# `leteo.db.bak-before-migrate` or a `leteo.db.pre-schema-N` with it, and those
# copies are the only way back from a one-way migration.
DATA_DIR_FILES="leteo.db leteo.db-wal leteo.db-shm leteo.db-journal settings.json cloud.json store.db store.db-wal store.db-shm store.db-journal"

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

# Said of a directory that survived. When the binary ran it already reported why
# it kept what it kept -- a hash that is not its pin, a symlink, an I/O error --
# so naming a stranger here would often be wrong. Without it, the only reason
# this script leaves a directory is that it was not empty.
say_kept() {
    if [ "$ran" -eq 1 ]; then
        say "  $1 was kept; the report from leteo uninstall above says why"
    elif [ -n "$(ls -A "$1" 2>/dev/null)" ]; then
        say "  $1 was kept: it holds files Leteo did not put there"
    else
        say "  $1 was kept: it could not be removed"
    fi
}

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
say "  the semantic search model, if one was installed"
say "      ($INSTALL_DIR/../share/leteo/model and $DATA_DIR/model)"
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
# still here. It resolves fifteen agents' config files and strips the MCP server,
# the hooks and the memory-protocol block from each; doing that here by hand
# would be a second, worse copy of the same knowledge.
# `ran` is whether the binary started and so judged the model files, whatever it
# exited with: it exits non-zero on any incomplete removal, an agent-config error
# for one, after it has already kept the model files it did not recognise. It
# takes a model file only when the file hashes to the pin it was built with, and
# the by-name removal below does not look at content, so it must not run behind a
# binary that has decided to keep a file. Started is read from what the binary
# itself printed, and not from the shell's exit code: a binary of the wrong
# architecture is answered with 126 or 127 by bash, but dash re-runs it as a
# script on ENOEXEC and exits 2, which looks like the binary failing, or like it
# succeeding when the file is empty. Two things count, either one: the line
# `leteo uninstall --yes` prints before it judges anything, and a line of its
# JSON report. The second is for a binary built before that line existed, which
# an older archive can leave beside this script; it judged the model and kept
# what it did not recognise, and no marker is no reason to delete it. The key is
# one no shell error, and no script dash re-ran, would print. A binary that
# printed neither judged nothing, and counting it as a run left the model behind
# and cited a report that was never printed. `handled` is the narrower claim
# that it finished, and gates only the data files, which the binary and this
# script name identically, so trying them again after a failure undoes no
# judgment.
STARTED_MARKER="leteo uninstall: started"
ran=0
handled=0
if [ -x "$BINARY" ]; then
    say "  removing agent configuration and memories"
    status=0
    output=$("$BINARY" uninstall --yes 2>&1) || status=$?
    printf '%s\n' "$output"
    if printf '%s\n' "$output" | grep -Eqx "$STARTED_MARKER|  \"model_removed\": (true|false),?"; then
        ran=1
        if [ "$status" -eq 0 ]; then
            handled=1
        else
            say "  leteo uninstall failed; carrying on with the files"
        fi
    else
        say "  could not start leteo uninstall; carrying on with the files"
    fi
else
    say "  no binary to ask, removing the data directory directly"
fi

# Only when the binary could not do it. `leteo uninstall` removes its own files
# and leaves anything it did not create; this is the fallback for a store whose
# binary is already gone, so it names the same files rather than reaching for
# `rm -rf` on a path `LETEO_DATA_DIR` may point anywhere.
if [ -d "$DATA_DIR" ] && [ "$handled" -eq 0 ]; then
    say "  removing Leteo's files from $DATA_DIR"
    for own in $DATA_DIR_FILES; do
        rm -f "$DATA_DIR/$own"
    done
    rm -rf "$DATA_DIR/hooks" "$DATA_DIR"/backup-*
    # `leteo model install` writes here by default.
    if [ "$ran" -eq 0 ]; then
        for file in $MODEL_FILES; do rm -f "$DATA_DIR/model/$file"; done
    fi
    rmdir "$DATA_DIR/model" 2>/dev/null || true
    # Only if that emptied it. A note somebody filed beside the store keeps the
    # directory, and is reported rather than taken along with it.
    if [ -z "$(ls -A "$DATA_DIR" 2>/dev/null)" ]; then
        rmdir "$DATA_DIR"
    else
        say_kept "$DATA_DIR"
    fi
fi

# The two files the installer wrote, by name. Never the directory: the default
# is `~/.local/bin`, which is shared with every other tool somebody installed.
if [ -e "$BINARY" ]; then
    say "  removing $BINARY"
    rm -f "$BINARY"
fi
rm -f "$INSTALL_DIR/uninstall.sh"
# The model the installer put under `../share/leteo/model`, by name, and the
# directories only if that emptied them: never `share/` itself. The files are
# left alone when the binary ran, for the reason given above; an empty directory
# is no one's judgment and still goes.
SHARE_DIR="$INSTALL_DIR/../share/leteo"
if [ "$ran" -eq 0 ]; then
    for file in $MODEL_FILES; do rm -f "$SHARE_DIR/model/$file"; done
fi
rmdir "$SHARE_DIR/model" 2>/dev/null || true
rmdir "$SHARE_DIR" 2>/dev/null || true
for kept in "$SHARE_DIR/model" "$SHARE_DIR"; do
    [ ! -d "$kept" ] || say_kept "$kept"
done

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
