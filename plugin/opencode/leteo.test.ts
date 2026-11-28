// Contract tests for the OpenCode plugin.
//
// OpenCode 1.x loads the `server` factory; 2.x loads `setup`. Those two shapes
// are what these tests hold the plugin to, because the failure this file exists
// to catch — a plugin that silently does not load on 2.x — is exactly a shape
// mismatch, and nothing else in the repository exercises either contract.
//
// Run with: `node --test leteo.test.ts` from this directory.
import { test } from "node:test"
import assert from "node:assert/strict"
import { chmodSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { fileURLToPath } from "node:url"

import plugin, { firstOfCompaction, Leteo, runHook, V2_COMPACTION_WINDOW_MS } from "./leteo.ts"

const HERE = fileURLToPath(new URL(".", import.meta.url))

test("the default export serves both majors", () => {
  assert.equal((plugin as any).id, "leteo")
  assert.equal((plugin as any).server, Leteo)
  assert.equal(typeof (plugin as any).setup, "function")
})

test("the 1.x factory binds the four hooks 1.x calls", async () => {
  const hooks = (await Leteo({ directory: HERE } as any)) as Record<string, unknown>
  for (const name of [
    "event",
    "chat.message",
    "tool.execute.after",
    "experimental.chat.system.transform",
  ]) {
    assert.equal(typeof hooks[name], "function", `${name} is bound`)
  }
})

test("the 2.x setup registers its hooks and subscribes to events", async () => {
  const registered: string[] = []
  let subscribed = false
  let released = 0
  const ctx = {
    location: { directory: HERE },
    event: {
      subscribe: () => {
        subscribed = true
        return (async function* () {})()
      },
    },
    session: {
      hook: async (name: string) => {
        registered.push(`session:${name}`)
        return {
          dispose: async () => {
            released++
          },
        }
      },
    },
    tool: {
      hook: async (name: string) => {
        registered.push(`tool:${name}`)
        return {
          dispose: async () => {
            released++
          },
        }
      },
    },
  }
  const cleanup = await (plugin as any).setup(ctx)
  // The loop reconnects forever; cleanup must run even when an assertion fails,
  // or a regression shows up as a hung suite instead of a red one.
  try {
    assert.deepEqual(registered.sort(), ["session:context", "tool:execute.after"])
    assert.ok(subscribed, "the event stream is subscribed")
  } finally {
    await cleanup()
  }
  assert.equal(released, 2, "cleanup disposes both registrations")
})

test("a child that stops reading does not crash the hook path", async () => {
  const previous = process.env.LETEO_BIN
  // A child that closes stdin immediately, with a payload larger than the pipe
  // buffer: the write emits EPIPE, and without a listener on the stream Node
  // rethrows it and this process exits instead of the hook being swallowed.
  process.env.LETEO_BIN = "/usr/bin/true"
  try {
    const output = await runHook("subagent-stop", {
      session_id: "s1",
      cwd: HERE,
      stdout: "x".repeat(1 << 20),
    } as any)
    assert.deepEqual(output, {})
  } finally {
    if (previous === undefined) delete process.env.LETEO_BIN
    else process.env.LETEO_BIN = previous
  }
})

test("no Bun-only API on the hook path", () => {
  const source = readFileSync(new URL("./leteo.ts", import.meta.url), "utf8")
  // Comments may name Bun to explain why it is gone; only code counts.
  const code = source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/\/\/[^\n]*/g, "")
  assert.ok(!/\bBun\b/.test(code), "the plugin must not use Bun")
  assert.ok(code.includes("node:child_process"), "the hook path spawns through Node")
})

// A stand-in for the `leteo` binary that records every hook it is asked to run
// and answers the way the real one does: a session opening without its
// directive when asked, and the compaction text only for `post-compaction`.
const FAKE_LETEO = `#!/usr/bin/env node
const fs = require("node:fs")
let body = ""
process.stdin.setEncoding("utf8")
process.stdin.on("data", (chunk) => (body += chunk))
process.stdin.on("end", () => {
  const event = process.argv[3]
  const payload = JSON.parse(body || "{}")
  fs.appendFileSync(process.env.LETEO_FAKE_LOG, JSON.stringify({ event, payload }) + "\\n")
  const context = {
    "session-start": payload.without_directive ? "MEMORY BLOCK" : "DIRECTIVE AND MEMORY BLOCK",
    "post-compaction": "Context was compacted. MEMORY BLOCK",
    "user-prompt-submit": (payload.prompt || "").includes("storage") ? "RECALL HINT" : "",
  }[event]
  process.stdout.write(context ? JSON.stringify({ hookSpecificOutput: { additionalContext: context } }) : "{}")
})
`

