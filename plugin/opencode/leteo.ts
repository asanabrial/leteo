/**
 * Leteo — OpenCode plugin
 *
 * Maps OpenCode's lifecycle events onto `leteo hook <event>`, and injects the
 * memory protocol into every system prompt so the agent keeps using its tools
 * after a compaction.
 *
 * Why this is thin: every decision lives in the Leteo binary. Project
 * detection, the stable manual session, `<private>` redaction, deduplication,
 * the save reminder and its debounce, and the `.leteo/` import are all
 * implemented once, in Rust, and shared with the CLI, the MCP server, and the
 * Claude Code hooks. This file only translates event shapes.
 *
 * It also needs no HTTP server and no port: `leteo hook` talks to SQLite
 * directly, so there is nothing to start, nothing to health-check, and nothing
 * listening on the machine.
 */

import { spawn } from "node:child_process"
import type { Plugin } from "@opencode-ai/plugin"

/** Binary to invoke. Set LETEO_BIN when it is not on PATH. */
function leteoBinary(): string {
  return process.env.LETEO_BIN ?? "leteo"
}

/** Leteo's own MCP tools never count as project work. */
const LETEO_TOOLS = new Set([
  "mem_capture_passive",
  "mem_compare",
  "mem_context",
  "mem_current_project",
  "mem_delete",
  "mem_doctor",
  "mem_get_observation",
  "mem_judge",
  "mem_merge_projects",
  "mem_pin",
  "mem_review",
  "mem_save",
  "mem_save_prompt",
  "mem_search",
  "mem_session_end",
  "mem_session_start",
  "mem_session_summary",
  "mem_stats",
  "mem_suggest_topic_key",
  "mem_timeline",
  "mem_unpin",
  "mem_update",
])

/** Prompts shorter than this are not worth persisting. */
const MIN_PROMPT_LENGTH = 10
/** Tool output shorter than this cannot hold a learnings section. */
const MIN_CAPTURE_LENGTH = 50
/** A hook must never delay the user's turn; it is killed past this. */
const HOOK_TIMEOUT_MS = 5000

const MEMORY_PROTOCOL = `## Leteo Persistent Memory — Protocol

Leteo is persistent memory. Its tools survive sessions and context compaction.

### Save important work

Call \`mem_save\` immediately after completing a bug fix, making an architecture
or design decision, discovering a non-obvious constraint, changing
configuration, or establishing a reusable convention. Use a short searchable
title and structure the content as What, Why, Where, and Learned. Reuse a
\`topic_key\` to revise an evolving decision instead of inserting a near
duplicate; call \`mem_suggest_topic_key\` when unsure.

### Recall before acting

When prior work may be relevant, call \`mem_search\` with the words of the thing you are asking
about, then \`mem_get_observation\` when the complete observation is needed.

\`mem_context\` answers a different question — everything a project holds — and it is what a session
opens with where Leteo installs hooks. Measured on a real store, that opening block named thirty
memories as titles in 3.4 KB and \`mem_context\` came back with the same thirty, previews included,
in 8.4 KB. So call it when the session did not open with one, when you have lost it, or for another
project; not as the first step of looking something up.

### Projects

Every response says which project it used and why, in \`project\` and
\`project_source\`. If a write fails with \`ambiguous_project\`, ask the user
which project it belongs to, then retry with their choice plus
\`project_choice_reason=user_selected_after_ambiguous_project\` and the
\`recovery_token\` from that error. Never guess.

### Close sessions

Before saying the work is done, call \`mem_session_summary\` with the goal,
discoveries, accomplishments, next steps, and relevant files. After context compaction,
persist the compacted summary first and then restore context with \`mem_context\`.`

/** One hook event's payload, matching what the Rust side deserializes. */
interface HookInput {
  session_id?: string
  cwd?: string
  prompt?: string
  stdout?: string
  source?: string
  /** Leave the memory directive out of a `session-start` answer. */
  without_directive?: boolean
  /**
   * What the call answers lives in one turn's system prompt and is gone from the
   * next, so the hook must not record it as delivered.
   */
  transient_hints?: boolean
}

/** The response `leteo hook` prints. */
interface HookOutput {
  hookSpecificOutput?: { hookEventName?: string; additionalContext?: string }
  systemMessage?: string
}

type HookEvent =
  | "session-start"
  | "post-compaction"
  | "user-prompt-submit"
  | "subagent-stop"
  | "session-stop"

