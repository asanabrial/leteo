// Contract tests for the Pi package.
//
// The extension is held to the Pi API it was written against by a fake `pi`
// that records what is registered, and to the Leteo binary by a fake `leteo`
// that records what it is asked to do. Nothing here starts Pi or the real
// binary, and nothing touches a real store.
//
// Run with: `node --test leteo.test.ts` from this directory.
import { test } from "node:test"
import assert from "node:assert/strict"
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { fileURLToPath, pathToFileURL } from "node:url"

import leteo, {
  attemptHook,
  exposureFor,
  leteoBinary,
  piVersion,
  resultText,
  runHook,
  summaryTitle,
} from "./leteo.ts"

const HERE = fileURLToPath(new URL(".", import.meta.url))

// A stand-in for the `leteo` binary. `hook` records its payload and answers the
// way the real one does; `save` records its arguments and where it ran.
const FAKE_LETEO = `#!/usr/bin/env node
const fs = require("node:fs")
const log = (entry) => fs.appendFileSync(process.env.LETEO_FAKE_LOG, JSON.stringify(entry) + "\\n")
const [command, ...rest] = process.argv.slice(2)
if (command === "hook") {
  let body = ""
  process.stdin.setEncoding("utf8")
  process.stdin.on("data", (chunk) => (body += chunk))
  process.stdin.on("end", () => {
    log({ kind: "hook", event: rest[0], client: process.env.LETEO_HOOK_CLIENT, payload: JSON.parse(body || "{}") })
    const context = { "session-start": "OPENING CONTEXT", "post-compaction": "RECOVERED CONTEXT", "user-prompt-submit": "RECALL HINT" }[rest[0]]
    process.stdout.write(context ? JSON.stringify({ hookSpecificOutput: { additionalContext: context } }) : "{}")
    if (process.env.LETEO_FAKE_HOOK_EXIT) process.exit(Number(process.env.LETEO_FAKE_HOOK_EXIT))
  })
} else if (command === "save") {
  log({ kind: "save", args: rest, cwd: process.cwd() })
  process.exit(Number(process.env.LETEO_FAKE_SAVE_EXIT || 0))
}
`

interface Harness {
  log: () => any[]
  cleanup: () => void
}

/** Points the extension at the fake binary and a fresh log, and undoes it. */
function harness(env: Record<string, string | undefined> = {}): Harness {
  const directory = mkdtempSync(join(tmpdir(), "leteo-pi-"))
  const binary = join(directory, "leteo")
  const logPath = join(directory, "calls.log")
  writeFileSync(binary, FAKE_LETEO)
  chmodSync(binary, 0o755)
  writeFileSync(logPath, "")
  const previous: Record<string, string | undefined> = {}
  const values = { LETEO_BIN: binary, LETEO_FAKE_LOG: logPath, ...env }
  for (const [key, value] of Object.entries(values)) {
    previous[key] = process.env[key]
    if (value === undefined) delete process.env[key]
    else process.env[key] = value
  }
  return {
    log: () =>
      readFileSync(logPath, "utf8")
        .split("\n")
        .filter(Boolean)
        .map((line) => JSON.parse(line)),
    cleanup() {
      for (const [key, value] of Object.entries(previous)) {
        if (value === undefined) delete process.env[key]
        else process.env[key] = value
      }
    },
  }
}

function fakePi(allTools: any[] = []) {
  const handlers = new Map<string, (...args: any[]) => any>()
  const servers = new Map<string, any>()
  return {
    handlers,
    servers,
    on(name: string, handler: (...args: any[]) => any) {
      handlers.set(name, handler)
      return () => {}
    },
    registerMcpServer(name: string, config: any) {
      servers.set(name, config)
    },
    getAllTools: () => allTools,
  }
}

function context(id = "pi-session-1", cwd = "/work/project") {
  return {
    cwd,
    sessionManager: { getSessionId: () => id },
  }
}

const hooks = (h: Harness, event: string) =>
  h.log().filter((entry) => entry.kind === "hook" && entry.event === event)

