# Working on Leteo

The house rules live in [`AGENTS.md`](AGENTS.md), and this file imports them.

Every other agent — Codex, Cursor, Gemini CLI, OpenCode, Windsurf — reads
`AGENTS.md` from the project root directly. Claude Code reads this file and
only this one, so the two names have to end up with the same rules.

This used to be a symbolic link, which is the other documented way to do it and
the one that does not survive a clone: Git for Windows leaves `core.symlinks`
off, and `core.symlinks` is a client setting that no repository can turn on for
whoever clones it — not through `.gitattributes`, not through a hook, and
deliberately so. Measured rather than assumed: `git clone -c core.symlinks=false`
of this repository produced a `CLAUDE.md` of nine bytes whose entire contents
were the string `AGENTS.md`, which Claude Code would have read as the project's
instructions without anything going wrong out loud.

@AGENTS.md