/**
 * Runs one hook. Failures are swallowed on purpose: memory is an assistant, not
 * a gate, and a missing binary must never break the user's session.
 *
 * `node:child_process` rather than `Bun.spawn`, because OpenCode 2.x loads
 * plugins on Node, where `Bun` is not defined — and a hook that throws on the
 * way to spawning is a hook that never runs. Node's `spawn` exists under both
 * runtimes, so this is the one API that works wherever the plugin is loaded.
 *
 * Exported for the contract test, which drives it with a binary that stops
 * reading to prove the stdin error handler below holds.
 */
export function runHook(event: HookEvent, input: HookInput): Promise<HookOutput> {
  return new Promise((resolve) => {
    let settled = false
    let timer: ReturnType<typeof setTimeout> | undefined
    const finish = (output: HookOutput) => {
      if (settled) return
      settled = true
      if (timer) clearTimeout(timer)
      resolve(output)
    }

    let child
    try {
      child = spawn(leteoBinary(), ["hook", event], { stdio: ["pipe", "pipe", "ignore"] })
    } catch {
      resolve({})
      return
    }

    let stdout = ""
    child.stdout?.setEncoding("utf8")
    child.stdout?.on("data", (chunk: string) => {
      stdout += chunk
    })
    child.on("error", () => finish({}))
    child.on("close", () => {
      try {
        finish(stdout.trim() ? (JSON.parse(stdout) as HookOutput) : {})
      } catch {
        finish({})
      }
    })
    // A child that stops reading before the payload drains (a binary that
    // rejects the subcommand, or one killed at the timeout below) closes its
    // read end, and the write then emits `EPIPE`. Without a listener Node
    // rethrows that as an uncaught exception and takes the host down — which is
    // exactly what this function promises not to do. The synchronous try/catch
    // around `end` cannot see it: stream errors arrive on a later tick. The
    // close event still resolves with whatever the child printed.
    child.stdin?.on("error", () => {})
    // A hook must never delay the user's turn; it is killed past this. The kill
    // closes the child, which resolves with whatever it managed to print.
    timer = setTimeout(() => {
      try {
        child.kill()
      } catch {}
    }, HOOK_TIMEOUT_MS)
    try {
      child.stdin?.end(JSON.stringify(input))
    } catch {}
  })
}

function additionalContext(output: HookOutput): string {
  return output.hookSpecificOutput?.additionalContext?.trim() ?? ""
}

