/**
 * Leteo — Pi package
 *
 * Registers Leteo's MCP server with Pi, maps Pi's lifecycle events onto
 * `leteo hook <event>`, and appends the memory protocol to every system prompt
 * so the agent keeps using its tools after a compaction.
 *
 * Why this is thin: every decision lives in the Leteo binary. Project
 * detection, the stable manual session, `<private>` redaction and the length
 * bound, deduplication and the save reminder are implemented once, in Rust, and
 * shared with the CLI, the MCP server and every other agent's hooks. Nothing
 * here redacts or truncates: a prompt cut at a character count before the
 * binary saw it could leave half a `<private>` span with no closing tag, and
 * the binary would then have nothing to match.
 */

import { spawn } from "node:child_process"
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent"

/**
 * Filled in by `leteo setup pi` with the binary that ran it, as every other
 * agent's configuration is. Empty in the source, where PATH decides.
 */
const INSTALLED_BINARY = ""

/**
 * Filled in by `leteo setup pi` with the tool profile it was asked for. The
 * source holds the default, and a test holds it to `setup::DEFAULT_TOOLS`.
 */
const MCP_TOOLS = "agent"

/**
 * Binary to invoke: LETEO_BIN, then the one setup recorded, then `leteo` on
 * PATH. Installed from npm, setup has recorded nothing and PATH decides.
 */
export function leteoBinary(): string {
  return process.env.LETEO_BIN ?? (INSTALLED_BINARY || "leteo")
}

/**
 * The version of Pi that loaded this package, or `undefined` when it cannot be
 * read. Pi supplies `@earendil-works/pi-coding-agent` to extensions and aliases
 * the specifier to its own entry point, so the import resolves inside Pi and
 * fails in the test runner, which has no `node_modules` and starts no Pi.
 */
export async function piVersion(): Promise<string | undefined> {
  try {
    const { VERSION } = await import("@earendil-works/pi-coding-agent")
    return typeof VERSION === "string" && VERSION.trim() !== "" ? VERSION : undefined
  } catch {
    return undefined
  }
}

/**
 * The exposure the memory tools are registered with, on the Pi that is running:
 * `deferred` from 1.2.0, and `direct` below it or when no version could be read.
 *
 * The gate sits at the release this was measured on, and `deferred` is older
 * than that: it is in the 0.99.0 line, where it was still dropping the tools
 * `tool_search` had loaded on resume and `/reload` until 1.0.0 fixed that, and a
 * 1.1.0 run reaches the tools through `tool_search`. Lowering the gate is a
 * change of its own, and owes its own measurement of the versions it moves.
 */
export function exposureFor(version: string | undefined): "deferred" | "direct" {
  return atLeastVersion(version, "1.2.0") ? "deferred" : "direct"
}

/** Whether a dotted version reaches `minimum`. A version that cannot be read does not. */
function atLeastVersion(version: string | undefined, minimum: string): boolean {
  if (!version) return false
  const numbers = (text: string) => text.split(".").map((part) => Number.parseInt(part, 10))
  const running = numbers(version)
  const wanted = numbers(minimum)
  if (running.some(Number.isNaN) || wanted.some(Number.isNaN)) return false
  for (let index = 0; index < Math.max(running.length, wanted.length); index++) {
    const left = running[index] ?? 0
    const right = wanted[index] ?? 0
    if (left !== right) return left > right
  }
  return true
}

/** What Pi calls the tools of the MCP server registered as `leteo`. */
const MCP_SERVER = "leteo"
const MCP_TOOL_PREFIX = `mcp__${MCP_SERVER}__`

/** Prompts this short are not worth persisting. */
const MIN_PROMPT_LENGTH = 10
/** Tool output this short cannot hold a learnings section. */
const MIN_CAPTURE_LENGTH = 50
/** A hook must never delay the user's turn; it is killed past this. */
const HOOK_TIMEOUT_MS = 5000

/**
 * The name this package gives `leteo hook` for the binary's Pi guard. Claude
 * Code's hooks are plain `leteo hook` commands that fire inside Pi too, through
 * pi-claude-bridge, and the binary silences them when it sees `PI_CODING_AGENT`.
 * This package inherits the same variable, so it says who it is.
 */