/** Points the plugin at the fake binary with a fresh log, and undoes it. */
async function withFakeLeteo(run: (calls: () => any[]) => Promise<void>): Promise<void> {
  const directory = mkdtempSync(join(tmpdir(), "leteo-opencode-"))
  const binary = join(directory, "leteo")
  const log = join(directory, "calls.log")
  writeFileSync(binary, FAKE_LETEO)
  chmodSync(binary, 0o755)
  writeFileSync(log, "")
  const previous = { bin: process.env.LETEO_BIN, log: process.env.LETEO_FAKE_LOG }
  process.env.LETEO_BIN = binary
  process.env.LETEO_FAKE_LOG = log
  try {
    await run(() =>
      readFileSync(log, "utf8")
        .split("\n")
        .filter(Boolean)
        .map((line) => JSON.parse(line)),
    )
  } finally {
    for (const [key, value] of [
      ["LETEO_BIN", previous.bin],
      ["LETEO_FAKE_LOG", previous.log],
    ] as const) {
      if (value === undefined) delete process.env[key]
      else process.env[key] = value
    }
  }
}

test("a fresh session is opened with the memory alone, not a compaction notice", async () => {
  await withFakeLeteo(async (calls) => {
    const hooks = (await Leteo({ directory: HERE } as any)) as Record<string, any>
    await hooks.event({ event: { type: "session.created", properties: { info: { id: "s1" } } } })

    const system = ["base"]
    await hooks["experimental.chat.system.transform"]({ sessionID: "s1" }, { system })

    const text = system.join("\n")
    assert.ok(text.includes("MEMORY BLOCK"), "the memory reaches the agent")
    assert.ok(!text.includes("Context was compacted"), "a fresh session was not compacted")
    assert.ok(!text.includes("DIRECTIVE"), "the directive is not said on top of the protocol")
    assert.equal(text.split("## Leteo Persistent Memory").length, 2, "the protocol appears once")
    assert.deepEqual(
      calls().map((call) => call.event),
      ["session-start"],
      "no compaction hook runs for a session nobody compacted",
    )

    const again = ["base"]
    await hooks["experimental.chat.system.transform"]({ sessionID: "s1" }, { system: again })
    assert.ok(!again.join("\n").includes("MEMORY BLOCK"), "the opening is handed over once")
  })
})

test("what the per-prompt hook named reaches the model on every step of that turn only", async () => {
  await withFakeLeteo(async (calls) => {
    const hooks = (await Leteo({ directory: HERE } as any)) as Record<string, any>
    await hooks.event({ event: { type: "session.created", properties: { info: { id: "s1" } } } })
    const prompt = async (text: string) =>
      hooks["chat.message"](
        { sessionID: "s1" },
        { parts: [{ type: "text", text }], message: {} },
      )
    const system = async () => {
      const out = ["base"]
      await hooks["experimental.chat.system.transform"]({ sessionID: "s1" }, { system: out })
      return out.join("\n")
    }

    await prompt("Which storage engine did we choose?")
    assert.ok((await system()).includes("RECALL HINT"), "the hint the hook recorded as shown is shown")
    assert.ok((await system()).includes("RECALL HINT"), "and on the next model call of the same turn")

    await prompt("Something else entirely, please")
    assert.ok(!(await system()).includes("RECALL HINT"), "a later prompt with no hint drops the old one")
    assert.equal(calls().filter((call) => call.event === "user-prompt-submit").length, 2)

    await prompt("Which storage engine did we choose?")
    assert.ok((await system()).includes("RECALL HINT"))
    await prompt("ok")
    assert.ok(!(await system()).includes("RECALL HINT"), "a prompt too short for the hook drops the old hint")
    assert.equal(calls().filter((call) => call.event === "user-prompt-submit").length, 3)
  })
})

test("the hook is told the hint and the opening block both last one request", async () => {
  await withFakeLeteo(async (calls) => {
    const hooks = (await Leteo({ directory: HERE } as any)) as Record<string, any>
    await hooks.event({ event: { type: "session.created", properties: { info: { id: "s1" } } } })
    await hooks["chat.message"](
      { sessionID: "s1" },
      { parts: [{ type: "text", text: "Which storage engine did we choose?" }], message: {} },
    )
    const [prompt] = calls().filter((call) => call.event === "user-prompt-submit")
    assert.equal(prompt.payload.transient_hints, true)
    const [opening] = calls().filter((call) => call.event === "session-start")
    assert.equal(
      opening.payload.transient_hints,
      true,
      "the block reaches one model request, so the hook must not record it as read",
    )
  })
})

