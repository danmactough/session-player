import { useMemo } from "react";
import type { AnnotationKind, NoteSummary, Outcome, SessionRow } from "../types";
import { KINDS, KIND_LABEL, OUTCOME_LABEL, sessionKey } from "../lib/notes";

const OUTCOMES: Outcome[] = ["succeeded", "partial", "failed", "unknown"];
const CONFIDENCES = ["high", "medium", "low"] as const;

interface Props {
  /** The sessions the list shows; every figure counts only these. */
  sessions: SessionRow[];
  notes: Map<string, NoteSummary>;
  outcomeFilter: Outcome[];
  kindFilter: AnnotationKind[];
  onOutcome: (o: Outcome) => void;
  onKind: (k: AnnotationKind) => void;
}

interface Stats {
  sessions: number;
  analyzed: number;
  annotated: number;
  annotations: number;
  outcomes: Record<Outcome, number>;
  /** Per kind: annotations, and sessions with at least one. */
  kinds: { kind: AnnotationKind; annotations: number; sessions: number }[];
  person: number;
  model: number;
  confidence: Record<(typeof CONFIDENCES)[number], number>;
}

function tally(sessions: SessionRow[], notes: Map<string, NoteSummary>): Stats {
  const outcomes = { succeeded: 0, partial: 0, failed: 0, unknown: 0 };
  const byKind = new Map<AnnotationKind, { annotations: number; sessions: number }>();
  const confidence = { high: 0, medium: 0, low: 0 };
  let analyzed = 0;
  let annotated = 0;
  let annotations = 0;
  let person = 0;
  let model = 0;
  // A pi session's branches share one key, so each set of notes counts once.
  const seen = new Set<string>();
  for (const s of sessions) {
    const key = sessionKey(s);
    if (seen.has(key)) continue;
    seen.add(key);
    const n = notes.get(key);
    outcomes[n?.outcome ?? "unknown"]++;
    if (!n) continue;
    if (n.analyzed) analyzed++;
    const count = n.person_annotations + n.model_annotations;
    if (count > 0) annotated++;
    annotations += count;
    person += n.person_annotations;
    model += n.model_annotations;
    for (const c of CONFIDENCES) confidence[c] += n.confidence[c] ?? 0;
    for (const [kind, k] of Object.entries(n.kind_counts) as [AnnotationKind, number][]) {
      const row = byKind.get(kind) ?? { annotations: 0, sessions: 0 };
      row.annotations += k;
      row.sessions++;
      byKind.set(kind, row);
    }
  }
  const kinds = KINDS.filter((k) => byKind.has(k))
    .map((kind) => ({ kind, ...byKind.get(kind)! }))
    .sort((a, b) => b.annotations - a.annotations || b.sessions - a.sessions);
  return { sessions: seen.size, analyzed, annotated, annotations, outcomes, kinds, person, model, confidence };
}

const pct = (n: number, of: number) => (of ? Math.round((n / of) * 100) : 0);

