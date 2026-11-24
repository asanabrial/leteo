"""One adapter per engine behind one interface.

The comparison used to know exactly two engines, and each was wired into the
loader, the search script and the page by hand. An adapter is the whole of what
the harness needs from an engine: start it isolated, write the corpus through the
engine's own write path, search through its own search path, and say which corpus
memory each returned hit is. Adding an engine is one class and one registry
entry; nothing downstream names it.

Isolation is the first rule and the one that is easy to break by accident: no
run may read or write a real store. `isolated_env` is the only place an
environment for an engine is built, and it removes every variable that could name
a real store before it sets the throwaway ones.
"""

import json
import os
import re
import shutil
import subprocess
import unicodedata

from mcpclient import BENCH, ENGRAM, LETEO, MCP, call_json, engine_cmd

# Variables an engine reads to find its data. They are removed from the inherited
# environment first, so a developer who exports one for daily use does not point
# a benchmark run at their own memory.
STORE_VARIABLES = (
    "ENGRAM_DATA_DIR", "LETEO_DATA_DIR",
    "BASIC_MEMORY_CONFIG_DIR", "BASIC_MEMORY_HOME", "BASIC_MEMORY_DEFAULT_PROJECT",
    "MEMORY_FILE_PATH",
)

# A content match shorter than this proves nothing: a ten-character snippet is
# contained in half the distractors, which share their vocabulary on purpose.
MIN_CONTENT_MATCH = 40


# Engines whose load returned in this process. Leteo reads it instead of a file
# because a file outlives the run that wrote it.
LOADED_THIS_RUN = set()


class Skipped(Exception):
    """The engine cannot be measured here, and the message says why.

    Raised for a missing binary, package manager or service. It is never turned
    into a row of zeros: a zero is a measurement, and nothing was measured.
    """


class Hit:
    """One search result, reduced to what mapping needs.

    `ref` is the engine's own identifier for it when the engine keeps one that we
    set; `text` is whatever content the reply carried, for the fallback rule.
    """

    __slots__ = ("ref", "text")

    def __init__(self, ref=None, text=""):
        self.ref = None if ref is None else str(ref)
        self.text = text or ""


def isolated_env(state, name, **extra):
    """The environment an engine runs in: a home under `state`, no real store.

    `name` keeps each engine's home apart from the others', so one engine's
    leftovers cannot answer for another.
    """
    home = os.path.join(state, "homes", name)
    os.makedirs(home, exist_ok=True)
    env = {k: v for k, v in os.environ.items() if k not in STORE_VARIABLES}
    env.update(HOME=home, USERPROFILE=home, XDG_CONFIG_HOME=os.path.join(home, ".config"),
               XDG_DATA_HOME=os.path.join(home, ".local", "share"),
               XDG_CACHE_HOME=os.path.join(home, ".cache"))
    env.update(extra)
    return env


def normalise(text):
    """Case, accents, punctuation and spacing folded away, so a snippet that an
    engine re-wrapped or re-cased still contains the text that was saved."""
    text = unicodedata.normalize("NFKD", text).casefold()
    text = "".join(c for c in text if not unicodedata.combining(c))
    return re.sub(r"[\W_]+", " ", text).strip()


def content_key(text, members):
    """The corpus memory a hit's text is, or None.

    Engine-independent: the text must contain the saved content or be a stretch of
    it of at least `MIN_CONTENT_MATCH` characters. A hit that fits more than one
    memory, or none, is unmappable; guessing between two would hand one engine a
    rank the other was denied.
    """
    seen = normalise(text)
    if len(seen) < MIN_CONTENT_MATCH:
        return None
    found = {m["key"] for m in members if normalise(m["content"]) in seen or seen in normalise(m["content"])}
    return found.pop() if len(found) == 1 else None


