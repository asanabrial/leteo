#!/bin/sh
# Does an installed Leteo find its model, and does uninstalling take it away?
#
#   sh tools/semantic/check_install.sh [path-to-release-leteo]
#
# The model is a file beside the binary and not inside it, so nothing fails if
# the release stops carrying it, or an installer puts it somewhere the binary
# does not look: `doctor` says the stage is off and search carries on by words.
# This builds an archive laid out the way `release.yml` packs one, runs
# `scripts/install.sh` against it over file://, asks the installed binary's
# `doctor`, and then removes Leteo three ways -- `leteo uninstall --yes`,
# `uninstall.sh` with the binary there, and `uninstall.sh` with the binary gone --
# and asserts that no model file or directory it created is left, and that
# `share/` is. Then it puts both uninstall scripts behind binaries that ran and
# failed, and behind ones that cannot start, and asserts that the model a binary
# judged is never taken by name.
#
# The npm wrapper is next: it is served over a local HTTPS endpoint with a
# certificate made for the run, and it needs `node` and `openssl` and `python3`,
# whose absence is a check that could not run and not a pass. `uninstall.ps1` is
# last, and needs `pwsh`; without it that section is reported as not run, after
# everything before it has. `install.sh` limits the rest to Unix, and
# `install.ps1` is not covered here.
#
# Nothing touches the real home or store, and the way that is guaranteed is not
# a list of what to unset. Every command runs under `env -i`, so the only things
# the binary and the scripts can see are the ones `isolated` names below: PATH,
# a HOME and a data directory inside one temporary directory, and what a call
# adds itself. A variable the binary learns to read tomorrow -- or already reads
# today, like `LETEO_DATABASE`, which outranks the data directory -- cannot
# reach it, because it was never passed. The directory is removed on the way out.

set -eu

cd "$(dirname "$0")/../.."
BINARY="${1:-target/release/leteo}"
[ -f "$BINARY" ] || { echo "install check could not run: $BINARY is not a file" >&2; exit 2; }
BINARY="$(cd "$(dirname "$BINARY")" && pwd)/$(basename "$BINARY")"
REPO="$(pwd)"

ROOT="$(mktemp -d)"
mkdir -p "$ROOT/tmp" "$ROOT/home"
cleanup() {
    # The server says its own pid: `$!` of a backgrounded shell function is the
    # subshell that runs it, and killing that leaves the server running.
    [ ! -s "$ROOT/server.pid" ] || kill "$(cat "$ROOT/server.pid")" 2>/dev/null || true
    rm -rf "$ROOT"
}
trap cleanup EXIT INT TERM

VERSION="v0.0.0-install-check"
# The names install.sh derives from the machine; it has no way to be asked.
case "$(uname -s)" in
    Linux)  os="unknown-linux-gnu" ;;
    Darwin) os="apple-darwin" ;;
    *) echo "install check could not run: no archive layout for $(uname -s)" >&2; exit 2 ;;
esac
case "$(uname -m)" in
    x86_64|amd64)  arch="x86_64" ;;
    arm64|aarch64) arch="aarch64" ;;
    *) echo "install check could not run: no archive layout for $(uname -m)" >&2; exit 2 ;;
esac
PACKAGE="leteo-$VERSION-$arch-$os"

# The staging steps of release.yml, for the files this concerns.
mkdir -p "$ROOT/dist/$PACKAGE"
cp "$BINARY" "$ROOT/dist/$PACKAGE/leteo"
cp scripts/uninstall.sh "$ROOT/dist/$PACKAGE/"
cp -R assets/model "$ROOT/dist/$PACKAGE/model"
cp -R LICENSES "$ROOT/dist/$PACKAGE/LICENSES"
tar -C "$ROOT/dist" -czf "$ROOT/dist/$PACKAGE.tar.gz" "$PACKAGE"
rm -rf "$ROOT/dist/$PACKAGE"
if command -v sha256sum >/dev/null 2>&1; then
    (cd "$ROOT/dist" && sha256sum "$PACKAGE.tar.gz" > SHA256SUMS)
else
    (cd "$ROOT/dist" && shasum -a 256 "$PACKAGE.tar.gz" > SHA256SUMS)
fi