test("a compaction announced by the 2.x session.next.compaction.ended runs the hook once, even beside session.compacted", async () => {
  await withFakeLeteo(async (calls) => {
    const events = [
      { type: "session.created", data: { sessionID: "s3" } },
      { type: "session.next.compaction.ended", data: { sessionID: "s3", timestamp: 1 } },
      { type: "session.compacted", data: { sessionID: "s3" } },
      { type: "session.next.compaction.ended", data: { sessionID: "s5", timestamp: 2 } },
      { type: "session.next.compaction.ended", data: { sessionID: "s4" }, location: { directory: "/elsewhere" } },
      { type: "session.deleted", data: { sessionID: "s3" } },
    ]
    let delivered = false
    const ctx = {
      location: { directory: HERE },
      event: {
        subscribe: () =>
          (async function* () {
            if (delivered) return
            delivered = true
            yield* events
          })(),
      },
      session: { hook: async () => ({ dispose: async () => {} }) },
      tool: { hook: async () => ({ dispose: async () => {} }) },
    }
    const cleanup = await (plugin as any).setup(ctx)
    try {
      for (let wait = 0; wait < 100 && !calls().some((call) => call.event === "session-stop"); wait++) {
        await new Promise((resolve) => setTimeout(resolve, 50))
      }
      assert.deepEqual(
        calls()
          .filter((call) => call.event === "post-compaction")
          .map((call) => call.payload.session_id),
        ["s3", "s5"],
        "s3 announced its compaction twice and s5 only through the new event",
      )
    } finally {
      await cleanup()
    }
  })
})

test("two compaction events of one session close together are one compaction, and a later one is another", () => {
  const seen = new Map<string, number>()
  assert.equal(firstOfCompaction(seen, "s1", 1_000), true)
  assert.equal(firstOfCompaction(seen, "s1", 1_000 + V2_COMPACTION_WINDOW_MS - 1), false)
  assert.equal(firstOfCompaction(seen, "s2", 1_001), true, "another session is its own")
  assert.equal(firstOfCompaction(seen, "s1", 1_000 + V2_COMPACTION_WINDOW_MS), true)
})

test("a compaction runs the post-compaction hook and hands its context over on the next turn only", async () => {
  await withFakeLeteo(async (calls) => {
    const hooks = (await Leteo({ directory: HERE } as any)) as Record<string, any>
    await hooks.event({ event: { type: "session.created", properties: { info: { id: "s1" } } } })
    const system = async () => {
      const out = ["base"]
      await hooks["experimental.chat.system.transform"]({ sessionID: "s1" }, { system: out })
      return out.join("\n")
    }
    assert.ok((await system()).includes("MEMORY BLOCK"), "the opening is handed over before the compaction")

    await hooks.event({ event: { type: "session.compacted", properties: { sessionID: "s1" } } })
    const compactions = calls().filter((call) => call.event === "post-compaction")
    assert.equal(compactions.length, 1)
    assert.deepEqual(compactions[0].payload, {
      session_id: "s1",
      cwd: HERE,
      transient_hints: true,
    })

    assert.ok(
      (await system()).includes("Context was compacted. MEMORY BLOCK"),
      "what the agent lost is put back in front of it",
    )
    assert.ok(!(await system()).includes("MEMORY BLOCK"), "and only once")
  })
})

test("a compaction of a sub-agent session is not a memory event", async () => {
  await withFakeLeteo(async (calls) => {
    const hooks = (await Leteo({ directory: HERE } as any)) as Record<string, any>
    await hooks.event({
      event: { type: "session.created", properties: { info: { id: "child", parentID: "s1" } } },
    })
    await hooks.event({ event: { type: "session.compacted", properties: { sessionID: "child" } } })
    assert.deepEqual(calls(), [])
  })
})

test("the 2.x adapter turns its session.compacted event into the same compaction", async () => {
  await withFakeLeteo(async (calls) => {
    const events = [
      { type: "session.created", data: { sessionID: "s2" } },
      { type: "session.compacted", data: { sessionID: "s2" } },
      { type: "session.compacted", data: { sessionID: "foreign" }, location: { directory: "/elsewhere" } },
      // Events are handled in order, so this one's hook having run means the
      // compactions before it have been judged.
      { type: "session.deleted", data: { sessionID: "s2" } },
    ]
    let delivered = false
    const ctx = {
      location: { directory: HERE },
      event: {
        subscribe: () =>
          (async function* () {
            if (delivered) return
            delivered = true
            yield* events
          })(),
      },
      session: { hook: async () => ({ dispose: async () => {} }) },
      tool: { hook: async () => ({ dispose: async () => {} }) },
    }
    const cleanup = await (plugin as any).setup(ctx)
    try {
      for (let wait = 0; wait < 100 && !calls().some((call) => call.event === "session-stop"); wait++) {
        await new Promise((resolve) => setTimeout(resolve, 50))
      }
      const compactions = calls().filter((call) => call.event === "post-compaction")
      assert.deepEqual(
        compactions.map((call) => call.payload.session_id),
        ["s2"],
        "another project's compaction is not this plugin's",
      )
    } finally {
      await cleanup()
    }
  })
})
