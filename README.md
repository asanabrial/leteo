<p align="center">
  <img width="1024" alt="Leteo — living memory for AI agents. Light. Local. Yours."
       src="assets/branding/leteo-banner.png" />
</p>

<p align="center">
  <a href="#install">Install</a> &bull;
  <a href="#what-it-feels-like">How it works</a> &bull;
  <a href="#what-you-type">Commands</a> &bull;
  <a href="#languages">Languages</a> &bull;
  <a href="openspec/">Specs</a>
</p>

<p align="center">
  <a href="https://github.com/asanabrial/leteo/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/asanabrial/leteo/actions/workflows/ci.yml/badge.svg" /></a>
  <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/badge/License-MIT-blue.svg" /></a>
  <a href="https://www.rust-lang.org"><img alt="Rust 1.97+" src="https://img.shields.io/badge/rust-1.97%2B-orange.svg" /></a>
  <a href="#mcp"><img alt="MCP: 22 tools" src="https://img.shields.io/badge/MCP-22%20tools-purple.svg" /></a>
</p>

---

# Leteo — persistent memory for AI coding agents

Your coding agent forgets everything when the session ends, and most of it when
the context is compacted. Leteo is the memory it keeps: decisions, bug fixes,
conventions and the discoveries that were expensive to make, stored in a local
SQLite database and handed back when they are relevant.

One binary, no server, no API key. Nothing leaves your machine unless you turn
on cloud replication for a project you name.

**Works with:** Claude Code · Codex · Cursor · Gemini CLI · OpenCode · Windsurf ·
VS Code Copilot · Kilo Code · Qwen · Kiro · Antigravity · Pi

## Install

```bash
# Linux and macOS
curl -fsSL https://raw.githubusercontent.com/asanabrial/leteo/main/install.sh | sh
```

```powershell
# Windows
irm https://raw.githubusercontent.com/asanabrial/leteo/main/install.ps1 | iex
```

Then open Leteo and set your agent up from the Setup screen:

```bash
leteo tui
```

That is all of it. Nothing else to install first — not Rust, not SQLite, not a
runtime: the archives are prebuilt binaries with the database compiled into
them, and each script checks its download against the published `SHA256SUMS`
before installing anything.

Releases carry five builds — x86-64 Linux, Windows and macOS, and arm64 Linux
and macOS. On anything else, build from source, which is the one route that
needs Rust:

```bash
cargo install --git https://github.com/asanabrial/leteo
```

## What it feels like

You never prompt it to remember. That is the whole idea.

After `leteo setup`, your agent opens each session already holding what the
project knows: the recent decisions, what the last sessions were for, anything
you pinned. You start where you left off instead of explaining yourself again.

While you work, it saves as it goes — a bug fixed, a convention agreed,
something non-obvious learned. Those are written for its future self rather than
for you, so they stay out of the conversation. When a new memory looks like it
contradicts an older one, Leteo says so and the agent settles it quietly.

If a session goes a long time with nothing kept, Sardi — the cat who tends the
store — nudges it. That is the one line you are meant to notice.

The rest is a SQLite file you own. `leteo tui` opens it, `leteo export` takes it
with you, and `leteo delete` means it.

## MCP

A 22-tool MCP server over standard input/output, run directly or installed into
a client by the command above:

```powershell
leteo mcp
```

Nineteen of the tools are the everyday ones an agent reaches for while it works;
three change or count the whole store and sit behind a profile. `--tools` picks
a profile or a single tool, and `--project` fixes the project for the process.

Alongside it there is a JSON command line, an interactive terminal UI, and
setup support for twelve MCP clients — the list at the top of this page.

## What you type

Rarely anything. This is the store from the outside, for when you want to look:

```powershell
leteo search "connection pool" --project leteo
leteo search "connection pool" --all-projects
leteo recent --project leteo --limit 20
leteo context leteo --scope project
leteo timeline 42 --before 5 --after 5
leteo stats
leteo tui

leteo save "SQLite architecture" "One writer, many readers" --project leteo --type architecture
leteo session-start session-1 --project leteo
leteo session-end session-1

leteo setup
leteo setup claude-code --hooks
leteo setup opencode --dry-run

leteo doctor
leteo doctor --repair
leteo export --project leteo --output leteo-export.json
leteo import leteo-export.json
leteo obsidian-export --vault C:\Vaults\Notes --project leteo

leteo projects list
leteo projects consolidate --project leteo --apply
leteo projects prune --apply

leteo conflicts list --project leteo --status pending
leteo conflicts show 7
leteo conflicts scan --project leteo --apply
leteo conflicts stats --project leteo

leteo delete observation 42 --hard
leteo delete session session-1
leteo delete project leteo --hard
```

Every command prints JSON. `projects consolidate`, `projects prune` and
`conflicts scan` report what they would change and only touch data with
`--apply`. The default database is `~/.leteo/leteo.db`.

## Languages

Three settings, because they answer three different questions.

**`interface`** is Leteo's own screens: the panels, the menus, the help. Twelve
languages — English, español, português, français, Deutsch, italiano, català,
galego, euskara, Nederlands, polski, svenska — deliberately the same twelve
offered for memories, from the same table. Left unset it follows the machine's
locale, so a Spanish computer gets a Spanish dashboard without being asked.