failed=0
ps1_skipped=0
check() {
    # check <description> <command...>
    description="$1"; shift
    if "$@"; then
        printf 'ok    %s\n' "$description"
    else
        printf 'FAIL  %s\n' "$description"
        failed=1
    fi
}

# The whole environment of every command here. Structural on purpose: see the
# header. TMPDIR keeps `mktemp` inside the temporary directory too.
isolated() {
    env -i PATH="$PATH" HOME="$ROOT/home" LETEO_DATA_DIR="$ROOT/data" TMPDIR="$ROOT/tmp" "$@"
}

install_into() {
    prefix="$1"
    isolated LETEO_INSTALL_DIR="$prefix/bin" LETEO_VERSION="$VERSION" \
        LETEO_BASE_URL="file://$ROOT/dist" sh scripts/install.sh >"$ROOT/install.log" 2>&1 \
        || { cat "$ROOT/install.log"; echo "install.sh failed" >&2; exit 1; }
}

model_names() {
    # The names the binary checks, read from the one list in the source.
    sed -n '/pub const MODEL_FILES/,/^];/p' src/semantic/mod.rs | sed -n 's/^ *"\([a-z0-9._]*\)",$/\1/p' \
        | grep '\.'
}

doctor_detail() {
    # doctor_detail <leteo...>: what doctor says about the model, on one line.
    # Blanks are stripped to read the JSON on one line, which is also why the
    # sentences compared against have none.
    isolated "$@" doctor 2>/dev/null | tr -d ' \n' \
        | grep -o '"code":"semantic_model","ok":true,"detail":"[^"]*' || true
}

verified_at() {
    # The detail doctor gives names the directory it found the model in, which has
    # to be the one install.sh wrote: `../share/leteo/model` from the binary.
    doctor_detail "$1/bin/leteo" | grep -q 'verifiedat[^"]*/bin/\.\./share/leteo/model$'
}

none_left() {
    # none_left <prefix>
    for name in $(model_names); do
        [ ! -e "$1/share/leteo/model/$name" ] || return 1
    done
    [ ! -e "$1/share/leteo" ] && [ -d "$1/share" ]
}

[ "$(model_names | wc -l)" -ge 3 ] || { echo "install check could not run: no model names found in src/semantic/mod.rs" >&2; exit 2; }

# Something of somebody else's in share/, which no removal may take.
seed_share() { mkdir -p "$1/share" && : > "$1/share/someone-elses"; }

echo "-- installed, then removed by leteo uninstall"
P="$ROOT/a"
seed_share "$P"
install_into "$P"
for name in $(model_names); do
    check "install.sh put $name under share/leteo/model" test -f "$P/share/leteo/model/$name"
done
check "the installed binary's doctor verifies the model there" verified_at "$P"
isolated "$P/bin/leteo" uninstall --yes >"$ROOT/uninstall.log" 2>&1 || { cat "$ROOT/uninstall.log"; failed=1; }
check "leteo uninstall removed the model and share/leteo, and not share/" none_left "$P"
check "and left what was in share/" test -f "$P/share/someone-elses"

echo "-- installed, then removed by uninstall.sh with the binary there"
P="$ROOT/b"
seed_share "$P"
install_into "$P"
check "doctor verifies the model" verified_at "$P"
isolated LETEO_INSTALL_DIR="$P/bin" sh "$P/bin/uninstall.sh" --yes >"$ROOT/uninstall.log" 2>&1 || { cat "$ROOT/uninstall.log"; failed=1; }
check "uninstall.sh removed the model and share/leteo, and not share/" none_left "$P"
check "and the binary" test ! -e "$P/bin/leteo"

echo "-- installed, then removed by uninstall.sh once the binary is gone"
P="$ROOT/c"
seed_share "$P"
install_into "$P"
rm -f "$P/bin/leteo"
isolated LETEO_INSTALL_DIR="$P/bin" sh "$P/bin/uninstall.sh" --yes >"$ROOT/uninstall.log" 2>&1 || { cat "$ROOT/uninstall.log"; failed=1; }
check "uninstall.sh alone removed the model and share/leteo, and not share/" none_left "$P"

