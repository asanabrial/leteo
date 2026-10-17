#!/bin/sh
# Does render.sh fill the winget manifests from what a release says, and say
# which thing went wrong when it cannot?
#
#   sh tools/winget/tests/check.sh
#
# render.sh is run behind a stand-in `curl` that answers from files here, so no
# network is touched and every answer GitHub can give is one this can choose: a
# release, a release without SHA256SUMS, one whose SHA256SUMS names another
# archive, a request nobody answered, and a tag with no published release. The
# real thing only meets the first of those, on a release day, which is too late
# to learn that a message names the wrong cause.
#
# Every run is under `env -i`, as in `tools/build-install/check.sh`, so the
# stand-in is the only `curl` render.sh can find.

set -eu

cd "$(dirname "$0")/../../.."
REPO="$(pwd -P)"

# `mktemp` on its own line, so that its failure stops the check rather than
# leaving `$T` empty (issue #191).
T="$(mktemp -d)" || { echo "winget check could not run: mktemp -d failed" >&2; exit 2; }
T="$(cd "$T" && pwd -P)"
case "$T" in
    /*) [ -d "$T" ] && [ "$T" != "$REPO" ] ;;
    *) false ;;
esac || { echo "winget check could not run: no temporary directory of its own" >&2; exit 2; }
trap 'rm -rf "$T"' EXIT INT TERM

mkdir -p "$T/bin"
cat > "$T/bin/curl" <<'EOF'
#!/bin/sh
# Answers render.sh's `curl -sSL -o <file> -w '%{http_code}' <url>` from
# $FAKE/<kind>.status and $FAKE/<kind>.body, where <kind> is `sums` or `release`.
out=""
while [ $# -gt 1 ]; do
    case "$1" in
        -o) out="$2"; shift 2 ;;
        -w) shift 2 ;;
        *) shift ;;
    esac
done
case "$1" in
    */SHA256SUMS) kind=sums ;;
    */releases/tags/*) kind=release ;;
    *) echo "stand-in curl was asked for $1" >&2; exit 99 ;;
esac
status="$(cat "$FAKE/$kind.status")"
if [ "$status" = 000 ]; then
    printf 000
    exit 7
fi
cp "$FAKE/$kind.body" "$out"
printf '%s' "$status"
EOF
chmod +x "$T/bin/curl"

HASH=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
UPPER=0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF
failures=0

# Sets the stand-in's answers: the two statuses, and the two bodies.
answer() {
    mkdir -p "$T/fake"
    printf '%s' "$1" > "$T/fake/sums.status"
    printf '%s\n' "$2" > "$T/fake/sums.body"
    printf '%s' "$3" > "$T/fake/release.status"
    printf '%s\n' "$4" > "$T/fake/release.body"
}

render() {
    rm -rf "$T/out"
    env -i PATH="$T/bin:/usr/bin:/bin" FAKE="$T/fake" \
        sh "$REPO/tools/winget/render.sh" v1.2.3 "$T/out" > "$T/stdout" 2> "$T/stderr"
}

fail() {
    echo "FAIL: $1" >&2
    sed 's/^/  stderr: /' "$T/stderr" >&2
    failures=$((failures + 1))
}

# Expects render.sh to fail and say $2.
refuses() {
    if render; then
        fail "$1: rendered, expected a refusal"
    elif ! grep -q "$2" "$T/stderr"; then
        fail "$1: refused without saying '$2'"
    fi
}

released='{"tag_name": "v1.2.3", "published_at": "2026-10-17T01:02:03Z"}'

answer 200 "$HASH *leteo-v1.2.3-x86_64-pc-windows-msvc.zip" 200 "$released"
if render; then
    installer="$T/out/asanabrial.leteo.installer.yaml"
    grep -q "InstallerSha256: $UPPER" "$installer" || fail "binary-mode sums: hash not filled in upper case"
    grep -q 'RelativeFilePath: leteo-v1.2.3-x86_64-pc-windows-msvc\\leteo.exe' "$installer" ||
        fail "binary-mode sums: nested path not versioned"
    grep -q 'ReleaseDate: 2026-10-17' "$installer" || fail "binary-mode sums: date not filled"
    grep -q 'PackageVersion: 1.2.3' "$T/out/asanabrial.leteo.yaml" || fail "binary-mode sums: version not filled"
    grep -q 'releases/tag/v1.2.3' "$T/out/asanabrial.leteo.locale.en-US.yaml" ||
        fail "binary-mode sums: release notes link not filled"
    [ "$(ls "$T/out" | wc -l)" -eq 3 ] || fail "binary-mode sums: not three manifests"
else
    fail "binary-mode sums: refused a release it should render"
fi

answer 200 "$HASH  leteo-v1.2.3-x86_64-pc-windows-msvc.zip" 200 "$released"
if render; then
    grep -q "InstallerSha256: $UPPER" "$T/out/asanabrial.leteo.installer.yaml" ||
        fail "text-mode sums: hash not filled"
else
    fail "text-mode sums: refused a release it should render"
fi

answer 404 "" 200 "$released"
refuses "no SHA256SUMS" "has no SHA256SUMS"

answer 000 "" 200 "$released"
refuses "SHA256SUMS unanswered" "could not be read"

answer 503 "" 200 "$released"
refuses "SHA256SUMS server error" "could not be read"

answer 200 "$HASH *leteo-v1.2.3-x86_64-apple-darwin.tar.gz" 200 "$released"
refuses "no Windows archive in SHA256SUMS" "names no leteo-v1.2.3-x86_64-pc-windows-msvc.zip"

answer 200 "$HASH *leteo-v1.2.3-x86_64-pc-windows-msvc.zip" 404 ""
refuses "unpublished release" "no published release"

answer 200 "$HASH *leteo-v1.2.3-x86_64-pc-windows-msvc.zip" 000 ""
refuses "release unanswered" "could not be read from GitHub"

answer 200 "$HASH *leteo-v1.2.3-x86_64-pc-windows-msvc.zip" 200 '{"tag_name": "v1.2.3"}'
refuses "release without a date" "carries no publication date"

if [ "$failures" -ne 0 ]; then
    echo "winget check: $failures failed" >&2
    exit 1
fi
echo "winget check: render.sh fills the manifests and names each refusal"