export const Leteo: Plugin = async (ctx) => {
  const directory = ctx.directory

  // Sub-agent sessions must not become top-level memory sessions: a single
  // conversation can spawn dozens and they would drown the real one.
  const subAgentSessions = new Set<string>()
  const startedSessions = new Set<string>()
  const recoveredSessions = new Set<string>()
  /**
   * What `session-start` returned for a session, or `post-compaction` after one,
   * until the next system prompt takes it.
   */
  const openings = new Map<string, string>()
  /**
   * What the per-prompt hook named for the turn in progress, by session.
   *
   * `chat.message` cannot carry it: adding a part there would put the hint in
   * the user's transcript as something they said. The system prompt is rebuilt
   * for every model call of a turn, so the hint is kept until the next prompt
   * replaces it and every step of the turn sees it, and not a turn longer. The
   * hook is told so, because a memory it recorded as shown would be withheld
   * from a later turn that no longer holds it.
   */
  const hints = new Map<string, string>()

  /**
   * Starts a memory session on demand, so a plugin loaded mid-conversation
   * still records the rest of it. The Rust side is idempotent, and this is
   * where the project migration and the `.leteo/` import happen.
   */
  async function ensureSession(sessionID: string): Promise<void> {
    if (!sessionID || subAgentSessions.has(sessionID)) return
    if (startedSessions.has(sessionID)) return
    startedSessions.add(sessionID)
    // Without the directive, because this plugin injects the protocol on every
    // turn and the opening would say it twice. What is left is the language,
    // the memory, and any verdicts waiting, which is what a fresh session needs.
    const output = await runHook("session-start", {
      session_id: sessionID,
      cwd: directory,
      without_directive: true,
    })
    const opening = additionalContext(output)
    if (opening) openings.set(sessionID, opening)
  }

  /**
   * Returns this project's prior memory once per session.
   *
   * It is the opening `ensureSession` already asked for, or what the
   * `session.compacted` handler recovered. `post-compaction` used to be called
   * here for being the hook that returns the memory without the protocol, and
   * its text begins "Context was compacted", which a session that had only just
   * been created was told; it now runs only after OpenCode says it compacted.
   */
  async function recoverContext(sessionID: string): Promise<string> {
    if (!sessionID || subAgentSessions.has(sessionID)) return ""
    if (recoveredSessions.has(sessionID)) return ""
    recoveredSessions.add(sessionID)
    const opening = openings.get(sessionID) ?? ""
    openings.delete(sessionID)
    return opening
  }

  return {
    event: async ({ event }) => {
      if (event.type === "session.created") {
        const info = (event.properties as any)?.info
        const sessionID: string | undefined = info?.id
        if (!sessionID) return
        // A sub-agent always carries a parent; the title suffix is a fallback
        // for versions that do not set one.
        const isSubAgent =
          Boolean(info?.parentID) || String(info?.title ?? "").endsWith(" subagent)")
        if (isSubAgent) {
          subAgentSessions.add(sessionID)
          return
        }
        await ensureSession(sessionID)
      }

      // OpenCode announces a compaction after it has happened, and the answer is
      // held for the next system prompt rather than injected here: this event
      // has nowhere to put text in front of the agent. The hook also clears the
      // list of memories it has named, which is why it has to run even when
      // the answer turns out empty.
      if (event.type === "session.compacted") {
        const sessionID: string | undefined = (event.properties as any)?.sessionID
        if (!sessionID || subAgentSessions.has(sessionID)) return
        await ensureSession(sessionID)
        const recovered = additionalContext(
          await runHook("post-compaction", { session_id: sessionID, cwd: directory }),
        )
        if (recovered) {
          openings.set(sessionID, recovered)
          recoveredSessions.delete(sessionID)
        }
      }

      if (event.type === "session.deleted") {
        const sessionID: string | undefined = (event.properties as any)?.info?.id
        if (!sessionID) return
        if (!subAgentSessions.has(sessionID)) {
          await runHook("session-stop", { session_id: sessionID, cwd: directory })
        }
        startedSessions.delete(sessionID)
        subAgentSessions.delete(sessionID)
        recoveredSessions.delete(sessionID)
        openings.delete(sessionID)
        hints.delete(sessionID)
      }
    },

    // Every user message is persisted so a memory can cite what prompted it.
    "chat.message": async (input, output) => {
      const sessionID = input.sessionID
      if (!sessionID || subAgentSessions.has(sessionID)) return

      const text = output.parts
        .filter((part) => part.type === "text")
        .map((part) => (part as any).text ?? "")
        .join("\n")
        .trim()
      const summary = output.message.summary
      const content =
        text ||
        (summary ? `${summary.title ?? ""}\n${summary.body ?? ""}`.trim() : "")
      // Dropped before the length check: a hint belongs to the turn the hook
      // evaluated, and a short prompt the hook never sees would otherwise
      // carry the previous turn's hint into its own.
      hints.delete(sessionID)
      if (content.length <= MIN_PROMPT_LENGTH) return

      await ensureSession(sessionID)
      const hint = additionalContext(
        await runHook("user-prompt-submit", {
          session_id: sessionID,
          cwd: directory,
          prompt: content,
          transient_hints: true,
        }),
      )
      if (hint) hints.set(sessionID, hint)
    },

    // A finished sub-agent usually reports what it learned; capture it.
    "tool.execute.after": async (input, output) => {
      const sessionID = input.sessionID
      if (!sessionID || subAgentSessions.has(sessionID)) return
      if (LETEO_TOOLS.has(input.tool.toLowerCase())) return
      if (input.tool !== "Task") return

      const text = typeof output === "string" ? output : JSON.stringify(output)
      if (text.length < MIN_CAPTURE_LENGTH) return
      await ensureSession(sessionID)
      await runHook("subagent-stop", {
        session_id: sessionID,
        cwd: directory,
        stdout: text,
        source: "opencode-task",
      })
    },

    // The protocol is re-injected on every message, which is what makes memory
    // survive a compaction: the agent is told again how to use it.
    //
    // It is appended to the last system entry instead of pushed as a new one.
    // Several local models reject more than one system block.
    "experimental.chat.system.transform": async (input, output) => {
      let block = MEMORY_PROTOCOL

      const sessionID: string = (input as any)?.sessionID ?? ""
      if (sessionID && !subAgentSessions.has(sessionID)) {
        await ensureSession(sessionID)
        const recovered = await recoverContext(sessionID)
        if (recovered) {
          block += `\n\n${recovered}`
        }
        const hint = hints.get(sessionID)
        if (hint) block += `\n\n${hint}`
      }

      if (output.system.length > 0) {
        output.system[output.system.length - 1] += `\n\n${block}`
      } else {
        output.system.push(block)
      }
    },
  }
}

