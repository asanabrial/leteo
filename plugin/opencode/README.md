# Leteo plugin for OpenCode

Makes memory automatic in OpenCode: sessions are registered, user prompts are
persisted, sub-agent findings are captured, and the memory protocol plus this
project's prior memory are injected into every system prompt, so the agent
keeps using its tools after a compaction.

## Install

Copy the plugin next to your OpenCode configuration:

```powershell
mkdir -Force $env:USERPROFILE\.config\opencode\plugin
copy plugin\opencode\leteo.ts $env:USERPROFILE\.config\opencode\plugin\
```

```bash
mkdir -p ~/.config/opencode/plugin
cp plugin/opencode/leteo.ts ~/.config/opencode/plugin/
```

Then register the MCP server so the agent gets the tools themselves:

```powershell
leteo setup opencode
```

Restart OpenCode. `LETEO_BIN` overrides the binary when `leteo` is not on
`PATH`.

## What it does

| OpenCode event | Command | Effect |
| --- | --- | --- |
| `session.created` | `leteo hook session-start` | Creates the session, folds a renamed project's memories in, imports `.leteo/` chunks |
| `chat.message` | `leteo hook user-prompt-submit` | Persists the user prompt and reminds the agent when the project has gone quiet |
| `tool.execute.after` (Task) | `leteo hook subagent-stop` | Captures Key Learnings from sub-agent output |
| `session.deleted` | `leteo hook session-stop` | Ends the session |
| every system prompt | `leteo hook post-compaction` (once per session) | Injects the protocol and prior memory |

## Design

The plugin is deliberately thin. Project detection, the stable manual session,
`<private>` redaction, deduplication, the save reminder and its debounce, and
the `.leteo/` import all live in the Leteo binary, shared with the CLI, the MCP
server, and the Claude Code hooks. This file only translates event shapes, so
there is one implementation to keep correct instead of two.

It needs **no HTTP server and no port**. `leteo hook` writes to SQLite
directly, so nothing has to be started or health-checked, and nothing listens
on the machine. Every hook is best-effort: a missing binary, a malformed
payload, or a slow call is swallowed and the turn continues. Memory is an
assistant, not a gate.

Sub-agent sessions are ignored on purpose. A single conversation can spawn
dozens of them, and registering each one would bury the real session.
