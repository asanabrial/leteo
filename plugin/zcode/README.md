# Leteo plugin for ZCode

Makes memory automatic in ZCode: each session opens holding what the project
already knows, prompts are kept, and a compaction is recovered from rather than
survived.

## Install

```text
zcode plugins marketplace add asanabrial/leteo
zcode plugins install leteo@leteo
```

The plugin carries configuration, not the binary. Install `leteo` first —
`curl -fsSL https://raw.githubusercontent.com/asanabrial/leteo/main/scripts/install.sh | sh`,
`cargo install leteo`, or the Windows script in the
[main README](../../README.md#install) — because every hook here and the MCP
entry itself invoke `leteo` from `PATH`.

**Pick this or `leteo setup zcode --hooks`, not both.** Registered twice, every
lifecycle event runs twice, which stored each prompt twice on the machine where
it was found — 23 identical pairs before anything looked wrong from the outside.
`setup` looks for an installed bundle under `~/.zcode/cli/plugins/cache` and
refuses rather than adding a second registration.

## Why the bundle and not just `setup`

Both routes reach the same three events, but they do not have the same failure.

ZCode runs configuration-file hooks only while `hooks.enabled` is true in
`~/.zcode/cli/config.json`, and that switch starts off. `leteo setup zcode
--hooks` turns it on, and refuses where somebody has deliberately turned it back
off — writing registrations into a block the client will not read is a setup
reporting success over a file nothing opens. A plugin bundle does not depend on
that switch: enabling the plugin is what enables its hooks.

So on a machine where the switch is somebody else's decision, this is the route
that works.

## What it does

| ZCode event | Command | Effect |
| --- | --- | --- |
| `SessionStart` (`startup`, `clear`) | `leteo hook session-start` | Opens the session and hands back the project's recent work, prompts and most relevant memories |
| `SessionStart` (`compact`) | `leteo hook post-compaction` | Puts back what the compaction took, and clears what had been marked as already shown |
| `UserPromptSubmit` | `leteo hook user-prompt-submit` | Keeps the prompt, and names a memory worth having in front of you — never the same one twice in a session |

Three, where the Claude Code and Codex bundles register five. ZCode fires seven
lifecycle events — `SessionStart`, `UserPromptSubmit`, `PreToolUse`,
`PermissionRequest`, `PostToolUse`, `PostToolUseFailure`, `Stop` — and neither
`SubagentStop` nor `SessionEnd` is among them, so `subagent-stop` and
`session-stop` have nowhere to land.

`session-stop` is deliberately **not** moved onto `Stop` to fill the gap. `Stop`
fires when the agent finishes a reply, at the end of every turn rather than at
the end of the conversation; registered there once, it ended the session on
every single prompt, which deleted the save reminder's debounce and made the
reminder appear on every prompt instead of every fifteen minutes. On ZCode the
closing summary therefore comes from the agent calling `mem_session_summary`
itself, which [`skills/memory`](skills/memory/SKILL.md) tells it to do.

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

Which events belong here is not written out twice either: the crate holds one
list per agent — `ZCODE_HOOK_REGISTRATIONS` for this one — and a guard
(`the_plugin_bundles_register_the_hooks_the_binary_writes`) reads this file
against the same list the installer writes from, matchers and deadlines
included. Written out here and nowhere held, they drift: the Codex bundle had
already done so twice before that guard was widened to watch the matcher.

The manifest lives at [`.zcode-plugin/plugin.json`](.zcode-plugin/plugin.json),
which is the path ZCode looks for first; it falls back to `.claude-plugin/` for
Claude Code compatibility, and this bundle does not use that fallback because
the Claude Code bundle beside it is a different set of events.

Every hook is best-effort and bounded: Leteo gives up before ZCode does, and a
hook that could not do its work says so rather than failing silently. Memory is
an assistant, not a gate.
