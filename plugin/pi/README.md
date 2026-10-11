# Leteo package for Pi

Makes memory automatic in [Pi](https://pi.dev): sessions are opened and closed,
user prompts are persisted, tool output that ends in a Key Learnings section is
captured, the compaction summary is saved and recovered, and the memory
protocol is appended to every system prompt. Leteo's memory tools are
registered natively.

## Install

```bash
leteo setup pi
```

writes this package into Pi's agent directory (`PI_CODING_AGENT_DIR`, or
`~/.pi/agent`) and lists it under `packages` in Pi's `settings.json`. The files
are built into the binary, so nothing is fetched and there is no npm package;
`leteo setup pi` again after an upgrade replaces them. Restart Pi, or run
`/reload`.

`LETEO_BIN` overrides the binary setup recorded, which in turn is preferred to
`leteo` on `PATH`. The package is also published to npm as
`@asanabrial/leteo-pi` for `pi install npm:@asanabrial/leteo-pi`; installed that
way setup has recorded nothing, so `leteo` must be on `PATH` or `LETEO_BIN` set,
and `leteo setup pi` remains the route that needs neither.

## What it does

| Pi event | Command | Effect |
| --- | --- | --- |
| extension load | `pi.registerMcpServer("leteo", …)` | Registers `leteo mcp --tools=agent`; Pi connects it when the session starts |
| `session_start` | `leteo hook session-start` | Opens the session and holds the project's recent work for the first turn |
| `before_agent_start` | `leteo hook user-prompt-submit` | Appends the protocol to the system prompt and persists the prompt. Input another extension injected is not persisted |
| `tool_call` | none | Adds the Pi session's id to a Leteo tool call whose schema takes one |
| `tool_execution_end` | `leteo hook subagent-stop` | Captures Key Learnings from the output of any tool but Leteo's own, when it is longer than 50 characters. Output with no learnings section stores nothing |
| `session_compact` | `leteo save --type session_summary`, then `leteo hook post-compaction` | Saves Pi's summary, recovers context, and hands it over on the next turn |
| `session_shutdown` | `leteo hook session-stop` | Ends the session, unless Pi is only reloading |

The memory tools are Leteo's MCP server, registered by the extension rather than
by an entry in `mcp.json`, and exposed `direct`: the protocol tells the model to
save unprompted, in the middle of other work, and a tool that is not declared or
has to be searched for first is one it will not think to call. Pi names them
`mcp__leteo__<tool>`, and the appended protocol says so. A `leteo` entry in
`mcp.json` would override this registration by name, which is why setup removes
the one an earlier version wrote.

A tool call carries the Pi session's id unless the model names another, which
files it under the conversation and project the session was opened in rather
than the server's shared manual session. Pi lets an extension change a call's
input from `tool_call`, and the schema Pi reports decides which tools take the
id.

## Design

The package is deliberately thin and starts nothing itself. Project detection, the stable session,
`<private>` redaction, the length bound, deduplication and the save reminder
all live in the Leteo binary. Nothing here redacts or truncates before the
binary sees the text: a prompt cut at a count could leave half a `<private>`
span with no closing tag.

Every hook is best-effort and bounded. A missing binary or a malformed payload
costs the memory, never the turn. Capture after a
tool is not awaited, so a process per tool call does not hold the agent up.

The hooks run with `LETEO_HOOK_CLIENT=pi`. Claude Code's hooks are plain
`leteo hook` commands that also fire inside Pi, because pi-claude-bridge loads
the user's Claude Code settings, and the binary makes them do nothing when it
sees `PI_CODING_AGENT`. This package inherits that variable, so it says who it
is.

## Tests

```bash
cd plugin/pi && node --test leteo.test.ts
```

They run against a fake `pi` and a fake `leteo` and start neither.

Not ported from Engram's package: sessions per foreign project.