class Adapter:
    """The interface. A subclass sets `name` and `label` and implements the rest."""

    name = ""
    label = ""
    # The engine the page is about: if it cannot be measured the run has failed,
    # rather than quietly producing a page without it.
    required = False
    # How many independent loads the deterministic tier gives this engine. One
    # is right for an engine whose store is the same twice; an engine that fills
    # its index asynchronously is measured over several and reported as a spread.
    passes = 1

    def __init__(self, state):
        self.state = state
        self.keymap = {}
        self._servers = {}

    def check(self):
        """Raise `Skipped` when the engine's prerequisites are absent."""

    def version(self):
        raise NotImplementedError

    def load(self, members):
        """Write every member through the engine's own write path and fill
        `self.keymap` with `hit.ref -> corpus key` for what was written."""
        raise NotImplementedError

    def search(self, project, query, limit):
        raise NotImplementedError

    def map_hit(self, hit, members):
        """Provenance first; the content rule only for a hit carrying none."""
        if hit.ref is not None:
            return self.keymap.get(hit.ref)
        return content_key(hit.text, members)

    def close(self):
        for server in self._servers.values():
            server.close()
        self._servers = {}

    def _server(self, project, launch, warmup):
        if project not in self._servers:
            server = MCP(self.name, project, launch=launch())
            # The first search pays for model loads and cold caches, which is not
            # what the ranking numbers are about.
            warmup(server)
            self._servers[project] = server
        return self._servers[project]


class _SearchTool(Adapter):
    """Engram and Leteo: `mem_search` over MCP, replying `{"results": [{"id": ..}]}`."""

    def search(self, project, query, limit):
        m = self._server(project, lambda: engine_cmd(self.name, project),
                         lambda s: s.call("mem_search", {"query": "warmup"}))
        reply, _, _ = call_json(m, "mem_search", {"query": query, "limit": limit})
        return [Hit(r["id"], r.get("content")) for r in reply.get("results") or []]

    def version(self):
        r = subprocess.run([self.binary, "--version"], capture_output=True, text=True, env=dict(os.environ))
        text = (r.stdout or "") + (r.stderr or "")
        m = re.search(self.version_pattern, text)
        return m.group(1) if m else "unknown"


class EngramAdapter(_SearchTool):
    name, label, binary = "engram", "Engram", ENGRAM
    version_pattern = r"engram\s+v?(\d+\.\d+\.\d+)"

    def check(self):
        if not shutil.which(self.binary):
            raise Skipped(f"`{self.binary}` was not found on PATH; set ENGRAM_BIN to the binary")

    def load(self, members):
        # Engram's own `engram save`, as before the adapters existed: its numbers
        # are comparable with every earlier run only if it is loaded the same way.
        import load_engram
        self.keymap = {str(i): k for k, i in load_engram.load().items()}
        LOADED_THIS_RUN.add(self.name)


class LeteoAdapter(_SearchTool):
    name, label, binary = "leteo", "Leteo", LETEO
    version_pattern = r"(\d+\.\d+\.\d+)"
    required = True

    def check(self):
        if not shutil.which(self.binary):
            raise Skipped(f"`{self.binary}` was not found; set LETEO_BIN to the build under test")

    def load(self, members):
        # Leteo adopts a copy of the Engram store, so ids survive and one answer
        # key serves both. It cannot be loaded without that store, and a keymap on
        # disk does not show it: a reused BENCH_STATE keeps the one an earlier run
        # wrote, so only an Engram load that returned in this process counts.
        if EngramAdapter.name not in LOADED_THIS_RUN:
            raise Skipped("its corpus is adopted from the Engram store, and Engram was not loaded in this run")
        env = dict(os.environ, BENCH_STATE=BENCH, LETEO_BIN=self.binary)
        here = os.path.dirname(os.path.abspath(__file__))
        r = subprocess.run(["sh", "load_leteo.sh"], cwd=here, env=env, capture_output=True, text=True)
        if r.returncode:
            raise RuntimeError(f"load_leteo.sh failed: {r.stderr.strip()}")
        engram_ids = json.load(open(os.path.join(BENCH, "keymap_engram.json")))
        self.keymap = {str(i): k for k, i in engram_ids.items()}