test("the manifest names the extension Pi loads", () => {
  const manifest = JSON.parse(readFileSync(new URL("./package.json", import.meta.url), "utf8"))
  assert.ok(manifest.keywords.includes("pi-package"))
  assert.deepEqual(manifest.pi.extensions, ["./leteo.ts"])
  assert.ok(existsSync(join(HERE, "leteo.ts")))
})

test("the factory registers the MCP server with the exposure the running Pi takes and subscribes to the lifecycle events", async () => {
  const h = harness()
  try {
    const pi = fakePi()
    await leteo(pi as any)
    const server = pi.servers.get("leteo")
    assert.equal(server.command, process.env.LETEO_BIN)
    assert.deepEqual(server.args, ["mcp", "--tools=agent"])
    assert.equal(
      server.exposure,
      exposureFor(await piVersion()),
      "the exposure follows the version Pi exports, which no Pi answers in this runner",
    )
    assert.ok(server.description)
    for (const name of [
      "session_start",
      "before_agent_start",
      "tool_execution_end",
      "session_compact",
      "session_shutdown",
      "tool_call",
    ]) {
      assert.equal(typeof pi.handlers.get(name), "function", `${name} is subscribed`)
    }
    // Pi loads extensions in invocations that never start a session.
    assert.deepEqual(h.log(), [])
  } finally {
    h.cleanup()
  }
})

test("the exposure is deferred from 1.2.0 and direct below it", () => {
  assert.equal(exposureFor("1.2.0"), "deferred")
  assert.equal(exposureFor("1.2.1"), "deferred")
  assert.equal(exposureFor("2.0.0"), "deferred")
  assert.equal(exposureFor("1.1.0"), "direct")
  assert.equal(exposureFor("1.0.4"), "direct")
  assert.equal(exposureFor("0.99.2"), "direct")
  assert.equal(exposureFor(undefined), "direct", "a Pi whose version cannot be read keeps the tools declared")
  assert.equal(exposureFor("1.2"), "deferred", "a missing part counts as zero, so 1.2 is 1.2.0")
  assert.equal(exposureFor("not a version"), "direct")
})

/** The extension as Pi loads it, in a directory whose `node_modules` answers Pi's version. */
function extensionUnderPi(version: string): string {
  const directory = mkdtempSync(join(tmpdir(), "leteo-pi-version-"))
  const extension = join(directory, "leteo.ts")
  writeFileSync(extension, readFileSync(new URL("./leteo.ts", import.meta.url), "utf8"))
  const packageDirectory = join(directory, "node_modules", "@earendil-works", "pi-coding-agent")
  mkdirSync(packageDirectory, { recursive: true })
  writeFileSync(
    join(packageDirectory, "package.json"),
    JSON.stringify({ name: "@earendil-works/pi-coding-agent", version, type: "module", main: "index.js" }),
  )
  writeFileSync(join(packageDirectory, "index.js"), `export const VERSION = ${JSON.stringify(version)}\n`)
  return extension
}

test("the registration follows the version Pi itself exports", async () => {
  const h = harness()
  try {
    for (const [version, exposure] of [
      ["1.2.0", "deferred"],
      ["1.1.0", "direct"],
    ] as const) {
      const extension = await import(pathToFileURL(extensionUnderPi(version)).href)
      assert.equal(await extension.piVersion(), version, "the version comes from the Pi that loaded the package")
      const pi = fakePi()
      await extension.default(pi as any)
      assert.equal(pi.servers.get("leteo").exposure, exposure, `the exposure Pi ${version} is given`)
    }
  } finally {
    h.cleanup()
  }
})

test("session_start opens the session as the Pi package", async () => {
  const h = harness()
  try {
    const pi = fakePi()
    await leteo(pi as any)
    await pi.handlers.get("session_start")!({ type: "session_start", reason: "startup" }, context())
    const [started] = hooks(h, "session-start")
    assert.deepEqual(started.payload, {
      session_id: "pi-session-1",
      cwd: "/work/project",
      without_directive: true,
    })
    assert.equal(started.client, "pi", "the binary's Pi guard lets this caller through")
  } finally {
    h.cleanup()
  }
})

