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

import type { Plugin } from "@opencode-ai/plugin"

/** Binary to invoke. Set LETEO_BIN when it is not on PATH. */
const LETEO_BIN = process.env.LETEO_BIN ?? "leteo"

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
 */
async function runHook(event: HookEvent, input: HookInput): Promise<HookOutput> {
  try {
    const child = Bun.spawn([LETEO_BIN, "hook", event], {
      stdin: new TextEncoder().encode(JSON.stringify(input)),
      stdout: "pipe",
      stderr: "ignore",
    })
    const timeout = setTimeout(() => child.kill(), HOOK_TIMEOUT_MS)
    const stdout = await new Response(child.stdout).text()
    clearTimeout(timeout)
    await child.exited
    return stdout.trim() ? (JSON.parse(stdout) as HookOutput) : {}
  } catch {
    return {}
  }
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
   * Starts a memory session on demand, so a plugin loaded mid-conversation
   * still records the rest of it. The Rust side is idempotent, and this is
   * where the project migration and the `.leteo/` import happen.
   */
  async function ensureSession(sessionID: string): Promise<void> {
    if (!sessionID || subAgentSessions.has(sessionID)) return
    if (startedSessions.has(sessionID)) return
    startedSessions.add(sessionID)
    await runHook("session-start", { session_id: sessionID, cwd: directory })
  }

  /**
   * Returns this project's prior memory once per session.
   *
   * The compaction hook is the right one here: unlike `session-start` it
   * returns the memory alone, without the protocol this plugin already
   * injects, so nothing is said twice.
   */
  async function recoverContext(sessionID: string): Promise<string> {
    if (!sessionID || subAgentSessions.has(sessionID)) return ""
    if (recoveredSessions.has(sessionID)) return ""
    recoveredSessions.add(sessionID)
    const output = await runHook("post-compaction", {
      session_id: sessionID,
      cwd: directory,
    })
    return additionalContext(output)
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

      if (event.type === "session.deleted") {
        const sessionID: string | undefined = (event.properties as any)?.info?.id
        if (!sessionID) return
        if (!subAgentSessions.has(sessionID)) {
          await runHook("session-stop", { session_id: sessionID, cwd: directory })
        }
        startedSessions.delete(sessionID)
        subAgentSessions.delete(sessionID)
        recoveredSessions.delete(sessionID)
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
      if (content.length <= MIN_PROMPT_LENGTH) return

      await ensureSession(sessionID)
      await runHook("user-prompt-submit", {
        session_id: sessionID,
        cwd: directory,
        prompt: content,
      })
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
      }

      if (output.system.length > 0) {
        output.system[output.system.length - 1] += `\n\n${block}`
      } else {
        output.system.push(block)
      }
    },
  }
}