const HOOK_CLIENT = "pi"

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

/**
 * Pi names an MCP server's tools `mcp__<server>__<tool>`, and the protocol
 * above says `mem_save`. The model matches the suffix without help, but a
 * sentence costs less than a tool call that names a tool that is not there.
 */
const TOOL_NAMING = `In this session Leteo's tools are named \`${MCP_TOOL_PREFIX}<tool>\`, for example \`${MCP_TOOL_PREFIX}mem_save\`.`

/** One hook event's payload, matching what the Rust side deserializes. */
interface HookInput {
  session_id?: string
  cwd?: string
  prompt?: string
  stdout?: string
  source?: string
  /** Leave the memory directive out of a `session-start` answer. */
  without_directive?: boolean
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
 * Runs one hook and says whether it ran. `ok` is false when the binary could not
 * be started, was killed at the timeout, exited non-zero, or printed something
 * that is not JSON, and an empty `output` then means "did not run" rather than
 * "had nothing to add". Callers that act on the difference use this.
 *
 * Failures never throw: memory is an assistant, not a gate, and a missing binary
 * must never break the user's session.
 *
 * Exported for the contract test, which drives it with a binary that stops
 * reading to prove the stdin error handler below holds.
 */
export function attemptHook(
  event: HookEvent,
  input: HookInput,
): Promise<{ ok: boolean; output: HookOutput }> {
  return new Promise((resolve) => {
    let settled = false
    let timer: ReturnType<typeof setTimeout> | undefined
    const finish = (ok: boolean, output: HookOutput = {}) => {
      if (settled) return
      settled = true
      if (timer) clearTimeout(timer)
      resolve({ ok, output })
    }

    let child
    try {
      child = spawn(leteoBinary(), ["hook", event], {
        stdio: ["pipe", "pipe", "ignore"],
        env: { ...process.env, LETEO_HOOK_CLIENT: HOOK_CLIENT },
      })
    } catch {
      finish(false)
      return
    }

    let stdout = ""
    child.stdout?.setEncoding("utf8")
    child.stdout?.on("data", (chunk: string) => {
      stdout += chunk
    })
    child.on("error", () => finish(false))
    child.on("close", (code: number | null) => {
      try {
        finish(code === 0, stdout.trim() ? (JSON.parse(stdout) as HookOutput) : {})
      } catch {
        finish(false)
      }
    })
    // A child that stops reading before the payload drains closes its read end,
    // and the write then emits `EPIPE`. Without a listener Node rethrows that
    // as an uncaught exception and takes Pi down with it.
    child.stdin?.on("error", () => {})
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

/** A hook's response, `{}` when it did not run. */
export async function runHook(event: HookEvent, input: HookInput): Promise<HookOutput> {
  return (await attemptHook(event, input)).output
}

/**
 * Runs `leteo` with arguments, for the one write no hook carries, and says
 * whether it exited zero. A store that was locked, or a process killed at the
 * timeout, is a summary that was not saved, and the caller has to be able to
 * tell.
 */
function runLeteo(args: string[], cwd: string): Promise<boolean> {
  return new Promise((resolve) => {
    try {
      const child = spawn(leteoBinary(), args, { cwd, stdio: "ignore" })
      const timer = setTimeout(() => {
        try {
          child.kill()
        } catch {}
      }, HOOK_TIMEOUT_MS)
      child.on("error", () => {
        clearTimeout(timer)
        resolve(false)
      })
      child.on("close", (code: number | null) => {
        clearTimeout(timer)
        resolve(code === 0)
      })
    } catch {
      resolve(false)
    }
  })
}

function additionalContext(output: HookOutput): string {
  return output.hookSpecificOutput?.additionalContext?.trim() ?? ""
}

/**
 * The text of a tool result. Pi hands over `{ content: [{ type: "text", text }] }`
 * or a bare string; the text is what is sent rather than its JSON, because JSON
 * escapes the newlines the learnings heading is recognised by.
 */
export function resultText(result: unknown): string {
  if (typeof result === "string") return result
  const content = (result as { content?: unknown } | null | undefined)?.content
  if (Array.isArray(content)) {
    return content
      .filter((part) => part?.type === "text" && typeof part.text === "string")
      .map((part) => part.text)
      .join("\n")
  }
  if (result === undefined || result === null) return ""
  try {
    return JSON.stringify(result)
  } catch {
    return ""
  }
}

/** The Pi runtime's own id, which is also the id of the Leteo session. */
function sessionId(ctx: { sessionManager?: { getSessionId?: () => string } }): string {
  try {
    return ctx.sessionManager?.getSessionId?.()?.trim() ?? ""
  } catch {
    return ""
  }
}

function appendPromptSection(options: { appendSystemPrompt?: string }, section: string): void {
  const existing = options.appendSystemPrompt ?? ""
  if (existing.includes(section)) return
  options.appendSystemPrompt = existing.length > 0 ? `${existing}\n\n${section}` : section
}

/**
 * What a compaction summary is filed under: its first line that is not a
 * heading, which is what `mem_session_summary` does with the same text.
 */
export function summaryTitle(summary: string): string {
  const line = summary.split("\n").map((text) => text.trim()).find((text) => text && !text.startsWith("#"))
  return (line ?? "Pi compaction summary").replace(/\s+/g, " ").slice(0, 100)
}

export default async function leteo(pi: ExtensionAPI) {
  /** Context to put in front of the agent on its next turn, by session. */
  const pendingContext = new Map<string, string>()
  /** Work started without being awaited, which shutdown waits for. */
  const inFlight = new Set<Promise<unknown>>()
  let lastInputSource: string | undefined

  function track(work: Promise<unknown>): void {
    inFlight.add(work)
    work.finally(() => inFlight.delete(work))
  }

  // Registering only declares the server; Pi connects it when the session
  // starts, so nothing is spawned from the factory.
  //
  // The protocol this package appends tells the model to call these tools
  // unprompted, after a bug fix or a decision, in the middle of other work, and
  // a tool that is not declared is one it has to search for first. That was the
  // trade while a declaration cost what it did; measured on Pi 1.2.0 with
  // `opencode-go/deepseek-v4.1-flash`, an empty project, a throwaway store and
  // Estigia's server configured beside this one, first-request prompt tokens were
  // 13,832 with no Leteo, 20,037 with `direct`, 14,797 with `deferred`, and
  // 15,963 with the four session tools kept `direct`. `deferred` costs 5,240
  // tokens less than `direct` and 1,166 less than the mixed registration, and
  // the search is not a wall: one recorded run under it called `tool_search`,
  // then `mem_session_start`, then `mem_save`.
  //
  // `direct` is what Pi before 1.2.0 keeps, which is what it has always had.
  pi.registerMcpServer(MCP_SERVER, {
    command: leteoBinary(),
    args: ["mcp", `--tools=${MCP_TOOLS}`],
    exposure: exposureFor(await piVersion()),
    description: "Leteo persistent memory: save and recall decisions, fixes and conventions across sessions",
  })

  // The server works out the project from the directory it is started in, and
  // files a save under that project's shared manual session unless it is told a
  // session. The session this package opened with the real id owns the right
  // project and the right conversation, so the id is added to any Leteo call
  // whose schema takes one. Pi allows an extension to change a call's input in
  // place from `tool_call`.
  pi.on("tool_call", (event, ctx) => {
    if (!event.toolName.startsWith(MCP_TOOL_PREFIX)) return
    const id = sessionId(ctx)
    if (!id || event.input.session_id !== undefined) return
    const schema = pi.getAllTools().find((tool) => tool.name === event.toolName)?.parameters as
      | { properties?: Record<string, unknown> }
      | undefined
    if (schema?.properties?.session_id) event.input.session_id = id
  })

  // Pi's queued steer and follow-up inputs bypass before_agent_start, and input
  // that another extension injected is not something the user said.
  pi.on("input", (event) => {
    if (!event.streamingBehavior) lastInputSource = event.source
    return { action: "continue" as const }
  })

  pi.on("session_start", async (event, ctx) => {
    const id = sessionId(ctx)
    if (!id) return
    // The protocol is appended to every turn below; the directive on top of it
    // was about 925 bytes of the same rules said again.
    const opening = additionalContext(
      await runHook("session-start", { session_id: id, cwd: ctx.cwd, without_directive: true }),
    )
    // A reload keeps the transcript, so the opening context is already in it.
    if (opening && event.reason !== "reload") pendingContext.set(id, opening)
  })

  pi.on("before_agent_start", async (event, ctx) => {
    const id = sessionId(ctx)
    const source = lastInputSource
    lastInputSource = undefined

    // Asked before the section is built because the hook records every memory it
    // names as shown, so its answer has to reach the model: it is added to this
    // run's system prompt, which is where Pi lets an extension speak per prompt.
    const prompt = event.prompt?.trim() ?? ""
    let hint = ""
    if (id && source !== "extension" && prompt.length > MIN_PROMPT_LENGTH) {
      hint = additionalContext(await runHook("user-prompt-submit", { session_id: id, cwd: ctx.cwd, prompt }))
    }

    let section = `${MEMORY_PROTOCOL}\n\n${TOOL_NAMING}`
    const pending = id ? pendingContext.get(id) : undefined
    if (pending) {
      section += `\n\n${pending}`
      pendingContext.delete(id)
    }
    if (hint) section += `\n\n${hint}`

    // The structured options are the supported place for an addition on newer
    // Pi: a returned `systemPrompt` replaces the whole prompt for the run and
    // turns that skip this hook never see it. Older Pi has only the return.
    const options = event.systemPromptOptions as { appendSystemPrompt?: string } | undefined
    let result: { systemPrompt: string } | undefined
    if (options && typeof options === "object") {
      appendPromptSection(options, section)
    } else {
      const base = event.systemPrompt ?? ""
      result = { systemPrompt: base.length > 0 ? `${base}\n\n${section}` : section }
    }

    return result
  })

  pi.on("tool_execution_end", (event, ctx) => {
    const id = sessionId(ctx)
    const name = event.toolName ?? ""
    if (!id || name.toLowerCase().startsWith(MCP_TOOL_PREFIX)) return
    const text = resultText(event.result)
    if (text.length <= MIN_CAPTURE_LENGTH) return
    // Not awaited: this fires after every tool call, and a process spawn per
    // call must not hold the turn up. Shutdown waits for what is still running.
    track(runHook("subagent-stop", { session_id: id, cwd: ctx.cwd, stdout: text, source: name }))
  })

  pi.on("session_compact", async (event, ctx) => {
    const id = sessionId(ctx)
    if (!id) return
    const summary = event.compactionEntry?.summary?.trim()
    // The summary first, so the context the hook hands back already holds it.
    // Through `leteo save` because no tool can be called from here and a hook
    // payload has no field for it; the process runs in the session's directory
    // so the project is the one the session belongs to. Options come first and
    // `--` ends them, because a summary is model-written text and one that opens
    // with `-` or `--project=other` would otherwise be read as an option.
    let saved = true
    if (summary) {
      saved = await runLeteo(
        ["save", "--type", "session_summary", "--session", id, "--", summaryTitle(summary), summary],
        ctx.cwd,
      )
    }
    const { ok, output } = await attemptHook("post-compaction", { session_id: id, cwd: ctx.cwd })

    // Said to the model as well as the user, because what the recovery text
    // holds depends on the save: a summary that was not stored is not in it.
    const notices: string[] = []
    if (!saved) {
      notices.push(
        "Leteo could not store the summary of the compaction that just happened, so the context below does not include it. Call the Leteo session summary tool with what was done so far.",
      )
    }
    if (!ok) notices.push("Leteo could not recover this project's context after the compaction.")
    if (notices.length > 0) {
      try {
        ctx.ui?.notify?.(notices.join(" "), "warning")
      } catch {}
    }
    const recovered = [...notices, additionalContext(output)].filter(Boolean).join("\n\n")
    if (recovered) pendingContext.set(id, recovered)
  })

  pi.on("session_shutdown", async (event, ctx) => {
    // Pi tears the extension down and rebuilds it on a reload, with the same
    // session still running.
    if (event.reason === "reload") return
    const id = sessionId(ctx)
    pendingContext.delete(id)
    await Promise.allSettled([...inFlight])
    if (id) await runHook("session-stop", { session_id: id, cwd: ctx.cwd })
  })
}
