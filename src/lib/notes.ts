import type { AnnotationKind, EventRef, Outcome, SessionEvent, SessionRow } from "../types";
import type { Block } from "./blocks";

/** The key a session's metadata and annotations are stored under (see
 * `session_key` in annotations.rs). A pi session's branches share it. */
export function sessionKey(s: SessionRow): string {
  return `${s.from_bundle ? "bundle" : "local"}:${s.source}:${s.native_id}`;
}

export const KIND_LABEL: Record<AnnotationKind, string> = {
  tool_error: "Tool error",
  human_intervention: "Human intervention",
  agent_stuck: "Agent stuck",
  human_frustration: "Human frustration",
  wrong_assumption: "Wrong assumption",
  wasted_work: "Wasted or redone work",
  environment: "Environment or setup",
  missing_context: "Missing context",
  automatable: "Worth automating",
  key_decision: "Key decision",
  ignored_instruction: "Ignored instruction",
  other: "Other",
};

export const KINDS = Object.keys(KIND_LABEL) as AnnotationKind[];

export const OUTCOME_LABEL: Record<Outcome, string> = {
  unknown: "Unknown",
  succeeded: "Succeeded",
  partial: "Partial",
  failed: "Failed",
};

export const ref = (e: SessionEvent): EventRef => ({ anchor: e.anchor ?? "", kind: e.kind, ts: e.ts, entry: e.entry, name: e.context?.name });

/** The events a block shows, which an annotation on the block points at. */
export function blockEvents(b: Block): SessionEvent[] {
  switch (b.type) {
    case "tools":
      return b.steps.flatMap((s) => (s.result ? [s.call, s.result] : [s.call]));
    case "command":
      return b.output ? [b.ev, b.output] : [b.ev];
    case "fork_from":
    case "fork_to":
      return [];
    default:
      return [b.ev];
  }
}