def _tool_cache(*cmd):
    """A package manager's own cache directory, or None.

    Resolved by the tool that owns it, in the caller's real environment, so a
    redirected HOME does not make every run download the packages again.
    """
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=30)
    except (OSError, subprocess.TimeoutExpired):
        return None
    out = r.stdout.strip()
    return out if r.returncode == 0 and out else None


def package_cache_env():
    env = {}
    uv = _tool_cache("uv", "cache", "dir")
    if uv:
        env["UV_CACHE_DIR"] = uv
    npm = _tool_cache("npm", "config", "get", "cache")
    if npm:
        env["npm_config_cache"] = npm
    return env


def stored_titles(members):
    """The name each memory is stored under: its title, made unique per project.

    Both external engines key a note or an entity by name, and the corpus holds
    one pair of distractors with the same title in one project (`c5` and `c51`).
    Engram's own `save` folds such a pair into one observation; writing the
    second over the first here would silently drop a memory from these engines
    too, so the later one carries a ` (2)`. Only a distractor is touched, and the
    text that is searched stays as it was.
    """
    seen, out = {}, {}
    for m in members:
        n = seen[(m["project"], m["title"])] = seen.get((m["project"], m["title"]), 0) + 1
        out[m["key"]] = m["title"] if n == 1 else f"{m['title']} ({n})"
    return out


class _External(Adapter):
    """An engine that is a separate package started through a package runner.

    Each project gets its own server and its own store, which is how Engram and
    Leteo are asked too: a query is put to the project it belongs to and cannot
    be answered from the other's memories.
    """

    runner = ""
    needs = ""

    def check(self):
        if not shutil.which(self.runner):
            raise Skipped(f"`{self.runner}` was not found on PATH; it is needed to run {self.label} ({self.needs})")

    def command(self):
        raise NotImplementedError

    def env_for(self, project):
        raise NotImplementedError

    def launch(self, project):
        cwd = os.path.join(BENCH, "work", f"{self.name}-{project}")
        os.makedirs(cwd, exist_ok=True)
        return self.command(), cwd, self.env_for(project)

    def open(self, project):
        return self._server(project, lambda: self.launch(project), self.warm)

    def by_project(self, members):
        out = {}
        for m in members:
            out.setdefault(m["project"], []).append(m)
        return out