// ─── OpenCode 2.x adapter ────────────────────────────────────────────────────
//
// OpenCode 1.x calls the `Leteo` factory above; 2.x calls `setup`. The adapter
// binds the same four handlers onto 2.x's per-domain registrations and its
// event stream, translating 2.x's event, prompt and tool shapes into the ones
// the V1 handlers already read. Its types are structural because a 2.x host
// does not ship `@opencode-ai/plugin`, so importing the V1 `Plugin` type is not
// enough to describe what 2.x hands over.

type V2SystemPart = { type: "text"; text: string }
type V2Registration = { dispose: () => Promise<void> }
type V2Hook = (
  name: string,
  callback: (input: any) => Promise<void> | void,
) => Promise<V2Registration>
type V2Context = {
  location: { directory: string }
  event: { subscribe: (options?: { signal?: AbortSignal }) => AsyncIterable<any> }
  session: { hook: V2Hook }
  tool: { hook: V2Hook }
}

async function withSystemStrings(
  system: V2SystemPart[],
  run: (texts: string[]) => Promise<void>,
): Promise<void> {
  const texts = system.map((part) => part.text)
  await run(texts)
  texts.forEach((text, index) => {
    if (index >= system.length) system.push({ type: "text", text })
    else if (text !== system[index].text) system[index] = { ...system[index], text }
  })
}

/** 2.x `Tool.Result.content` is `string | Content[]`; `subagent` returns a string. */
function v2ToolResultText(result: any): string {
  const text =
    typeof result?.content === "string"
      ? result.content
      : Array.isArray(result?.content)
        ? result.content
            .filter((part: any) => part?.type === "text")
            .map((part: any) => part.text ?? "")
            .join("\n")
        : ""
  if (text) return text
  if (typeof result?.output === "string") return result.output
  return result?.output === undefined ? "" : JSON.stringify(result.output)
}

/**
 * The 2.x server is shared across locations, so an event may belong to another
 * project's instance while the V1 plugin only ever saw its own. The location is
 * read from both the envelope and the payload, because the two event kinds put
 * it in different places, and a foreign event is rejected whichever carries it.
 */
function isForeign(event: any, directory: string): boolean {
  const data = event?.data
  return [event?.location?.directory, data?.location?.directory].some(
    (location) => typeof location === "string" && location !== directory,
  )
}

/**
 * What 2.x emits when a compaction finishes. A host may emit it without the
 * `session.compacted` the 1.x handler listens to, and one that emits both
 * announces one compaction twice.
 */
const V2_COMPACTION_ENDED = "session.next.compaction.ended"

/** Two compaction events of one session this close are the same compaction. */
export const V2_COMPACTION_WINDOW_MS = 10_000

/**
 * Whether an event announcing a compaction is the first of its compaction, so
 * that one compaction runs `post-compaction` once and not once per event name.
 * Kept by session and arrival time: `session.compacted` carries no timestamp of
 * its own to compare with the other event's.
 */
export function firstOfCompaction(
  seen: Map<string, number>,
  sessionID: string,
  now: number = Date.now(),
): boolean {
  const last = seen.get(sessionID)
  if (last !== undefined && now - last < V2_COMPACTION_WINDOW_MS) return false
  seen.set(sessionID, now)
  return true
}

/**
 * 2.x session events carry `data.sessionID`; the V1 handler reads
 * `properties.info.id`.
 */
function v1SessionEvent(event: any, directory: string): any {
  const data = event?.data
  if (typeof data?.sessionID !== "string") return undefined
  if (
    event.type !== "session.created" &&
    event.type !== "session.updated" &&
    event.type !== "session.deleted" &&
    event.type !== "session.compacted" &&
    event.type !== V2_COMPACTION_ENDED
  ) {
    return undefined
  }
  if (isForeign(event, directory)) return undefined
  // 1.x carries the id of a compacted session in `properties.sessionID`, not in
  // an `info` object like the lifecycle events.
  if (event.type === "session.compacted" || event.type === V2_COMPACTION_ENDED) {
    return { type: "session.compacted", properties: { sessionID: data.sessionID } }
  }
  if (event.type === "session.deleted") {
    return { type: event.type, properties: { info: { id: data.sessionID } } }
  }
  return {
    type: event.type,
    properties: { info: { id: data.sessionID, parentID: data.parentID } },
  }
}

