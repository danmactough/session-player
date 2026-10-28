import type { SessionEvent } from "../types";

export interface ToolStep {
  call: SessionEvent;
  result?: SessionEvent;
}

export type Block =
  | { type: "user"; ev: SessionEvent; turn: number }
  | { type: "assistant"; ev: SessionEvent }
  | { type: "thinking"; ev: SessionEvent }
  | { type: "tools"; steps: ToolStep[] }
  | { type: "command"; ev: SessionEvent; output?: SessionEvent }
  | { type: "system"; ev: SessionEvent }
  | { type: "compaction"; ev: SessionEvent }
  | { type: "orphan_result"; ev: SessionEvent };

/**
 * Turns the flat event stream into display blocks: tool results are attached to
 * the call that produced them, and runs of consecutive tool calls collapse into
 * one block so the assistant's prose stays readable.
 */
export function buildBlocks(events: SessionEvent[]): Block[] {
  const results = new Map<string, SessionEvent>();
  for (const e of events) {
    if (e.kind === "tool_result" && e.call_id) results.set(e.call_id, e);
  }
  const callIds = new Set(events.filter((e) => e.kind === "tool_call" && e.call_id).map((e) => e.call_id!));

  const blocks: Block[] = [];
  let turn = 0;
  for (let i = 0; i < events.length; i++) {
    const e = events[i];
    switch (e.kind) {
      case "user":
        if (!e.sidechain) turn++;
        blocks.push({ type: "user", ev: e, turn });
        break;
      case "assistant":
        blocks.push({ type: "assistant", ev: e });
        break;
      case "thinking":
        blocks.push({ type: "thinking", ev: e });
        break;
      case "tool_call": {
        const step = { call: e, result: e.call_id ? results.get(e.call_id) : undefined };
        const last = blocks[blocks.length - 1];
        if (last && last.type === "tools") last.steps.push(step);
        else blocks.push({ type: "tools", steps: [step] });
        break;
      }
      case "tool_result":
        if (!e.call_id || !callIds.has(e.call_id)) blocks.push({ type: "orphan_result", ev: e });
        break;
      case "command": {
        const next = events[i + 1];
        // Shell escapes and slash commands are followed by their output as a system event.
        if (next && next.kind === "system" && next.text !== "Interrupted by user") {
          blocks.push({ type: "command", ev: e, output: next });
          i++;
        } else {
          blocks.push({ type: "command", ev: e });
        }
        break;
      }
      case "system":
        blocks.push({ type: "system", ev: e });
        break;
      case "compaction":
        blocks.push({ type: "compaction", ev: e });
        break;
    }
  }
  return blocks;
}

export function blockText(b: Block): string {
  switch (b.type) {
    case "tools":
      return b.steps.map((s) => `${s.call.tool_name ?? ""} ${s.call.text} ${s.result?.text ?? ""}`).join("\n");
    case "command":
      return `${b.ev.text}\n${b.output?.text ?? ""}`;
    default:
      return b.ev.text;
  }
}
