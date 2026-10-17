#!/bin/sh
# Writes the winget manifests for one released tag into a directory, in the
# layout `komac submit` takes.
#
# The manifests are templates rather than Komac's own `update` output because
# that copies the previous version's locale forward unread: run against 0.4.1 it
# kept a documentation link that had answered 404 since the directory it named
# was removed, and its `--output` writes nothing under `--dry-run`, so there was
# no point at which the result could be corrected before it was submitted.
#
# The SHA-256 is read from the release's own SHA256SUMS, not computed from a
# download, so the manifest names the bytes the release says it shipped.
set -eu

tag="${1:?usage: render.sh <tag> <output directory>}"
out="${2:?usage: render.sh <tag> <output directory>}"
version="${tag#v}"
here="$(cd "$(dirname "$0")/../.." && pwd)"
templates="$here/tools/winget/manifests"
archive="leteo-$tag-x86_64-pc-windows-msvc.zip"

# Fetches a URL into a file and answers its HTTP status, so a release that lacks
# the file (404) is told apart from a request that went unanswered.
fetch() {
    curl -sSL -o "$2" -w '%{http_code}' "$1" || true
}

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

status="$(fetch "https://github.com/asanabrial/leteo/releases/download/$tag/SHA256SUMS" "$scratch/sums")"
case "$status" in
    200) ;;
    404) echo "the release $tag has no SHA256SUMS" >&2; exit 1 ;;
    *) echo "SHA256SUMS of $tag could not be read (HTTP $status); try again" >&2; exit 1 ;;
esac
sha="$(awk -v name="$archive" '$2 == name || $2 == "*" name { print toupper($1) }' "$scratch/sums")"
if [ -z "$sha" ]; then
    echo "SHA256SUMS of $tag names no $archive" >&2
    exit 1
fi

status="$(fetch "https://api.github.com/repos/asanabrial/leteo/releases/tags/$tag" "$scratch/release")"
case "$status" in
    200) ;;
    404) echo "GitHub has no published release for $tag" >&2; exit 1 ;;
    *) echo "the release $tag could not be read from GitHub (HTTP $status); try again" >&2; exit 1 ;;
esac
date="$(sed -n 's/.*"published_at": *"\([0-9-]*\)T.*/\1/p' "$scratch/release" | head -n 1)"
if [ -z "$date" ]; then
    echo "GitHub's answer for $tag carries no publication date" >&2
    exit 1
fi

mkdir -p "$out"
for template in "$templates"/*.yaml; do
    sed -e "s/@VERSION@/$version/g" -e "s/@TAG@/$tag/g" \
        -e "s/@SHA256@/$sha/g" -e "s/@DATE@/$date/g" \
        "$template" > "$out/$(basename "$template")"
done
if grep -n '@[A-Z0-9]*@' "$out"/*.yaml; then
    echo "a placeholder above was left unfilled" >&2
    exit 1
fi
