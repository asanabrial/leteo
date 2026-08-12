# Leteo plugin for Codex

Makes memory automatic in Codex: each session opens holding what the project
already knows, prompts are kept, sub-agent findings are captured before their
context is discarded, and a compaction is recovered from rather than survived.

## Install

Add this repository as a plugin marketplace and install `leteo` from it, then
restart Codex.

The plugin carries configuration, not the binary. Install `leteo` first —
`curl -fsSL https://raw.githubusercontent.com/asanabrial/leteo/main/scripts/install.sh | sh`,
`cargo install leteo`, or the Windows script in the
[main README](../../README.md#install) — because every hook here and the MCP
entry itself invoke `leteo` from `PATH`.

**In Codex this bundle is the only route to the hooks.** `leteo setup codex`
registers the MCP server and nothing else, which leaves Codex holding the tools
with nothing telling it when to reach for them — measured, that shape saved
nothing across eight tasks. The MCP entry is the same either way, so running
`setup` as well is redundant rather than harmful here; on Claude Code, where
both routes install hooks, doing both registers every event twice.

**Codex gates hooks behind trust.** Observed on Codex 0.146.0: a hook config is
`Untrusted` until the directory is trusted, and an untrusted hook does not fire
and does not explain itself. If memory is not opening with your sessions, that
is the first thing to check — the tools will be working, which makes it look
like the hooks are too.

## What it does

| Codex event | Command | Effect |
| --- | --- | --- |
| `SessionStart` (`startup`, `clear`) | `leteo hook session-start` | Opens the session and hands back the project's recent work, prompts and most relevant memories |
| `SessionStart` (`compact`) | `leteo hook post-compaction` | Puts back what the compaction took, and clears what had been marked as already shown |
| `UserPromptSubmit` | `leteo hook user-prompt-submit` | Keeps the prompt, and names a memory worth having in front of you — never the same one twice in a session |
| `SubagentStop` | `leteo hook subagent-stop` | Captures a sub-agent's Key Learnings, in any of the twelve languages Leteo speaks |
| `SessionEnd` | `leteo hook session-stop` | Closes the session |

`SessionEnd` asks for three seconds rather than five because Codex clamps it
and says so on every session otherwise. Alongside the hooks,
[`.mcp.json`](.mcp.json) registers the MCP server with the `agent` tool
profile, and [`skills/memory`](skills/memory/SKILL.md) carries the protocol
that says when to use it.

## Design

The bundle is deliberately thin. Project detection, the stable manual session,
`<private>` redaction, deduplication, the save reminder and its debounce, and
the `.leteo/` import all live in the Leteo binary, shared with the CLI, the MCP
server and every other agent's hooks. These files only say which event maps to
which command.

The handlers call `leteo hook <event>` directly rather than through shell
scripts, so nothing here needs a shell — which is what makes the same bundle
work on Windows.

The five events, their matchers and their deadlines are held against the
binary's own list by a guard in the Rust crate
(`the_plugin_bundles_register_the_hooks_the_binary_writes`). This file had
already drifted from it twice — an extra `resume` on `session-start` and a `.*`
on `SubagentStop` — and neither was visible until that guard was widened to
compare matchers rather than only timeouts.