test("a Leteo tool call is bound to the Pi session when its schema takes one", async () => {
  const h = harness()
  try {
    const pi = fakePi([
      { name: "mcp__leteo__mem_save", parameters: { properties: { title: {}, session_id: {} } } },
      { name: "mcp__leteo__mem_stats", parameters: { properties: {} } },
      { name: "bash", parameters: { properties: { session_id: {} } } },
    ])
    await leteo(pi as any)
    const call = pi.handlers.get("tool_call")!

    const save = { toolName: "mcp__leteo__mem_save", input: { title: "A decision" } as any }
    call(save, context())
    assert.equal(save.input.session_id, "pi-session-1")

    const named = { toolName: "mcp__leteo__mem_save", input: { title: "x", session_id: "other" } as any }
    call(named, context())
    assert.equal(named.input.session_id, "other", "a session the caller named is the caller's to name")

    const stats = { toolName: "mcp__leteo__mem_stats", input: {} as any }
    call(stats, context())
    assert.equal(stats.input.session_id, undefined, "a tool that takes no session is not given one")

    const bash = { toolName: "bash", input: {} as any }
    call(bash, context())
    assert.equal(bash.input.session_id, undefined, "another tool's input is not touched")
  } finally {
    h.cleanup()
  }
})

test("before_agent_start appends the protocol, the tool names, the opening context once, and captures the prompt", async () => {
  const h = harness()
  try {
    const pi = fakePi()
    await leteo(pi as any)
    await pi.handlers.get("session_start")!({ type: "session_start", reason: "startup" }, context())

    const options = { appendSystemPrompt: "existing" }
    const result = await pi.handlers.get("before_agent_start")!(
      {
        type: "before_agent_start",
        prompt: "Why did we choose SQLite?",
        systemPrompt: "base",
        systemPromptOptions: options,
      },
      context(),
    )
    assert.equal(result, undefined, "newer Pi receives it through the structured options")
    assert.ok(options.appendSystemPrompt.startsWith("existing\n\n## Leteo Persistent Memory"))
    assert.ok(options.appendSystemPrompt.includes("mcp__leteo__mem_save"), "the model is told what Pi calls the tools")
    assert.ok(options.appendSystemPrompt.includes("OPENING CONTEXT"))
    assert.ok(
      options.appendSystemPrompt.includes("RECALL HINT"),
      "what the per-prompt hook named reaches the model, because the hook records it as shown",
    )
    assert.deepEqual(
      hooks(h, "user-prompt-submit").map((entry) => entry.payload),
      [{ session_id: "pi-session-1", cwd: "/work/project", prompt: "Why did we choose SQLite?" }],
    )

    const second = { appendSystemPrompt: "" }
    await pi.handlers.get("before_agent_start")!(
      { type: "before_agent_start", prompt: "short", systemPrompt: "base", systemPromptOptions: second },
      context(),
    )
    assert.ok(second.appendSystemPrompt.startsWith("## Leteo Persistent Memory"))
    assert.ok(!second.appendSystemPrompt.includes("OPENING CONTEXT"), "the opening context is handed over once")
    assert.ok(!second.appendSystemPrompt.includes("RECALL HINT"), "a prompt the hook never saw carries no hint")
    assert.equal(hooks(h, "user-prompt-submit").length, 1, "a prompt of ten characters or fewer is not kept")
  } finally {
    h.cleanup()
  }
})

test("a reload does not hand the opening context over a second time", async () => {
  const h = harness()
  try {
    const pi = fakePi()
    await leteo(pi as any)
    await pi.handlers.get("session_start")!({ type: "session_start", reason: "reload" }, context())
    const options = { appendSystemPrompt: "" }
    await pi.handlers.get("before_agent_start")!(
      { type: "before_agent_start", prompt: "continue the work", systemPrompt: "", systemPromptOptions: options },
      context(),
    )
    assert.ok(!options.appendSystemPrompt.includes("OPENING CONTEXT"))
  } finally {
    h.cleanup()
  }
})

