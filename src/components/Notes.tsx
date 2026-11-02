import { useState } from "react";
import { ask } from "@tauri-apps/plugin-dialog";
import type { Annotation, AnnotationKind, Outcome, SessionMeta } from "../types";
import { KINDS, KIND_LABEL, OUTCOME_LABEL } from "../lib/notes";
import { fullDate } from "../lib/format";
import { Menu } from "./Menu";

const OUTCOMES = Object.keys(OUTCOME_LABEL) as Outcome[];

const blank = (key: string): SessionMeta => ({
  key,
  outcome: "unknown",
  outcome_note: null,
  components: [],
  updated_at: 0,
  generated: [],
  generated_by: null,
  cleared: [],
});

/**
 * The analysis of a session: how it turned out, the parts of the repository
 * it worked on, and its annotations, each of which jumps to its event.
 */
export function Analysis({
  sessionKey,
  meta,
  revision,
  annotations,
  onSaveMeta,
  onSaveAnnotation,
  onDeleteAnnotation,
  onJump,
}: {
  sessionKey: string;
  meta: SessionMeta | null;
  /** The transcript's current revision, to tell whether generated results are out of date. */
  revision: string | null;
  annotations: Annotation[];
  onSaveMeta: (meta: SessionMeta) => Promise<void>;
  onSaveAnnotation: (a: Annotation) => Promise<void>;
  onDeleteAnnotation: (a: Annotation) => Promise<void>;
  onJump: (a: Annotation) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [kind, setKind] = useState<AnnotationKind | "all">("all");
  const present = KINDS.filter((k) => annotations.some((a) => a.kind === k));
  // Deleting the last annotation of the chosen kind shows them all again.
  const active = kind !== "all" && !present.includes(kind) ? "all" : kind;
  const shown = active === "all" ? annotations : annotations.filter((a) => a.kind === active);
  const m = meta ?? blank(sessionKey);
  const hasOutcome = m.outcome !== "unknown" || !!m.outcome_note;
  const stale = !!(m.generated_by && revision && m.generated_by.revision !== revision);

  return (
    <section className="analysis">
      {editing ? (
        <AnalysisForm
          initial={m}
          onCancel={() => setEditing(false)}
          onSave={async (next) => {
            await onSaveMeta(next);
            setEditing(false);
          }}
        />
      ) : (
        <dl className="details-view">
          <div>
            <dt>Outcome</dt>
            <dd>
              {hasOutcome ? (
                <>
                  <span className={`outcome outcome-${m.outcome}`}>{OUTCOME_LABEL[m.outcome]}</span>
                  {m.outcome_note && <span> {m.outcome_note}</span>}
                </>
              ) : (
                <span className="dim">Not set</span>
              )}
            </dd>
          </div>
          <div>
            <dt>Components</dt>
            <dd className="tags">
              {m.components.length ? (
                m.components.map((c) => (
                  <span key={c} className="component">
                    {c}
                  </span>
                ))
              ) : (
                <span className="dim">Not set</span>
              )}
            </dd>
          </div>
          <div>
            <dt />
            <dd className="details-links">
              <button className="link" onClick={() => setEditing(true)}>
                Edit
              </button>
              {stale && m.generated.length > 0 && (
                <span className="stale">Generated from an earlier version of this transcript.</span>
              )}
            </dd>
          </div>
        </dl>
      )}

      <div className="analysis-annotations">
        <div className="analysis-annotations-head">
          <span className="details-label">
            Annotations{annotations.length ? ` · ${annotations.length}` : ""}
          </span>
          {present.length > 1 && (
            <Menu
              label="Filter annotations"
              value={active}
              options={[{ key: "all" as const, label: "All kinds" }, ...present.map((k) => ({ key: k, label: KIND_LABEL[k] }))]}
              onChange={setKind}
            />
          )}
        </div>
        {annotations.length === 0 && (
          <p className="dim">None yet. Hover a message and click + in the margin, or open a tool call and click Annotate this call.</p>
        )}
        {shown.map((a) => (
          <AnnotationItem
            key={a.id}
            annotation={a}
            revision={revision}
            onSave={onSaveAnnotation}
            onDelete={onDeleteAnnotation}
            onJump={onJump}
          />
        ))}
      </div>
    </section>
  );
}

function AnalysisForm({
  initial,
  onSave,
  onCancel,
}: {
  initial: SessionMeta;
  onSave: (meta: SessionMeta) => Promise<void>;
  onCancel: () => void;
}) {
  const [outcome, setOutcome] = useState<Outcome>(initial.outcome);
  const [outcomeNote, setOutcomeNote] = useState(initial.outcome_note ?? "");
  const [components, setComponents] = useState(initial.components.join(", "));
  const [err, setErr] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const save = async () => {
    setErr(null);
    setSaving(true);
    try {
      await onSave({
        ...initial,
        outcome,
        outcome_note: outcomeNote.trim() || null,
        components: [...new Set(components.split(",").map((c) => c.trim()).filter(Boolean))],
      });
    } catch (e) {
      setErr(String(e));
      setSaving(false);
    }
  };

  return (
    <form
      className="details-form"
      onSubmit={(e) => {
        e.preventDefault();
        void save();
      }}
      onKeyDown={(e) => {
        // Keep typing out of the app's shortcuts (j/k, /, Space…).
        e.stopPropagation();
        if (e.key === "Escape") onCancel();
      }}
    >
      <div className="details-row">
        <span className="details-label">Outcome</span>
        <div className="segmented" role="radiogroup" aria-label="Outcome">
          {OUTCOMES.map((o) => (
            <button type="button" key={o} role="radio" aria-checked={outcome === o} className={outcome === o ? "on" : ""} onClick={() => setOutcome(o)}>
              {OUTCOME_LABEL[o]}
            </button>
          ))}
        </div>
      </div>
      <label>
        Why
        <input value={outcomeNote} onChange={(e) => setOutcomeNote(e.target.value)} placeholder="Why it succeeded or didn't" autoFocus />
      </label>
      <label>
        Components
        <input value={components} onChange={(e) => setComponents(e.target.value)} placeholder="Paths or packages, comma-separated: packages/api, apps/web" />
      </label>
      {err && <div className="sheet-err">{err}</div>}
      <div className="details-actions">
        <button type="submit" className="primary" disabled={saving}>
          Save
        </button>
        <button type="button" className="ghost" onClick={onCancel}>
          Cancel
        </button>
      </div>
    </form>
  );
}

/** Kind and note for a new or edited annotation. */
export function AnnotationEditor({
  initial,
  onSave,
  onCancel,
}: {
  initial?: { kind: AnnotationKind; note: string };
  onSave: (kind: AnnotationKind, note: string) => Promise<void>;
  onCancel: () => void;
}) {
  const [kind, setKind] = useState<AnnotationKind>(initial?.kind ?? "other");
  const [note, setNote] = useState(initial?.note ?? "");
  const [err, setErr] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const save = async () => {
    // A second click or ⌘Enter while saving would create a second annotation.
    if (saving) return;
    setErr(null);
    setSaving(true);
    try {
      await onSave(kind, note.trim());
    } catch (e) {
      setErr(String(e));
    } finally {
      setSaving(false);
    }
  };
  return (
    <div
      className="annotation-editor"
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Escape") onCancel();
        if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) void save();
      }}
    >
      <Menu
        label="Annotation kind"
        value={kind}
        options={KINDS.map((k) => ({ key: k, label: KIND_LABEL[k] }))}
        onChange={setKind}
      />
      <textarea rows={2} value={note} onChange={(e) => setNote(e.target.value)} placeholder="What happened here (optional)" autoFocus />
      {err && <div className="sheet-err">{err}</div>}
      <div className="details-actions">
        <button className="primary" onClick={() => void save()} title="⌘Enter" disabled={saving}>
          Save
        </button>
        <button className="ghost" onClick={onCancel}>
          Cancel
        </button>
      </div>
    </div>
  );
}

