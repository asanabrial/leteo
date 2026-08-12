#!/bin/sh
# Build the store the README's GIF is recorded against.
#
# Three memories about a payments service, of three different types, written
# the way an agent writes them: the symptom, the cause, and the thing that
# would be expensive to work out twice.
#
# A store rather than a mock-up, because the recording pipes real commands into
# a real binary. Nothing in the GIF is typed for effect — if a promise here ever
# stops holding, the next recording shows it instead of hiding it.
#
# Never the real store: it lands under `store/` beside this script, and
# `--database` keeps every command pointed at it.
set -eu

here="$(cd "$(dirname "$0")" && pwd)"
store="$here/store"
db="$store/demo.db"

command -v leteo >/dev/null 2>&1 || {
    echo "leteo is not on PATH — install it, or build with 'cargo build --release'" >&2
    exit 1
}

rm -rf "$store"
# The project is worked out from the directory, so the recording runs from one
# named after it. That is the same resolution a real session uses.
mkdir -p "$store/payments"

leteo session-start demo --project payments --directory "$store/payments" --database "$db" >/dev/null

leteo save \
    "The connection pool runs out at 20 workers" \
    "Postgres cuts off at 100 connections and each worker opens 5. Dropped to 12 workers and max_overflow to 3. The symptom was timeouts on /checkout, not database errors." \
    --project payments --type bugfix --database "$db" >/dev/null

leteo save \
    "Money is integer cents, never a float" \
    "Decided after the 3% rounding bug. Conversion happens at the edge, in money::from_cents, so nothing downstream can reintroduce a float." \
    --project payments --type decision --database "$db" >/dev/null

leteo save \
    "Stripe retries webhooks three times" \
    "Handlers have to be idempotent by event_id. Found because one charge was recorded twice in production." \
    --project payments --type discovery --database "$db" >/dev/null

echo "store built at $db"
leteo stats --database "$db"