test("an older Pi gets the protocol through the returned system prompt", async () => {
  const h = harness()
  try {
    const pi = fakePi()
    await leteo(pi as any)
    const result = await pi.handlers.get("before_agent_start")!(
      { type: "before_agent_start", prompt: "hi", systemPrompt: "base prompt" },
      context(),
    )
    assert.ok(result.systemPrompt.startsWith("base prompt\n\n## Leteo Persistent Memory"))
  } finally {
    h.cleanup()
  }
})

test("a prompt that another extension injected is not captured", async () => {
  const h = harness()
  try {
    const pi = fakePi()
    await leteo(pi as any)
    pi.handlers.get("input")!({ type: "input", text: "synthetic", source: "extension" }, context())
    await pi.handlers.get("before_agent_start")!(
      {
        type: "before_agent_start",
        prompt: "an injected prompt of some length",
        systemPrompt: "",
        systemPromptOptions: {},
      },
      context(),
    )
    assert.equal(hooks(h, "user-prompt-submit").length, 0)

    pi.handlers.get("input")!({ type: "input", text: "typed", source: "interactive" }, context())
    await pi.handlers.get("before_agent_start")!(
      {
        type: "before_agent_start",
        prompt: "a prompt the user typed in",
        systemPrompt: "",
        systemPromptOptions: {},
      },
      context(),
    )
    assert.equal(hooks(h, "user-prompt-submit").length, 1)
  } finally {
    h.cleanup()
  }
})

test("tool results are captured except Leteo's own and anything of 50 characters or fewer", async () => {
  const h = harness()
  try {
    const pi = fakePi()
    await leteo(pi as any)
    const end = pi.handlers.get("tool_execution_end")!
    const learnings = "## Key Learnings:\n1. The hook path never blocks the turn\n2. Capture runs after the tool"
    const text = (value: string) => ({ content: [{ type: "text", text: value }] })

    end({ toolName: "mcp__leteo__mem_save", result: text(learnings) }, context())
    end({ toolName: "MCP__LETEO__MEM_SEARCH", result: text(learnings) }, context())
    end({ toolName: "bash", result: text("x".repeat(50)) }, context())
    end({ toolName: "bash", result: undefined }, context())
    end({ toolName: "subagent", result: text(learnings) }, context())
    // Capture is not awaited by the handler; shutdown is what waits for it, so
    // by the time it returns every capture has run.
    await pi.handlers.get("session_shutdown")!({ type: "session_shutdown", reason: "quit" }, context())

    const captured = hooks(h, "subagent-stop")
    assert.equal(captured.length, 1)
    assert.deepEqual(captured[0].payload, {
      session_id: "pi-session-1",
      cwd: "/work/project",
      stdout: learnings,
      source: "subagent",
    })
    assert.equal(captured[0].client, "pi")
    assert.equal(hooks(h, "session-stop").length, 1, "and the session ends after them")
  } finally {
    h.cleanup()
  }
})

test("shutdown waits for a capture still running before it ends the session", async () => {
  const h = harness()
  try {
    const pi = fakePi()
    await leteo(pi as any)
    const learnings = "## Key Learnings:\n1. Shutdown must not outrun a capture that has already started"
    pi.handlers.get("tool_execution_end")!(
      { toolName: "subagent", result: { content: [{ type: "text", text: learnings }] } },
      context(),
    )
    await pi.handlers.get("session_shutdown")!({ type: "session_shutdown", reason: "quit" }, context())
    assert.deepEqual(
      h.log().map((entry) => entry.event),
      ["subagent-stop", "session-stop"],
    )
  } finally {
    h.cleanup()
  }
})