# The `ran` gate of both uninstall scripts. The by-name removal of the model does
# not look at content, so it may run only behind a binary that never judged the
# model; behind one that did, it would take a file the binary kept because its
# hash is not the pin. Nothing else exercises that: the by-name path is only
# correct when the binary is gone, and the other flows here never leave a file
# the binary would keep.
PLANTED="somebody else's config, not the pinned one"
# An agent whose config the binary cannot parse makes `complete()` false, so it
# exits non-zero after judging the model. A malformed file and not a permission
# bit, because a runner that is root ignores the permission.
fail_an_agent() { printf '{not json' > "$ROOT/home/.claude.json"; }
heal_the_agent() { rm -f "$ROOT/home/.claude.json"; }
plant_in() { printf '%s' "$PLANTED" > "$1/config.json"; }
planted_survived() { [ "$(cat "$1/config.json" 2>/dev/null)" = "$PLANTED" ]; }
cites_a_report() { grep -q "the report from leteo uninstall above" "$ROOT/uninstall.log"; }
not() { ! "$@"; }
seed_store() { mkdir -p "$ROOT/data" && : > "$ROOT/data/leteo.db"; }
store_gone() { [ ! -e "$ROOT/data/leteo.db" ]; }
lacks_model_files() {
    for name in $(model_names); do [ ! -e "$1/$name" ] || return 1; done
}

echo "-- uninstall.sh behind a binary that ran, kept a file, and exited non-zero"
P="$ROOT/d"
install_into "$P"
plant_in "$P/share/leteo/model"
fail_an_agent
isolated LETEO_INSTALL_DIR="$P/bin" sh "$P/bin/uninstall.sh" --yes >"$ROOT/uninstall.log" 2>&1 || { cat "$ROOT/uninstall.log"; failed=1; }
heal_the_agent
check "the binary exited non-zero, so the run is the one this flow means" grep -q "leteo uninstall failed" "$ROOT/uninstall.log"
check "the file whose hash is not its pin survived the by-name removal" planted_survived "$P/share/leteo/model"
check "and the message cites the report the binary printed" cites_a_report

echo "-- uninstall.sh behind a binary that cannot start"
P="$ROOT/e"
install_into "$P"
# Executable, and exec fails: the shell answers 126 or 127 without running a line.
printf '#!/nonexistent-interpreter\n' > "$P/bin/leteo"
chmod +x "$P/bin/leteo"
seed_store
isolated LETEO_INSTALL_DIR="$P/bin" sh "$P/bin/uninstall.sh" --yes >"$ROOT/uninstall.log" 2>&1 || { cat "$ROOT/uninstall.log"; failed=1; }
check "and so are the data files, which nothing else is left to remove" store_gone
check "the model is removed by name, as for a binary that is gone" none_left "$P"
check "and the message does not cite a report that was never printed" not cites_a_report

# `ran` is decided by what the binary said, and not by the shell's exit code. An
# executable that is not a program for this machine is answered with 126 or 127
# by bash, but dash re-runs it as a script on ENOEXEC: with no `#!` line and a
# body that says `exit 2`, that is a "binary" that exits 2 having judged
# nothing, and with `exit 0` one that looks like a success.
UNINSTALL_SH="sh"
command -v dash >/dev/null 2>&1 && UNINSTALL_SH="dash"
for fixture_exit in 2 0; do
    echo "-- uninstall.sh under $UNINSTALL_SH behind an executable that is no program and exits $fixture_exit"
    P="$ROOT/e$fixture_exit"
    install_into "$P"
    printf 'exit %s\n' "$fixture_exit" > "$P/bin/leteo"
    chmod +x "$P/bin/leteo"
    seed_store
    isolated LETEO_INSTALL_DIR="$P/bin" "$UNINSTALL_SH" "$P/bin/uninstall.sh" --yes >"$ROOT/uninstall.log" 2>&1 || { cat "$ROOT/uninstall.log"; failed=1; }
    check "the data files are removed by name" store_gone
    check "the model is removed by name, as for a binary that is gone" none_left "$P"
    check "and the message does not cite a report that was never printed" not cites_a_report
done

# A stand-in for a binary that started, said so, and then failed without doing
# any of the removal itself, so what the script removes afterwards is the
# script's own doing and not the binary's.
ran_and_failed() {
    printf '#!/bin/sh\n[ "$1" != uninstall ] || echo "leteo uninstall: started" >&2\nexit 1\n' > "$1"
    chmod +x "$1"
}

