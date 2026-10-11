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

When prior work may be relevant, call \`mem_context\` first. If it is not there,
call \`mem_search\`, then \`mem_get_observation\` for the full text.

### Projects

Every response says which project it used and why, in \`project\` and
\`project_source\`. If a write fails with \`ambiguous_project\`, ask the user
which project it belongs to, then retry with their choice plus
\`project_choice_reason=user_selected_after_ambiguous_project\` and the
\`recovery_token\` from that error. Never guess.

### Close sessions

Before saying the work is done, call \`mem_session_summary\` with the goal,
discoveries, accomplishments, next steps, and relevant files. After a
compaction, persist the compacted summary first, then call \`mem_context\`.`

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
      child = spawn(leteoBinary(), ["hook", event], {
        stdio: ["pipe", "pipe", "ignore"],
        env: { ...process.env, LETEO_HOOK_CLIENT: HOOK_CLIENT },
      })
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

/**
 * Runs `leteo` with arguments, for the one write no hook carries. Failures are
 * swallowed for the same reason a hook's are.
 */
function runLeteo(args: string[], cwd: string): Promise<void> {
  return new Promise((resolve) => {
    try {
      const child = spawn(leteoBinary(), args, { cwd, stdio: "ignore" })
      const timer = setTimeout(() => {
        try {
          child.kill()
        } catch {}
      }, HOOK_TIMEOUT_MS)
      child.on("error", () => resolve())
      child.on("close", () => {
        clearTimeout(timer)
        resolve()
      })
    } catch {
      resolve()
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

export default function leteo(pi: ExtensionAPI) {
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
  // `direct` because the protocol this package appends tells the model to call
  // these tools unprompted, after a bug fix or a decision, in the middle of
  // other work. Under the default `codemode` they are neither declared nor
  // listed, and under `deferred` the model must first search for them, so a
  // memory would be saved only by a model that had already thought to look.
  // The agent profile is about twenty tools, which is the "small, frequently
  // used tool set" Pi's documentation names for `direct`.
  pi.registerMcpServer(MCP_SERVER, {
    command: leteoBinary(),
    args: ["mcp", `--tools=${MCP_TOOLS}`],
    exposure: "direct",
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
    const opening = additionalContext(await runHook("session-start", { session_id: id, cwd: ctx.cwd }))
    // A reload keeps the transcript, so the opening context is already in it.
    if (opening && event.reason !== "reload") pendingContext.set(id, opening)
  })

  pi.on("before_agent_start", async (event, ctx) => {
    const id = sessionId(ctx)
    const source = lastInputSource
    lastInputSource = undefined

    let section = `${MEMORY_PROTOCOL}\n\n${TOOL_NAMING}`
    const pending = id ? pendingContext.get(id) : undefined
    if (pending) {
      section += `\n\n${pending}`
      pendingContext.delete(id)
    }

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

    const prompt = event.prompt?.trim() ?? ""
    if (id && source !== "extension" && prompt.length > MIN_PROMPT_LENGTH) {
      await runHook("user-prompt-submit", { session_id: id, cwd: ctx.cwd, prompt })
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
    // so the project is the one the session belongs to.
    if (summary) {
      await runLeteo(
        ["save", summaryTitle(summary), summary, "--type", "session_summary", "--session", id],
        ctx.cwd,
      )
    }
    const recovered = additionalContext(await runHook("post-compaction", { session_id: id, cwd: ctx.cwd }))
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