test("a tool result is sent as its text, because JSON escapes the newlines a learnings heading needs", () => {
  assert.equal(resultText("plain"), "plain")
  assert.equal(
    resultText({ content: [{ type: "text", text: "a\nb" }, { type: "image" }, { type: "text", text: "c" }] }),
    "a\nb\nc",
  )
  assert.equal(resultText({ other: 1 }), '{"other":1}')
  assert.equal(resultText(undefined), "")
})

test("a summary is filed under its first line that is not a heading", () => {
  assert.equal(summaryTitle("## Goal\n\nShip the Pi package\n\n## Done\n- x"), "Ship the Pi package")
  assert.equal(summaryTitle("## Only a heading"), "Pi compaction summary")
  assert.equal(summaryTitle("x".repeat(300)).length, 100)
})

test("a compaction saves the summary in the session's directory, recovers context, and hands it over next turn", async () => {
  const h = harness()
  try {
    const pi = fakePi()
    await leteo(pi as any)
    await pi.handlers.get("session_compact")!(
      { type: "session_compact", compactionEntry: { summary: "## Goal\nShip the Pi package" } },
      context("pi-session-1", HERE),
    )
    const [saved] = h.log().filter((entry) => entry.kind === "save")
    assert.deepEqual(saved.args, [
      "--type",
      "session_summary",
      "--session",
      "pi-session-1",
      "--",
      "Ship the Pi package",
      "## Goal\nShip the Pi package",
    ])
    assert.equal(realpathSync(saved.cwd), realpathSync(HERE), "it runs where the session does")
    assert.deepEqual(
      h.log().map((entry) => entry.kind === "save" ? "save" : entry.event),
      ["save", "post-compaction"],
      "the summary is saved before the context is recovered",
    )

    const options = { appendSystemPrompt: "" }
    await pi.handlers.get("before_agent_start")!(
      { type: "before_agent_start", prompt: "continue the work", systemPrompt: "", systemPromptOptions: options },
      context(),
    )
    assert.ok(options.appendSystemPrompt.includes("RECOVERED CONTEXT"))
  } finally {
    h.cleanup()
  }
})

test("shutdown ends the session unless Pi is reloading", async () => {
  const h = harness()
  try {
    const pi = fakePi()
    await leteo(pi as any)
    const shutdown = pi.handlers.get("session_shutdown")!
    await shutdown({ type: "session_shutdown", reason: "reload" }, context())
    assert.equal(hooks(h, "session-stop").length, 0, "a reload keeps the session running")

    for (const reason of ["quit", "new", "resume", "fork"]) {
      await shutdown({ type: "session_shutdown", reason }, context(`s-${reason}`))
    }
    assert.deepEqual(
      hooks(h, "session-stop").map((entry) => entry.payload.session_id),
      ["s-quit", "s-new", "s-resume", "s-fork"],
    )
  } finally {
    h.cleanup()
  }
})

test("a child that stops reading does not crash the hook path", async () => {
  // A child that exits without reading closes the pipe under a payload larger
  // than the pipe buffer, and the write then emits EPIPE on the stdin stream.
  // Without the listener Node rethrows it as an uncaught exception, which the
  // test runner reports as this test failing.
  const directory = mkdtempSync(join(tmpdir(), "leteo-pi-deaf-"))
  const deaf = join(directory, "leteo")
  writeFileSync(deaf, `#!${process.execPath}\nprocess.exit(0)\n`)
  chmodSync(deaf, 0o755)
  const h = harness({ LETEO_BIN: deaf })
  try {
    const { ok, output } = await attemptHook("subagent-stop", {
      session_id: "s1",
      cwd: HERE,
      stdout: "x".repeat(4 << 20),
    })
    assert.deepEqual(output, {})
    assert.equal(ok, true, "it exited zero, having read nothing")
  } finally {
    h.cleanup()
  }
})