class BasicMemoryAdapter(_External):
    """basic-memory, AGPL-3.0: run only as an external process, nothing vendored."""

    name, label, runner = "basic-memory", "basic-memory", "uvx"
    needs = "uvx runs the pinned release"
    PIN = "1.0.0"
    # On 0.23.2 its full-text index was filled asynchronously after a note was
    # written, with no completion signal to wait on (`status --wait` was
    # documented to return immediately), so two loads of the same corpus ranked
    # a paraphrase differently and the row was reported as a spread. On 1.0.0
    # five loads gave identical figures. It stays at five: that they agree is
    # what was measured, and one load could not have shown it.
    passes = 5
    # 1.0.0 renamed its tools (`write_note` to `bm_write`, `search_notes` to
    # `bm_search`) and refuses either without a scope: a search with none
    # answers INVALID_ARGUMENTS, "Search needs an explicit scope". `main` is the
    # project it creates by default under BASIC_MEMORY_HOME, and each corpus
    # project already has a home of its own, so it is the only one there.
    SCOPE = "main"

    def command(self):
        return [self.runner, f"basic-memory@{self.PIN}", "mcp"]

    def paths(self, project):
        root = os.path.join(BENCH, "basic-memory", project)
        return os.path.join(root, "config"), os.path.join(root, "notes")

    def env_for(self, project):
        config, home = self.paths(project)
        os.makedirs(home, exist_ok=True)
        return isolated_env(
            BENCH, f"basic-memory-{project}", BASIC_MEMORY_CONFIG_DIR=config, BASIC_MEMORY_HOME=home,
            BASIC_MEMORY_SEMANTIC_EMBEDDING_CACHE_DIR=os.path.join(BENCH, "basic-memory", "embedding-model"),
            **package_cache_env())

    def version(self):
        env = self.env_for("version")
        r = subprocess.run(self.command()[:-1] + ["--version"], capture_output=True, text=True, env=env, timeout=300)
        m = re.search(r"version:\s*(\d+\.\d+\.\d+)", (r.stdout or "") + (r.stderr or ""))
        if not m:
            raise RuntimeError(f"`basic-memory --version` printed no version: {(r.stderr or r.stdout).strip()[:200]}")
        return m.group(1)

    def warm(self, server):
        call_json(server, "bm_search", {"query": "warmup", "project": self.SCOPE, "output_format": "json"})

    def load(self, members):
        shutil.rmtree(os.path.join(BENCH, "basic-memory"), ignore_errors=True)
        names = stored_titles(members)
        for project, group in self.by_project(members).items():
            server = self.open(project)
            for m in group:
                args = {"title": names[m["key"]], "content": m["content"], "directory": project,
                        "note_type": m["type"], "project": self.SCOPE, "output_format": "json"}
                if m["topic"]:
                    args["tags"] = [m["topic"]]
                reply, _, _ = call_json(server, "bm_write", args)
                self.keymap[reply["file_path"]] = m["key"]

    def search(self, project, query, limit):
        reply, _, _ = call_json(self.open(project), "bm_search",
                                {"query": query, "page_size": limit, "project": self.SCOPE, "output_format": "json"})
        return [Hit(r.get("file_path"), r.get("content") or r.get("matched_chunk")) for r in reply.get("results") or []]


class ServerMemoryAdapter(_External):
    """The reference MCP knowledge-graph server.

    Its `search_nodes` is an unranked, case-insensitive substring match of the
    whole query against entity names, types and observations: the order is the
    order stored, and a multi-word query matches only if those words appear
    together and in that order. An empty answer to such a query is what the
    server does, not a fault of the adapter.
    """

    name, label, runner = "server-memory", "server-memory", "npx"
    needs = "npx runs the pinned release"
    PIN = "2026.8.31"

    def command(self):
        return [self.runner, "-y", f"@modelcontextprotocol/server-memory@{self.PIN}"]

    def env_for(self, project):
        store = os.path.join(BENCH, "server-memory")
        os.makedirs(store, exist_ok=True)
        return isolated_env(BENCH, f"server-memory-{project}",
                            MEMORY_FILE_PATH=os.path.join(store, f"{project}.jsonl"), **package_cache_env())

    def version(self):
        # npx cannot print the version it ran, and the server reports its own
        # protocol-level string in `initialize`, which is not the package's. The
        # pin is the version that runs.
        return self.PIN

    def warm(self, server):
        call_json(server, "search_nodes", {"query": "warmup"})

    def load(self, members):
        shutil.rmtree(os.path.join(BENCH, "server-memory"), ignore_errors=True)
        names = stored_titles(members)
        for project, group in self.by_project(members).items():
            server = self.open(project)
            for m in group:
                call_json(server, "create_entities", {"entities": [
                    {"name": names[m["key"]], "entityType": m["type"], "observations": [m["content"]]}]})
                self.keymap[f"{project}/{names[m['key']]}"] = m["key"]

    def search(self, project, query, limit):
        reply, _, _ = call_json(self.open(project), "search_nodes", {"query": query})
        return [Hit(f"{project}/{e['name']}", " ".join(e.get("observations") or [])) for e in reply.get("entities") or []]


# The deterministic tier: no model, no network at query time beyond fetching the
# engine itself, so a re-run on the same versions gives the same ranking.
DETERMINISTIC = (EngramAdapter, LeteoAdapter, BasicMemoryAdapter, ServerMemoryAdapter)
