# @asanabrial/leteo

Local-first persistent memory for AI coding agents. One binary over one SQLite
database — decisions, bug fixes and conventions saved as they happen, handed
back when they are relevant.

The package is scoped and the command is not: `npx @asanabrial/leteo` and, for
a global install, `leteo`. npm refuses the bare name — it reads `leteo` as too
close to `level`, `leven` and `metro` — and a scope is what the MCP servers
that ship this way use anyway.

Full documentation lives at
[github.com/asanabrial/leteo](https://github.com/asanabrial/leteo).

## What this package is

A way to run Leteo without installing anything first. It downloads the release
binary for your platform, checks it against the published `SHA256SUMS`, and
hands every argument to it.

In an MCP client's configuration:

```json
{
  "mcpServers": {
    "leteo": {
      "command": "npx",
      "args": ["-y", "@asanabrial/leteo", "mcp"]
    }
  }
}
```

Or straight from a terminal:

```sh
npx @asanabrial/leteo tui
npx @asanabrial/leteo search "connection pool"
```

The npm version *is* the release tag, so pinning a version here pins the binary
it fetches, and nothing else.

## What this package is not

It is not a JavaScript reimplementation, and it is not the recommended install.
Leteo is a Rust binary; the shortest path to it is still

```sh
curl -fsSL https://raw.githubusercontent.com/asanabrial/leteo/main/install.sh | sh
```

which puts `leteo` on your `PATH` once, or `cargo install leteo`. This package
exists because most MCP documentation assumes `npx`, and pasting three lines
into a JSON file should not require reading an install script first.

It is also not the way to run `leteo setup`, and that command now refuses when
it notices. `setup` writes the path of the running binary into your agent's
configuration, and the binary this package downloads lives inside npm's cache —
a path `npm cache clean` deletes and the next version replaces. The
configuration would keep working until it suddenly did not, and the hooks would
fail without saying so. The three lines above have no such problem: `npx` is on
your `PATH` and resolves the package itself, every time.

Five platforms have prebuilt binaries: Linux, macOS and Windows on x86-64, and
Linux and macOS on arm64. Anywhere else, `cargo install leteo` builds it.

## Environment

- `LETEO_VERSION` — fetch a release tag other than this package's version.
- `LETEO_BASE_URL` — download from somewhere other than GitHub releases, for a
  mirror serving the same layout.

Both belong to this wrapper, not to the binary, and behave as they do in
`install.sh`.

The checksum is always verified and there is no flag to skip it: this runs
unattended inside an agent, which is exactly where nobody would notice a
substituted archive.

## License

MIT. Leteo is a reimplementation of Engram; the attribution in `NOTICE` and
`LICENSE` in the main repository stays.