test("a hook that did not run is told apart from one with nothing to add", async () => {
  const h = harness()
  try {
    assert.deepEqual(await attemptHook("session-stop", { session_id: "s" }), { ok: true, output: {} })
  } finally {
    h.cleanup()
  }
  const failing = harness({ LETEO_FAKE_HOOK_EXIT: "3" })
  try {
    assert.equal((await attemptHook("session-stop", { session_id: "s" })).ok, false, "non-zero exit")
  } finally {
    failing.cleanup()
  }
  const missing = harness({ LETEO_BIN: "/nonexistent/leteo" })
  try {
    assert.equal((await attemptHook("session-stop", { session_id: "s" })).ok, false, "no such binary")
  } finally {
    missing.cleanup()
  }
})

test("a summary that starts like an option is still a summary", async () => {
  const h = harness()
  try {
    const pi = fakePi()
    await leteo(pi as any)
    const summary = "--project=other\n- fixed the bug"
    await pi.handlers.get("session_compact")!(
      { type: "session_compact", compactionEntry: { summary } },
      context("pi-session-1", HERE),
    )
    const [saved] = h.log().filter((entry) => entry.kind === "save")
    const separator = saved.args.indexOf("--")
    assert.deepEqual(saved.args.slice(separator + 1), ["--project=other", summary])
    assert.ok(
      saved.args.slice(0, separator).every((argument: string) => !argument.startsWith("--project")),
      "nothing the model wrote is before the separator",
    )
  } finally {
    h.cleanup()
  }
})

test("a summary that could not be saved is not reported as saved", async () => {
  const notices: string[] = []
  const h = harness({ LETEO_FAKE_SAVE_EXIT: "1" })
  try {
    const pi = fakePi()
    await leteo(pi as any)
    const failing = { ...context("pi-session-1", HERE), ui: { notify: (text: string) => notices.push(text) } }
    await pi.handlers.get("session_compact")!(
      { type: "session_compact", compactionEntry: { summary: "Goal: ship" } },
      failing,
    )
    assert.equal(notices.length, 1)
    assert.match(notices[0], /could not store the summary/)

    const options = { appendSystemPrompt: "" }
    await pi.handlers.get("before_agent_start")!(
      { type: "before_agent_start", prompt: "continue the work", systemPrompt: "", systemPromptOptions: options },
      context(),
    )
    assert.match(options.appendSystemPrompt, /could not store the summary/, "the model is told too")
    assert.ok(options.appendSystemPrompt.includes("RECOVERED CONTEXT"), "the recovery still follows")
  } finally {
    h.cleanup()
  }
})

test("nothing in the extension redacts or truncates before the binary sees the text", () => {
  const source = readFileSync(new URL("./leteo.ts", import.meta.url), "utf8")
  const code = source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/\/\/[^\n]*/g, "")
  assert.ok(!/<private>|substring|truncate/.test(code), "redaction and bounds are the binary's")
})

test("the binary is LETEO_BIN, then the one setup recorded, then leteo on PATH", async () => {
  const previous = process.env.LETEO_BIN
  try {
    delete process.env.LETEO_BIN
    assert.equal(leteoBinary(), "leteo", "installed from npm, nothing is recorded and PATH decides")

    // Setup rewrites one line of the source; a copy with that line rewritten is
    // what an installed extension is.
    const source = readFileSync(new URL("./leteo.ts", import.meta.url), "utf8")
    assert.ok(source.includes('const INSTALLED_BINARY = ""'))
    const copy = join(mkdtempSync(join(tmpdir(), "leteo-pi-installed-")), "leteo.ts")
    writeFileSync(copy, source.replace('const INSTALLED_BINARY = ""', 'const INSTALLED_BINARY = "/opt/leteo/bin/leteo"'))
    const installed = await import(pathToFileURL(copy).href)
    assert.equal(installed.leteoBinary(), "/opt/leteo/bin/leteo")

    process.env.LETEO_BIN = "/override/leteo"
    assert.equal(installed.leteoBinary(), "/override/leteo", "the environment wins over what setup recorded")
    assert.equal(leteoBinary(), "/override/leteo")
  } finally {
    if (previous === undefined) delete process.env.LETEO_BIN
    else process.env.LETEO_BIN = previous
  }
})