export function AnalysisView({ sessions, notes, outcomeFilter, kindFilter, onOutcome, onKind }: Props) {
  const st = useMemo(() => tally(sessions, notes), [sessions, notes]);
  const maxKind = Math.max(1, ...st.kinds.map((k) => k.annotations));
  const maxOutcome = Math.max(1, ...OUTCOMES.map((o) => st.outcomes[o]));

  return (
    <div className="stats-view">
      <header className="stats-head" data-tauri-drag-region>
        <h1 data-tauri-drag-region>Analysis</h1>
        <p className="dim" data-tauri-drag-region>
          Counts cover the {st.sessions} {st.sessions === 1 ? "session" : "sessions"} the list shows. Click an outcome or annotation type to
          narrow the list to it.
        </p>
      </header>

      <div className="stat-tiles">
        <Tile label="Sessions" value={st.sessions} />
        <Tile label="Analyzed" value={st.analyzed} of={st.sessions} />
        <Tile label="With annotations" value={st.annotated} of={st.sessions} />
        <Tile label="Annotations" value={st.annotations} />
      </div>

      <section className="stats-section">
        <h2>Outcomes</h2>
        <ul className="bars" role="list">
          {OUTCOMES.map((o) => (
            <BarRow
              key={o}
              label={<span className={`outcome outcome-${o}`}>{OUTCOME_LABEL[o]}</span>}
              value={st.outcomes[o]}
              max={maxOutcome}
              detail={`${pct(st.outcomes[o], st.sessions)}%`}
              title={`${st.outcomes[o]} of ${st.sessions} sessions ${o === "unknown" ? "have no known outcome" : o}`}
              barClass={`bar-${o}`}
              on={outcomeFilter.length === 1 && outcomeFilter[0] === o}
              onClick={() => onOutcome(o)}
            />
          ))}
        </ul>
      </section>

      <section className="stats-section">
        <h2>Annotation types</h2>
        {st.kinds.length === 0 ? (
          <p className="dim">None of these sessions have annotations yet.</p>
        ) : (
          <>
            <div className="bars-key dim">
              <span>Count</span>
              <span>Sessions</span>
            </div>
            <ul className="bars" role="list">
              {st.kinds.map((k) => (
                <BarRow
                  key={k.kind}
                  label={KIND_LABEL[k.kind]}
                  value={k.annotations}
                  max={maxKind}
                  detail={`${k.sessions}`}
                  title={`${k.annotations} ${k.annotations === 1 ? "annotation" : "annotations"} in ${k.sessions} ${k.sessions === 1 ? "session" : "sessions"} (${pct(k.sessions, st.sessions)}% of sessions)`}
                  on={kindFilter.length === 1 && kindFilter[0] === k.kind}
                  onClick={() => onKind(k.kind)}
                />
              ))}
            </ul>
          </>
        )}
      </section>

      {st.annotations > 0 && (
        <section className="stats-section stats-pair">
          <div>
            <h2>Written by</h2>
            <dl className="stat-list">
              <div className="stat-list-row">
                <dt>People</dt>
                <dd>{st.person}</dd>
              </div>
              <div className="stat-list-row">
                <dt>Analyze</dt>
                <dd>{st.model}</dd>
              </div>
            </dl>
          </div>
          {st.model > 0 && (
            <div>
              <h2>Analyze's confidence</h2>
              <dl className="stat-list">
                {CONFIDENCES.map((c) => (
                  <div key={c} className="stat-list-row">
                    <dt>{c[0].toUpperCase() + c.slice(1)}</dt>
                    <dd>{st.confidence[c]}</dd>
                  </div>
                ))}
              </dl>
            </div>
          )}
        </section>
      )}
    </div>
  );
}

function Tile({ label, value, of }: { label: string; value: number; of?: number }) {
  return (
    <div className="stat-tile">
      <div className="stat-value">{value}</div>
      <div className="stat-label">
        {label}
        {of !== undefined && <span className="dim"> · {pct(value, of)}%</span>}
      </div>
    </div>
  );
}

function BarRow({
  label,
  value,
  max,
  detail,
  title,
  barClass,
  on,
  onClick,
}: {
  label: React.ReactNode;
  value: number;
  max: number;
  detail: string;
  title: string;
  barClass?: string;
  on: boolean;
  onClick: () => void;
}) {
  return (
    <li>
      <button className={`bar-row ${on ? "on" : ""}`} onClick={onClick} title={title} aria-pressed={on}>
        <span className="bar-label">{label}</span>
        <span className="bar-track">
          {value > 0 && <span className={`bar-fill ${barClass ?? ""}`} style={{ width: `${(value / max) * 100}%` }} />}
        </span>
        <span className="bar-value">{value}</span>
        <span className="bar-detail">{detail}</span>
      </button>
    </li>
  );
}
