#!/bin/sh
# Does install-publisher.sh check the bytes it was told to, and does it say
# which thing went wrong when it cannot?
#
#   sh tools/registry/tests/check.sh
#
# The install body used to run only on a release dispatch, so a wrong digest in
# the workflow's `env` map, or a `case` line mapping a platform to another
# platform's digest, would first be discovered on a release day. Here it runs
# behind a stand-in `curl` and `sha256sum`, with no network and no release, on a
# platform this runner is not.
#
# Every run is under `env -i`, as in `tools/winget/tests/check.sh`, so the
# stand-ins are the only `curl` and `sha256sum` the script can find. `tar` is
# the real one: the archive a stand-in hands over has to extract to a runnable
# `mcp-publisher`, and that is the half this check does not fake.

set -eu

cd "$(dirname "$0")/../../.."
REPO="$(pwd -P)"

# `mktemp` on its own line, so that its failure stops the check rather than
# leaving `$T` empty (issue #191).
T="$(mktemp -d)" || { echo "registry check could not run: mktemp -d failed" >&2; exit 2; }
T="$(cd "$T" && pwd -P)"
case "$T" in
    /*) [ -d "$T" ] && [ "$T" != "$REPO" ] ;;
    *) false ;;
esac || { echo "registry check could not run: no temporary directory of its own" >&2; exit 2; }
trap 'rm -rf "$T"' EXIT INT TERM

VERSION=1.8.16
# Four different sums, so a `case` line that reads another platform's variable
# is caught here rather than on a platform no runner uses. The stand-in
# `sha256sum` only compares strings; the digest of the real archive is the
# release's business, not this check's.
D_LINUX_AMD64=1111111111111111111111111111111111111111111111111111111111111111
D_LINUX_ARM64=2222222222222222222222222222222222222222222222222222222222222222
D_DARWIN_AMD64=3333333333333333333333333333333333333333333333333333333333333333
D_DARWIN_ARM64=4444444444444444444444444444444444444444444444444444444444444444
OTHER=9999999999999999999999999999999999999999999999999999999999999999

mkdir -p "$T/bin" "$T/fixtures"

# A stand-in curl. `-fsSLO <url>` is the shape install-publisher.sh uses: the
# archive keeps the remote name, and the bytes come from $FAKE_ARCHIVE.
cat > "$T/bin/curl" <<'EOF'
#!/bin/sh
for last; do :; done
name="${last##*/}"
if [ -z "$name" ] || [ "$name" = "$last" ]; then
    echo "stand-in curl was asked for $last" >&2
    exit 99
fi
: > "$FAKE_RAN"
cp "$FAKE_ARCHIVE" "$name"
EOF
chmod +x "$T/bin/curl"

# A stand-in sha256sum. It answers `sha256sum -c -` the way the real one does
# for a single entry, comparing the recorded digest with the one the test set as
# the archive's: "OK" and exit 0, or "FAILED" and exit 1.
cat > "$T/bin/sha256sum" <<'EOF'
#!/bin/sh
if [ "$1" != "-c" ]; then
    echo "stand-in sha256sum was asked to $*" >&2
    exit 99
fi
read -r digest name rest
if [ "${digest}" = "${FAKE_EXPECT}" ]; then
    echo "${name}: OK"
    exit 0
fi
echo "${name}: FAILED"
exit 1
EOF
chmod +x "$T/bin/sha256sum"

# Builds an archive whose extracted `mcp-publisher` prints $2 when asked for its
# version, and records its path in $1.
fixture() {
    rm -rf "$T/pkg"
    mkdir -p "$T/pkg"
    cat > "$T/pkg/mcp-publisher" <<EOF
#!/bin/sh
printf '%s\n' 'mcp-publisher version $2'
EOF
    chmod +x "$T/pkg/mcp-publisher"
    tar -czf "$1" -C "$T/pkg" mcp-publisher
}

fixture "$T/fixtures/good.tar.gz" "$VERSION"
fixture "$T/fixtures/other.tar.gz" 1.8.15

failures=0

fail() {
    echo "FAIL: $1" >&2
    sed 's/^/  stderr: /' "$T/stderr" >&2
    failures=$((failures + 1))
}

# Installs with the globals below, in a directory of its own, so "extracted
# nothing" is a fact about that directory rather than a claim.
install() {
    rm -rf "$T/work"
    mkdir -p "$T/work"
    rm -f "$T/curl-ran"
    ( cd "$T/work" &&
        env -i PATH="$T/bin:/usr/bin:/bin" \
            MCP_PUBLISHER="$VERSION" \
            SHA256_LINUX_AMD64="$D_LINUX_AMD64" \
            SHA256_LINUX_ARM64="$D_LINUX_ARM64" \
            SHA256_DARWIN_AMD64="$D_DARWIN_AMD64" \
            SHA256_DARWIN_ARM64="$D_DARWIN_ARM64" \
            FAKE_ARCHIVE="$ARCHIVE" FAKE_EXPECT="$EXPECT" FAKE_RAN="$T/curl-ran" \
            sh "$REPO/tools/registry/install-publisher.sh" --platform "$PLATFORM" \
            > "$T/stdout" 2> "$T/stderr" )
}

