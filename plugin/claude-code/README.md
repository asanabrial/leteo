# Leteo plugin for Claude Code

Makes memory automatic in Claude Code: each session opens holding what the
project already knows, prompts are kept, sub-agent findings are captured before
their context is discarded, and a compaction is recovered from rather than
survived.

## Install

```text
/plugin marketplace add asanabrial/leteo
/plugin install leteo@leteo
```

The plugin carries configuration, not the binary. Install `leteo` first —
`curl -fsSL https://raw.githubusercontent.com/asanabrial/leteo/main/install.sh | sh`,
`cargo install leteo`, or the Windows script in the
[main README](../../README.md#install) — because every hook here and the MCP
entry itself invoke `leteo` from `PATH`.

**Pick this or `leteo setup claude-code --hooks`, not both.** Registered twice,
every lifecycle event runs twice, which stored each prompt twice on the machine
where it was found — 23 identical pairs before anything looked wrong from the
outside. `setup` now looks for an installed bundle and refuses rather than
adding a second registration.

## What it does

| Claude Code event | Command | Effect |
| --- | --- | --- |
| `SessionStart` (`startup`, `clear`) | `leteo hook session-start` | Opens the session and hands back the project's recent work, prompts and most relevant memories |
| `SessionStart` (`compact`) | `leteo hook post-compaction` | Puts back what the compaction took, and clears what had been marked as already shown |
| `UserPromptSubmit` | `leteo hook user-prompt-submit` | Keeps the prompt, and names a memory worth having in front of you — never the same one twice in a session |
| `SubagentStop` | `leteo hook subagent-stop` | Captures a sub-agent's Key Learnings, in any of the twelve languages Leteo speaks |
| `SessionEnd` | `leteo hook session-stop` | Closes the session |

Alongside them, [`.mcp.json`](.mcp.json) registers the MCP server with the
`agent` tool profile, so the everyday tools are there from the first message,
and [`skills/memory`](skills/memory/SKILL.md) carries the protocol that tells
the agent when to reach for them. Tools without that protocol is the shape that
measured 0 saves out of 8 tasks.

## Design

The bundle is deliberately thin. Project detection, the stable manual session,
`<private>` redaction, deduplication, the save reminder and its debounce, and
the `.leteo/` import all live in the Leteo binary, shared with the CLI, the MCP
server and every other agent's hooks. These files only say which event maps to
which command, so there is one implementation to keep correct instead of two.

The five events, their matchers and their deadlines are held against the
binary's own list by a guard in the Rust crate
(`the_plugin_bundles_register_the_hooks_the_binary_writes`). Written out here
and nowhere held, they drift — the Codex bundle had already done so twice
before the guard was widened to watch the matcher.

Every hook is best-effort and bounded: Leteo gives up before Claude Code does,
and a hook that could not do its work says so rather than failing silently.
Memory is an assistant, not a gate.