const byLine = (a: Annotation) =>
  `${a.author.type === "model" ? a.author.model : (a.author.name ?? "You")} · ${fullDate(a.updated_at)}${a.updated_at !== a.created_at ? " (edited)" : ""}`;

/** One annotation, with edit and delete. */
export function AnnotationItem({
  annotation: a,
  revision,
  onSave,
  onDelete,
  onJump,
}: {
  annotation: Annotation;
  /** The transcript's current revision, to tell whether a model's annotation is out of date. */
  revision: string | null;
  onSave: (a: Annotation) => Promise<void>;
  onDelete: (a: Annotation) => Promise<void>;
  /** Shown in the panel: scrolls to the annotated event. */
  onJump?: (a: Annotation) => void;
}) {
  const [editing, setEditing] = useState(false);
  const unanchored = !a.anchors.some((r) => r.found);
  const stale = !!(a.revision && revision && a.revision !== revision);
  if (editing) {
    return (
      <AnnotationEditor
        initial={a}
        onCancel={() => setEditing(false)}
        onSave={async (kind, note) => {
          await onSave({ ...a, kind, note });
          setEditing(false);
        }}
      />
    );
  }
  return (
    <div className={`annotation kind-${a.kind}`}>
      <div className="annotation-head">
        <span className="annotation-kind">{KIND_LABEL[a.kind]}</span>
        {a.confidence && <span className="dim">{a.confidence} confidence</span>}
        {stale && <span className="stale" title="The transcript has changed since this was generated">out of date</span>}
        {unanchored && <span className="annotation-lost" title="The event it pointed at isn't in this transcript any more">event not found</span>}
        <span className="annotation-by">{byLine(a)}</span>
        {onJump && !unanchored && (
          <button className="link" onClick={() => onJump(a)}>
            Show
          </button>
        )}
        <button className="link" onClick={() => setEditing(true)}>
          Edit
        </button>
        <button
          className="link danger"
          onClick={async () => {
            if (await ask("Delete this annotation?", { title: "Delete annotation", kind: "warning", okLabel: "Delete" })) await onDelete(a);
          }}
        >
          Delete
        </button>
      </div>
      {a.note && <p className="annotation-note">{a.note}</p>}
      {a.rationale && <p className="annotation-rationale">Why: {a.rationale}</p>}
    </div>
  );
}