echo "-- uninstall.sh behind a binary that started and failed before removing anything"
P="$ROOT/h"
install_into "$P"
plant_in "$P/share/leteo/model"
ran_and_failed "$P/bin/leteo"
seed_store
isolated LETEO_INSTALL_DIR="$P/bin" sh "$P/bin/uninstall.sh" --yes >"$ROOT/uninstall.log" 2>&1 || { cat "$ROOT/uninstall.log"; failed=1; }
check "the data files are retried by name" store_gone
check "the file the binary may have kept survived" planted_survived "$P/share/leteo/model"
check "and the message cites the report" cites_a_report

echo "-- the npm wrapper, against a local release"
for tool in node openssl python3; do
    command -v "$tool" >/dev/null 2>&1 || { echo "install check could not run: the npm flow needs $tool" >&2; exit 2; }
done
# The wrapper speaks HTTPS only, so the release is served over HTTPS, with a
# certificate made here and trusted by this run's node alone.
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj "/CN=127.0.0.1" \
    -addext "subjectAltName=IP:127.0.0.1" -keyout "$ROOT/key.pem" -out "$ROOT/cert.pem" >/dev/null 2>&1 \
    || { echo "install check could not run: openssl could not make a certificate" >&2; exit 2; }
cat > "$ROOT/serve.py" <<'PY'
import functools, http.server, os, ssl, sys

directory, cert, key, port_file = sys.argv[1:5]


class Quiet(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *args):
        pass


server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Quiet, directory=directory))
context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
context.load_cert_chain(cert, key)
server.socket = context.wrap_socket(server.socket, server_side=True)
with open(os.path.join(os.path.dirname(port_file), "server.pid"), "w") as handle:
    handle.write(str(os.getpid()))
with open(port_file, "w") as handle:
    handle.write(str(server.server_address[1]))
server.serve_forever()
PY
isolated python3 "$ROOT/serve.py" "$ROOT/dist" "$ROOT/cert.pem" "$ROOT/key.pem" "$ROOT/port" >"$ROOT/serve.log" 2>&1 &
tries=0
while [ ! -s "$ROOT/port" ]; do
    tries=$((tries + 1))
    if [ "$tries" -gt 100 ]; then
        cat "$ROOT/serve.log" >&2
        echo "install check could not run: the local release server did not start" >&2
        exit 2
    fi
    sleep 0.1
done

# The wrapper keeps its cache beside itself, so it is run from a copy inside the
# temporary directory and not from the checkout, whose npm/ it would write into.
mkdir -p "$ROOT/npm"
cp -R npm/bin npm/package.json "$ROOT/npm/"
VENDOR="$ROOT/npm/vendor"
CACHE="$VENDOR/$VERSION-$arch-$os"

# NPM_EXTRA_ENV is one more assignment for a call that wants it, and has no
# blanks in it.
NPM_EXTRA_ENV=""
npm_leteo() {
    # shellcheck disable=SC2086
    isolated NODE_EXTRA_CA_CERTS="$ROOT/cert.pem" LETEO_VERSION="$VERSION" \
        LETEO_BASE_URL="https://127.0.0.1:$(cat "$ROOT/port")" $NPM_EXTRA_ENV \
        node "$ROOT/npm/bin/leteo.js" "$@"
}

npm_cache_whole() {
    [ -x "$CACHE/leteo" ] || return 1
    for name in $(model_names); do
        [ -f "$CACHE/model/$name" ] || return 1
    done
    # A half-made directory the wrapper should have cleaned up.
    [ -z "$(ls "$VENDOR" | grep '^staging-' || true)" ] || return 1
    # Run through the wrapper again, which also proves a whole cache is used as it is.
    npm_leteo doctor 2>/dev/null | tr -d ' \n' \
        | grep -o '"code":"semantic_model","ok":true,"detail":"[^"]*' \
        | grep -q "verifiedat[^\"]*/$VERSION-$arch-$os/model\$"
}

# Holds the directory rename of each wrapper run for long enough that both runs
# have looked at the cache before either has changed it. That interleaving is
# the one the wrapper has to survive and a scheduler produces only now and then:
# with the tolerance removed, unassisted runs of this section still passed.
cat > "$ROOT/slow_rename.js" <<'JS'
const fs = require("node:fs");
const path = require("node:path");
const real = fs.renameSync;
fs.renameSync = function (from, to) {
  if (path.basename(String(from)) === "ready") {
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 800);
  }
  return real.apply(this, arguments);
};
JS