**`voice_language`** is what Sardi speaks, and it is separate because those
lines are written *into your agent's conversation* rather than onto Leteo's
screens. Working in English on a Spanish machine is an ordinary thing to do.
Left unset it follows `interface`. It is the same twelve languages.

**`language`** is what memories are written in. It is handed to a model rather
than parsed, so it is free text and not limited to the twelve above: `español`,
`Spanish`, `português do Brasil` and `日本語` all work. Left unset, each memory
is written in the language of the conversation that produced it.

## Cloud

Optional, off by default, and per project. Your machine is the client; the cloud
never connects back.

```powershell
leteo cloud enroll --project leteo
leteo cloud status
leteo cloud sync
leteo serve
```

`leteo serve` replicates in the background until interrupted. The server side —
Compose stack, managed tokens, project grants — is in
[`openspec/specs/replication.md`](openspec/specs/replication.md).

## Coming From Engram

Leteo is an independent Rust product derived from the workflow and MIT-licensed
implementation of Gentleman Programming's Engram. It is not affiliated with or
endorsed by that project, and promises no drop-in CLI compatibility.

It reads an Engram database directly, so moving across is a copy:

```powershell
leteo import --from-engram --dry-run
leteo import --from-engram
```

It defaults to `~/.engram/engram.db`; pass `--source` for another path. The copy
folds in the write-ahead log, so a running Engram's most recent memories come
across and its own file is never written to.

## Build

Leteo requires Rust 1.97 or newer.

```powershell
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
```

The cloud tests need a real PostgreSQL and are skipped without one. Point
`TEST_DATABASE_URL` at a throwaway database and run `cargo test -- --ignored`;
they create and drop their own schema, and are not written to share one.

Contributors — human or agent — should read [`AGENTS.md`](AGENTS.md) first.

## Documentation

This file is the user-facing guide. What the system *guarantees*, and why, is in
[`openspec/`](openspec/) — one document per capability, cross-linked:

| Document | Covers |
| -------- | ------ |
| [`project.md`](openspec/project.md) | what Leteo is, the crate layout, the system-wide invariants |
| [`specs/memory-model.md`](openspec/specs/memory-model.md) | what a memory is, its types, review windows, normalisation |
| [`specs/search.md`](openspec/specs/search.md) | matching, ranking, the three stages, the narrowings |
| [`specs/store-and-schema.md`](openspec/specs/store-and-schema.md) | the database, migrations, `doctor` and its repairs |
| [`specs/mcp-tools.md`](openspec/specs/mcp-tools.md) | the MCP surface and the shape of its replies |
| [`specs/hooks.md`](openspec/specs/hooks.md) | the five lifecycle events and their time budgets |
| [`specs/cli.md`](openspec/specs/cli.md) | the command line and what its answers explain |
| [`specs/replication.md`](openspec/specs/replication.md) | the optional PostgreSQL peer |

## Environment

| Variable | Purpose |
| --- | --- |
| `LETEO_DATA_DIR` | Local data directory; defaults to `~/.leteo` |
| `LETEO_DATABASE` | Explicit local SQLite path |
| `LETEO_TOOLS` | MCP tool profiles or names; defaults to every tool |
| `LETEO_PROJECT` | Trusted process-level project for the MCP server |
| `LETEO_AGENT_CLI` | Agent CLI that judges conflict candidates: `claude` or `opencode` |
| `LETEO_SYSTEM_LANGUAGE` | Language this machine works in, when `LANG` does not say — used only to offer it in `leteo setup` |
| `LETEO_CLOUD_SERVER` | Cloud base URL for `cloud health`, `cloud sync` and the client config |
| `LETEO_DATABASE_URL` | PostgreSQL URL for cloud serve |
| `LETEO_DASHBOARD_SECRET` | Dashboard signing secret, at least 32 bytes |
| `LETEO_CLOUD_TOKEN_PEPPER` | Managed-token HMAC pepper, at least 32 bytes |
| `LETEO_CLOUD_TOKEN` | Legacy sync bearer token, at least 32 bytes |
| `LETEO_CLOUD_ADMIN` | Optional legacy admin bearer token, at least 32 bytes |
| `LETEO_CLOUD_ALLOWED_PROJECTS` | Required allowlist for legacy cloud tokens |
| `LETEO_CLOUD_HOST` | Cloud bind host; defaults to `127.0.0.1` |
| `LETEO_CLOUD_PORT` | Cloud port; defaults to `8080` |
| `LETEO_CLOUD_MAX_POOL` | PostgreSQL connection-pool limit |
| `LETEO_CLOUD_MAX_PUSH_BYTES` | Maximum cloud push body size |

## Uninstall

```powershell
leteo uninstall
leteo uninstall --yes
```

The first reports what would go and changes nothing. The second carries it out:
Leteo leaves every agent it configured, and then the machine. On Windows it also
registers itself in Installed apps, so it can be removed from there instead.

## License And Attribution

Leteo is distributed under the MIT License. See [LICENSE](LICENSE) and
[NOTICE](NOTICE) for upstream attribution and the exact reference revision.
Tagged binary archives also include a generated `THIRD_PARTY_LICENSES.html`
covering their Rust dependencies.