# Expects install to succeed.
accepts() {
    if ! install; then
        fail "$1: refused an install it should have made"
    fi
}

# Expects install to fail and name $2.
refuses() {
    if install; then
        fail "$1: installed, expected a refusal"
    elif ! grep -qF -- "$2" "$T/stderr"; then
        fail "$1: refused without naming '$2'"
    fi
}

# Expects install to fail and name $2 on standard output instead: `sha256sum -c`
# reports a mismatch there, the way the real one does.
refuses_on_stdout() {
    if install; then
        fail "$1: installed, expected a refusal"
    elif ! grep -qF -- "$2" "$T/stdout"; then
        fail "$1: refused without naming '$2' on standard output"
    fi
}

ARCHIVE="$T/fixtures/good.tar.gz"

# Every platform named in the `case`, each against its own digest: a line that
# read the wrong variable would still pass a check that only knew one sum.
for entry in \
    "linux/amd64 $D_LINUX_AMD64" \
    "linux/arm64 $D_LINUX_ARM64" \
    "darwin/amd64 $D_DARWIN_AMD64" \
    "darwin/arm64 $D_DARWIN_ARM64"
do
    PLATFORM="${entry%% *}"
    EXPECT="${entry#* }"
    accepts "a recorded digest on $PLATFORM"
    archive="mcp-publisher_$(printf '%s' "$PLATFORM" | tr '/' '_').tar.gz"
    grep -q "$archive: OK" "$T/stdout" ||
        fail "a recorded digest on $PLATFORM: the digest check did not report OK"
    grep -q "mcp-publisher version $VERSION" "$T/stdout" ||
        fail "a recorded digest on $PLATFORM: the version was not printed"
    [ -x "$T/work/mcp-publisher" ] || fail "a recorded digest on $PLATFORM: mcp-publisher was not extracted"
done

PLATFORM=linux/amd64
EXPECT="$OTHER"
refuses_on_stdout "a replaced archive" "mcp-publisher_linux_amd64.tar.gz: FAILED"
[ ! -e "$T/work/mcp-publisher" ] || fail "a replaced archive: extracted a binary it refused"

# A platform no release target covers, and a digest that was never recorded for
# one that is: the second is the case `set -u` alone would report under a
# wording that names no platform.
PLATFORM=linux/s390x
EXPECT="$D_LINUX_AMD64"
refuses "an unknown platform" "No sha256 recorded for mcp-publisher $VERSION on linux/s390x."
[ ! -e "$T/curl-ran" ] || fail "an unknown platform: downloaded before refusing"

PLATFORM=darwin/arm64
D_DARWIN_ARM64=""
refuses "an unrecorded digest" "SHA256_DARWIN_ARM64 is unset"
[ ! -e "$T/curl-ran" ] || fail "an unrecorded digest: downloaded before refusing"
D_DARWIN_ARM64=4444444444444444444444444444444444444444444444444444444444444444
PLATFORM=linux/amd64

# A release that served another binary under the recorded tag: the digest
# matched, so the version assertion is the only thing standing between it and
# `id-token: write`.
ARCHIVE="$T/fixtures/other.tar.gz"
refuses "another binary under the tag" "which names no version $VERSION"
ARCHIVE="$T/fixtures/good.tar.gz"

rm -rf "$T/work"
mkdir -p "$T/work"
if ( cd "$T/work" &&
        env -i PATH="$T/bin:/usr/bin:/bin" MCP_PUBLISHER="$VERSION" \
            sh "$REPO/tools/registry/install-publisher.sh" > "$T/stdout" 2> "$T/stderr" ); then
    fail "no platform: installed without one"
elif ! grep -qF -- "usage: install-publisher.sh --platform <os/arch>" "$T/stderr"; then
    fail "no platform: refused without saying what it needs"
fi

if ( cd "$T/work" &&
        env -i PATH="$T/bin:/usr/bin:/bin" MCP_PUBLISHER="$VERSION" \
            sh "$REPO/tools/registry/install-publisher.sh" --platform > "$T/stdout" 2> "$T/stderr" ); then
    fail "an empty platform: installed without one"
elif ! grep -qF -- "--platform needs an os/arch argument" "$T/stderr"; then
    fail "an empty platform: refused without saying what it needs"
fi

if [ "$failures" -ne 0 ]; then
    echo "registry check: $failures failed" >&2
    exit 1
fi
echo "registry check: install-publisher.sh checks the digest and the version, and names each refusal"