# Two wrapper runs started together, and whether both of them succeeded.
two_at_once() {
    NPM_EXTRA_ENV="NODE_OPTIONS=--require=$ROOT/slow_rename.js"
    npm_leteo doctor >"$ROOT/one.out" 2>"$ROOT/one.err" &
    first=$!
    npm_leteo doctor >"$ROOT/two.out" 2>"$ROOT/two.err" &
    second=$!
    NPM_EXTRA_ENV=""
    status_one=0; status_two=0
    wait "$first" || status_one=$?
    wait "$second" || status_two=$?
    [ "$status_one" -eq 0 ] && [ "$status_two" -eq 0 ] || { cat "$ROOT"/one.err "$ROOT"/two.err >&2; return 1; }
}

npm_leteo doctor >/dev/null 2>"$ROOT/npm.log" || { cat "$ROOT/npm.log"; failed=1; }
check "a cold run puts the binary and the model in one directory, and doctor verifies the model there" npm_cache_whole

rm -f "$CACHE/leteo"
npm_leteo doctor >/dev/null 2>"$ROOT/npm.log" || { cat "$ROOT/npm.log"; failed=1; }
check "a directory whose binary was deleted heals on the next run" npm_cache_whole

# Both succeed, and what is left is one whole install.
rm -rf "$VENDOR"
check "two cold runs at once both succeed" two_at_once
check "and leave one whole install" npm_cache_whole
rm -f "$CACHE/leteo"
check "two runs at once on a directory whose binary was deleted both succeed" two_at_once
check "and leave one whole install" npm_cache_whole

echo "-- uninstall.ps1, behind the same binaries"
if ! command -v pwsh >/dev/null 2>&1; then
    echo "install check could not run the uninstall.ps1 flows: pwsh is not installed" >&2
    ps1_skipped=1
else
# The model beside the executable, which is where uninstall.ps1 looks, and a
# Linux binary under the name Windows gives it: PowerShell runs it as it is.
windows_layout() {
    mkdir -p "$1/bin"
    cp "$BINARY" "$1/bin/leteo.exe"
    cp -R assets/model "$1/bin/model"
}
run_ps1() {
    isolated LETEO_INSTALL_DIR="$1/bin" pwsh -NoProfile -NonInteractive -File scripts/uninstall.ps1 -Yes >"$ROOT/uninstall.log" 2>&1 \
        || { cat "$ROOT/uninstall.log"; failed=1; }
}

P="$ROOT/f"
windows_layout "$P"
plant_in "$P/bin/model"
fail_an_agent
run_ps1 "$P"
heal_the_agent
check "ps1: the binary exited non-zero" grep -q "leteo uninstall exited with" "$ROOT/uninstall.log"
check "ps1: the file whose hash is not its pin survived" planted_survived "$P/bin/model"
check "ps1: and the message cites the report" cites_a_report

P="$ROOT/g"
windows_layout "$P"
printf '#!/nonexistent-interpreter\n' > "$P/bin/leteo.exe"
chmod +x "$P/bin/leteo.exe"
seed_store
run_ps1 "$P"
check "ps1: the data files are removed by name, which nothing else is left to do" store_gone
check "ps1: a binary that cannot start leaves the model removed by name" lacks_model_files "$P/bin/model"
check "ps1: and the message does not cite a report" not cites_a_report

# The data directory's own model, which `-not $ran` guards separately from the
# one beside the executable, and the retry of the data files after a binary that
# ran and failed. The stand-in removes nothing, so both are the script's doing.
P="$ROOT/i"
windows_layout "$P"
ran_and_failed "$P/bin/leteo.exe"
mkdir -p "$ROOT/data/model"
plant_in "$ROOT/data/model"
seed_store
run_ps1 "$P"
check "ps1: the data files are retried by name after a binary that ran and failed" store_gone
check "ps1: the file in the data directory's model survived" planted_survived "$ROOT/data/model"
check "ps1: and the message cites the report" cites_a_report
fi

if [ "$failed" -ne 0 ]; then
    echo "install check FAILED (repository: $REPO)"
    exit 1
fi
if [ "$ps1_skipped" -ne 0 ]; then
    echo "install check passed, without the uninstall.ps1 flows"
    exit 2
fi
echo "install check passed"
