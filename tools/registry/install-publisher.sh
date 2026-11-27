#!/bin/sh
# Installs the mcp-publisher release the MCP registry job publishes with, and
# refuses anything but the bytes recorded beside it.
#
#   sh tools/registry/install-publisher.sh --platform linux/amd64
#
# The body used to sit inline in `publish-registries.yml`, on a job holding
# `id-token: write`, and only a release dispatch ever ran it: neither the digest
# refusal nor the unknown-platform branch had executed anywhere before the day
# it would matter. Moved here so `tools/registry/tests/check.sh` can run it
# behind stand-in `curl` and `sha256sum`, on any runner, with no network and no
# release.
#
# The version and the four digests come from the environment, recorded in the
# workflow beside the step that calls this. An unset digest fails by the name of
# the variable that should have carried it, before anything is downloaded: `set
# -u` would abort on it too, but under its own wording, which says nothing about
# which platform has no recorded bytes.
set -eu

platform=""
while [ $# -gt 0 ]; do
    case "$1" in
        --platform)
            platform="${2:?--platform needs an os/arch argument}"
            shift 2
            ;;
        *)
            echo "usage: install-publisher.sh --platform <os/arch>" >&2
            exit 2
            ;;
    esac
done
if [ -z "$platform" ]; then
    echo "usage: install-publisher.sh --platform <os/arch>" >&2
    exit 2
fi
: "${MCP_PUBLISHER:?MCP_PUBLISHER is not set}"

case "$platform" in
    linux/amd64) digest="${SHA256_LINUX_AMD64:-}" ; name=SHA256_LINUX_AMD64 ;;
    linux/arm64) digest="${SHA256_LINUX_ARM64:-}" ; name=SHA256_LINUX_ARM64 ;;
    darwin/amd64) digest="${SHA256_DARWIN_AMD64:-}" ; name=SHA256_DARWIN_AMD64 ;;
    darwin/arm64) digest="${SHA256_DARWIN_ARM64:-}" ; name=SHA256_DARWIN_ARM64 ;;
    *)
        echo "No sha256 recorded for mcp-publisher $MCP_PUBLISHER on $platform." >&2
        exit 1
        ;;
esac
if [ -z "$digest" ]; then
    echo "No sha256 recorded for mcp-publisher $MCP_PUBLISHER on $platform: $name is unset." >&2
    exit 1
fi

archive="mcp-publisher_$(printf '%s' "$platform" | tr '/' '_').tar.gz"
curl -fsSLO "https://github.com/modelcontextprotocol/registry/releases/download/v$MCP_PUBLISHER/$archive"
printf '%s  %s\n' "$digest" "$archive" | sha256sum -c -
tar -xzf "$archive" mcp-publisher

# The version was printed and compared with nothing, so a release that served a
# different binary under the recorded tag would publish with the identity the
# registry trusts. It is asserted here instead: the output is kept in the log,
# and the run stops when it names another version.
version="$(./mcp-publisher --version)"
printf '%s\n' "$version"
found=""
for token in $version; do
    case "$token" in
        "$MCP_PUBLISHER" | "v$MCP_PUBLISHER") found=yes ;;
    esac
done
if [ -z "$found" ]; then
    echo "The extracted mcp-publisher answered '$version', which names no version $MCP_PUBLISHER." >&2
    exit 1
fi
