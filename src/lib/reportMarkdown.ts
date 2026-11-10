import type { ActionResponse, ActionStatus, CitedAnnotation, Evidence, LatestVerdict, Report, UserImpact, Verdict } from "../types";
import { KIND_LABEL, OUTCOME_LABEL } from "./notes";
import { fullDate } from "./format";

export const CATEGORY_LABEL = {
  codebase: "Codebase",
  skill: "Skill",
  instructions: "Prompt or instructions",
  environment: "Environment",
  automation: "Automation",
} as const;

export const PRIORITY_LABEL = { high: "High", medium: "Medium", low: "Low" } as const;

export const STATUS_LABEL: Record<ActionStatus, string> = { done: "Done", in_progress: "In progress", declined: "Declined", considering: "Thinking about it" };
export const USER_IMPACT_LABEL: Record<UserImpact, string> = { helped: "Helped", did_not_help: "Didn’t help" };
export const VERDICT_LABEL: Record<Verdict, string> = { worked: "Worked", did_not_work: "Didn’t work", too_early: "Too early to tell" };

/** Failures first: they are what the section exists to show. */
export const verdictOrder = (v: Verdict) => (v === "did_not_work" ? 0 : v === "worked" ? 1 : 2);

/** "claude-sonnet-5-5" → "Sonnet 5.5"; anything else as it is. */
export function modelName(model: string): string {
  const m = model.match(/^claude-([a-z]+)-(\d+)-(\d+)/);
  return m ? `${m[1][0].toUpperCase()}${m[1].slice(1)} ${m[2]}.${m[3]}` : model;
}

const oneLine = (s: string) => s.replace(/\s+/g, " ").trim();

/** One evidence line: what an annotation said (as the report saw it), or a session's outcome. */
export function evidenceLine(e: Evidence, report: Report, cited: Map<string, CitedAnnotation>): string {
  const session = report.sessions.find((s) => s.key === e.key);
  const title = session ? `“${oneLine(session.title)}”` : "a session";
  if (e.type === "session") {
    const outcome = session ? OUTCOME_LABEL[session.outcome] : "Unknown";
    return `${outcome} session ${title}${session?.outcome_note ? `: ${oneLine(session.outcome_note)}` : ""}`;
  }
  const a = cited.get(`${e.id}@${e.version}`)?.annotation;
  return a ? `${KIND_LABEL[a.kind]} in ${title}: ${oneLine(a.note)}` : `An annotation in ${title}`;
}

/** The report as Markdown, for a ticket, a PR description or a file. Each
 * citation is a footnote; one cited by several takeaways or actions is one
 * footnote referenced from each. */
export function reportMarkdown(
  report: Report,
  cited: Map<string, CitedAnnotation>,
  { responses, verdicts }: { responses: Map<number, ActionResponse>; verdicts: Map<number, LatestVerdict> },
): string {
  const out: string[] = [`# ${report.title}`, ""];
  out.push(`${fullDate(report.created_at)} · ${report.sessions.length} sessions · ${modelName(report.model)}`, "");
  if (report.focus) out.push(`Focus: ${oneLine(report.focus)}`, "");
  if (report.summary) out.push(report.summary, "");
  // Labels are numbered in the order evidence is first cited. GitHub and others
  // number footnotes themselves; the "e" keeps ours apart from any the model wrote.
  const notes = new Map<string, { label: string; text: string }>();
  const refs = (list: Evidence[]) =>
    list
      .map((e) => {
        const id = e.type === "session" ? `s:${e.key}` : `a:${e.id}@${e.version}`;
        let n = notes.get(id);
        if (!n) notes.set(id, (n = { label: `e${notes.size + 1}`, text: evidenceLine(e, report, cited) }));
        return n.label;
      })
      // In number order, though first cited elsewhere in another order.
      .sort((a, b) => +a.slice(1) - +b.slice(1))
      // Spaced, or adjacent superscripts read as one number ("12" for 1 and 2).
      .map((label) => `[^${label}]`)
      .join(" ");
  if (report.takeaways.length) {
    out.push("## Key takeaways", "");
    report.takeaways.forEach((t, i) => out.push(`${i + 1}. ${t.text}${refs(t.evidence)}`));
    out.push("");
  }
  if (report.actions.length) {
    out.push("## Recommended actions", "");
    report.actions.forEach((a, i) => {
      // On a line of its own: the change is Markdown and may end in a list or code block.
      out.push(`### ${a.title}`, "", `**${PRIORITY_LABEL[a.priority]} priority** · ${CATEGORY_LABEL[a.category]}`, "", a.change, "");
      if (a.follows_up) out.push(`Follows up on: ${oneLine(a.follows_up.title)}`, "");
      if (a.success_check) out.push(`Success check: ${oneLine(a.success_check)}`, "");
      const r = responses.get(i);
      if (r) {
        const status = r.status ? STATUS_LABEL[r.status] : "No status";
        const when = r.done_at ? ` on ${fullDate(r.done_at)}` : "";
        const impact = r.user_impact ? ` (${USER_IMPACT_LABEL[r.user_impact]})` : "";
        out.push(`Status: ${status}${when}${impact}${r.note ? `. Note: ${oneLine(r.note)}` : ""}`, "");
      }
      const v = verdicts.get(i);
      if (v) out.push(`Latest verdict: ${VERDICT_LABEL[v.verdict]}, per “${oneLine(v.report_title)}”`, "");
      out.push(`Evidence: ${refs(a.evidence)}`, "");
    });
  }
  if (report.impact.length) {
    out.push("## Did earlier actions work?", "");
    for (const v of [...report.impact].sort((a, b) => verdictOrder(a.verdict) - verdictOrder(b.verdict))) {
      out.push(`### ${v.action_title}`, "", `**${VERDICT_LABEL[v.verdict]}.** ${v.reasoning}`, "");
      if (v.evidence.length) out.push(`Evidence: ${refs(v.evidence)}`, "");
    }
  }
  out.push("## Sessions", "");
  for (const s of report.sessions) out.push(`- ${oneLine(s.title)} (${OUTCOME_LABEL[s.outcome].toLowerCase()})`);
  if (notes.size) out.push("", ...[...notes.values()].map((n) => `[^${n.label}]: ${n.text}`));
  return out.join("\n") + "\n";
}