/** 2.x admits each human prompt as a durable `user` inbox item. */
function v2InboxPrompt(
  event: any,
  directory: string,
): { sessionID: string; text: string } | undefined {
  if (event?.type !== "session.inbox.enqueued") return undefined
  if (isForeign(event, directory)) return undefined
  const data = event.data
  const item = data?.item
  if (typeof data?.sessionID !== "string" || !data.sessionID) return undefined
  if (item?.type !== "user" || typeof item.payload?.text !== "string") return undefined
  return { sessionID: data.sessionID, text: item.payload.text.trim() }
}

// The 2.x event stream ends or throws when the server restarts; reconnect with
// a bounded doubling delay that resets once events flow again.
const V2_EVENT_RETRY_MIN_MS = 50
const V2_EVENT_RETRY_MAX_MS = 5000

function delayUnlessAborted(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    if (signal.aborted) return resolve()
    const onAbort = () => {
      clearTimeout(timer)
      resolve()
    }
    const timer = setTimeout(() => {
      signal.removeEventListener("abort", onAbort)
      resolve()
    }, ms)
    signal.addEventListener("abort", onAbort, { once: true })
  })
}

async function setupLeteoV2(ctx: V2Context): Promise<() => Promise<void>> {
  const hooks = await Leteo({ directory: ctx.location.directory } as any)

  const abort = new AbortController()
  const registrations: V2Registration[] = []
  let listening: Promise<void> = Promise.resolve()
  const compactions = new Map<string, number>()
  const cleanup = async () => {
    abort.abort()
    await Promise.all(registrations.map((registration) => registration.dispose()))
    await listening
  }

  try {
    // The protocol is re-injected on every message, which is what makes memory
    // survive a compaction: the agent is told again how to use it.
    registrations.push(
      await ctx.session.hook("context", async (request) => {
        await withSystemStrings(request.system, (system) =>
          hooks["experimental.chat.system.transform"]({ sessionID: request.sessionID }, { system }),
        )
      }),
    )

    // 2.x renamed the 1.x `Task` delegation tool to `subagent`.
    registrations.push(
      await ctx.tool.hook("execute.after", async (call) => {
        const tool = call.tool === "subagent" ? "Task" : call.tool
        const output = call.status === "completed" ? v2ToolResultText(call.result) : ""
        await hooks["tool.execute.after"]({ tool, sessionID: call.sessionID }, output)
      }),
    )

    listening = (async () => {
      let retryMs = V2_EVENT_RETRY_MIN_MS
      while (!abort.signal.aborted) {
        try {
          for await (const event of ctx.event.subscribe({ signal: abort.signal })) {
            if (abort.signal.aborted) break
            // Every subscription opens with a server.connected handshake; only a
            // real event proves the stream is healthy enough to reset the backoff.
            if (event?.type !== "server.connected") retryMs = V2_EVENT_RETRY_MIN_MS
            try {
              const prompt = v2InboxPrompt(event, ctx.location.directory)
              if (prompt) {
                await hooks["chat.message"](
                  { sessionID: prompt.sessionID },
                  { parts: [{ type: "text", text: prompt.text }], message: {} },
                )
              }
              const translated = v1SessionEvent(event, ctx.location.directory)
              if (translated) {
                const sessionID: string = translated.properties?.sessionID ?? translated.properties?.info?.id
                if (translated.type === "session.deleted") compactions.delete(sessionID)
                if (translated.type !== "session.compacted" || firstOfCompaction(compactions, sessionID)) {
                  await hooks.event({ event: translated })
                }
              }
            } catch {
              // One failing event must not stop lifecycle tracking.
            }
          }
        } catch {
          // Events missed while disconnected are lost; hooks still bind sessions lazily.
        }
        if (abort.signal.aborted) break
        await delayUnlessAborted(retryMs, abort.signal)
        retryMs = Math.min(retryMs * 2, V2_EVENT_RETRY_MAX_MS)
      }
    })()
  } catch (cause) {
    await cleanup()
    throw cause
  }

  return cleanup
}

// OpenCode 1.x calls `server`; 2.x calls `setup`.
export default { id: "leteo", server: Leteo, setup: setupLeteoV2 }
