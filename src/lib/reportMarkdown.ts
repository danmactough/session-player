import type { CitedAnnotation, Evidence, Report } from "../types";
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

/** The report as Markdown, for a ticket, a PR description or a file. */
export function reportMarkdown(report: Report, cited: Map<string, CitedAnnotation>): string {
  const out: string[] = [`# ${report.title}`, ""];
  out.push(`${fullDate(report.created_at)} · ${report.sessions.length} sessions · ${modelName(report.model)}`, "");
  if (report.focus) out.push(`Focus: ${oneLine(report.focus)}`, "");
  if (report.summary) out.push(report.summary, "");
  const evidence = (list: Evidence[]) => list.map((e) => `  - ${evidenceLine(e, report, cited)}`);
  if (report.takeaways.length) {
    out.push("## Key takeaways", "");
    report.takeaways.forEach((t, i) => out.push(`${i + 1}. ${t.text}`, ...evidence(t.evidence)));
    out.push("");
  }
  if (report.actions.length) {
    out.push("## Recommended actions", "");
    for (const a of report.actions) {
      out.push(`### ${a.title}`, "", `**${PRIORITY_LABEL[a.priority]} priority** · ${CATEGORY_LABEL[a.category]}`, "", a.change, "", "Evidence:", "");
      out.push(...evidence(a.evidence).map((l) => l.slice(2)), "");
    }
  }
  out.push("## Sessions", "");
  for (const s of report.sessions) out.push(`- ${oneLine(s.title)} (${OUTCOME_LABEL[s.outcome].toLowerCase()})`);
  return out.join("\n") + "\n";
}
