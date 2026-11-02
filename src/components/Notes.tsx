import { useState } from "react";
import type { Annotation, AnnotationKind, Outcome, SessionMeta, SessionRow } from "../types";
import { KINDS, KIND_LABEL, OUTCOME_LABEL } from "../lib/notes";
import { fullDate } from "../lib/format";
import { Menu } from "./Menu";

const OUTCOMES = Object.keys(OUTCOME_LABEL) as Outcome[];

const blank = (key: string): SessionMeta => ({
  key,
  title: null,
  goal: null,
  context: null,
  outcome: "unknown",
  outcome_note: null,
  tags: [],
  links: [],
  updated_at: 0,
});

const text = (s: string) => (s.trim() ? s.trim() : null);

/**
 * The metadata people write about a session: what it was for, how it went,
 * tags and related issues or pull requests. Shown under the title, with a
 * form to edit it.
 */
export function SessionDetails({
  session,
  sessionKey,
  meta,
  onSave,
}: {
  session: SessionRow;
  sessionKey: string;
  meta: SessionMeta | null;
  onSave: (meta: SessionMeta) => Promise<void>;
}) {
  const [editing, setEditing] = useState(false);
  const has = !!meta;
  return (
    <div className="t-details">
      {meta?.title && <p className="t-original">Transcript title: {session.title}</p>}
      {has && !editing && (
        <dl className="details-view">
          {meta.goal && (
            <div>
              <dt>Goal</dt>
              <dd>{meta.goal}</dd>
            </div>
          )}
          {meta.context && (
            <div>
              <dt>Context</dt>
              <dd>{meta.context}</dd>
            </div>
          )}
          {(meta.outcome !== "unknown" || meta.outcome_note) && (
            <div>
              <dt>Outcome</dt>
              <dd>
                <span className={`outcome outcome-${meta.outcome}`}>{OUTCOME_LABEL[meta.outcome]}</span>
                {meta.outcome_note && <span> {meta.outcome_note}</span>}
              </dd>
            </div>
          )}
          {meta.tags.length > 0 && (
            <div>
              <dt>Tags</dt>
              <dd className="tags">
                {meta.tags.map((t) => (
                  <span key={t} className="tag">
                    {t}
                  </span>
                ))}
              </dd>
            </div>
          )}
          {meta.links.length > 0 && (
            <div>
              <dt>Links</dt>
              <dd className="links">
                {meta.links.map((l) => (
                  <a key={l.url} className="link" href={l.url} title={l.url}>
                    {l.label || l.url}
                  </a>
                ))}
              </dd>
            </div>
          )}
        </dl>
      )}
      {editing ? (
        <DetailsForm
          session={session}
          initial={meta ?? blank(sessionKey)}
          onCancel={() => setEditing(false)}
          onSave={async (m) => {
            await onSave(m);
            setEditing(false);
          }}
        />
      ) : (
        <button className="link details-edit" onClick={() => setEditing(true)}>
          {has ? "Edit details" : "Add details"}
        </button>
      )}
    </div>
  );
}

function DetailsForm({
  session,
  initial,
  onSave,
  onCancel,
}: {
  session: SessionRow;
  initial: SessionMeta;
  onSave: (meta: SessionMeta) => Promise<void>;
  onCancel: () => void;
}) {
  const [title, setTitle] = useState(initial.title ?? "");
  const [goal, setGoal] = useState(initial.goal ?? "");
  const [context, setContext] = useState(initial.context ?? "");
  const [outcome, setOutcome] = useState<Outcome>(initial.outcome);
  const [outcomeNote, setOutcomeNote] = useState(initial.outcome_note ?? "");
  const [tags, setTags] = useState(initial.tags.join(", "));
  // One link per line: the address, then an optional label.
  const [links, setLinks] = useState(initial.links.map((l) => (l.label ? `${l.url} ${l.label}` : l.url)).join("\n"));
  const [err, setErr] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const save = async () => {
    setErr(null);
    setSaving(true);
    try {
      await onSave({
        ...initial,
        title: text(title),
        goal: text(goal),
        context: text(context),
        outcome,
        outcome_note: text(outcomeNote),
        tags: [...new Set(tags.split(",").map((t) => t.trim()).filter(Boolean))],
        links: links
          .split("\n")
          .map((l) => l.trim())
          .filter(Boolean)
          .map((l) => {
            const [url, ...label] = l.split(/\s+/);
            return { url, label: label.join(" ") || null };
          }),
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
      <label>
        Title
        <input value={title} onChange={(e) => setTitle(e.target.value)} placeholder={session.title} autoFocus />
      </label>
      <label>
        Goal
        <textarea rows={2} value={goal} onChange={(e) => setGoal(e.target.value)} placeholder="The problem the session set out to solve" />
      </label>
      <label>
        Context
        <textarea rows={2} value={context} onChange={(e) => setContext(e.target.value)} placeholder="Background a reader needs" />
      </label>
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
        What happened
        <input value={outcomeNote} onChange={(e) => setOutcomeNote(e.target.value)} placeholder="Why it succeeded or didn't" />
      </label>
      <label>
        Tags
        <input value={tags} onChange={(e) => setTags(e.target.value)} placeholder="Comma-separated" />
      </label>
      <label>
        Issues and pull requests
        <textarea rows={2} value={links} onChange={(e) => setLinks(e.target.value)} placeholder="One per line: https://… then an optional label" />
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
  const save = async () => {
    setErr(null);
    try {
      await onSave(kind, note.trim());
    } catch (e) {
      setErr(String(e));
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
        <button className="primary" onClick={() => void save()} title="⌘Enter">
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
  onSave,
  onDelete,
  onJump,
}: {
  annotation: Annotation;
  onSave: (a: Annotation) => Promise<void>;
  onDelete: (a: Annotation) => Promise<void>;
  /** Shown in the panel: scrolls to the annotated event. */
  onJump?: (a: Annotation) => void;
}) {
  const [editing, setEditing] = useState(false);
  const unanchored = a.anchors.length > 0 && !a.anchors.some((r) => r.found);
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
        {a.author.type === "model" && <span className="tag">generated</span>}
        {onJump && a.anchors.length === 0 && <span className="dim">whole session</span>}
        {unanchored && <span className="annotation-lost" title="The event it pointed at isn't in this transcript any more">event not found</span>}
        <span className="annotation-by">{byLine(a)}</span>
        {onJump && !unanchored && a.anchors.length > 0 && (
          <button className="link" onClick={() => onJump(a)}>
            Show
          </button>
        )}
        <button className="link" onClick={() => setEditing(true)}>
          Edit
        </button>
        <button className="link danger" onClick={() => void onDelete(a)}>
          Delete
        </button>
      </div>
      {a.note && <p className="annotation-note">{a.note}</p>}
    </div>
  );
}

/** Every annotation on the session, filterable by kind, with a way to add a note about the whole session. */
export function AnnotationsPanel({
  annotations,
  onSave,
  onDelete,
  onJump,
  onAdd,
}: {
  annotations: Annotation[];
  onSave: (a: Annotation) => Promise<void>;
  onDelete: (a: Annotation) => Promise<void>;
  onJump: (a: Annotation) => void;
  onAdd: (kind: AnnotationKind, note: string) => Promise<void>;
}) {
  const [kind, setKind] = useState<AnnotationKind | "all">("all");
  const [adding, setAdding] = useState(false);
  const present = KINDS.filter((k) => annotations.some((a) => a.kind === k));
  // Deleting the last annotation of the chosen kind shows them all again.
  const active = kind !== "all" && !present.includes(kind) ? "all" : kind;
  const shown = active === "all" ? annotations : annotations.filter((a) => a.kind === active);
  return (
    <section className="annotations-panel">
      <div className="annotations-panel-head">
        {present.length > 1 && (
          <Menu
            label="Filter annotations"
            value={active}
            options={[{ key: "all" as const, label: "All kinds" }, ...present.map((k) => ({ key: k, label: KIND_LABEL[k] }))]}
            onChange={setKind}
          />
        )}
        <button className="link" onClick={() => setAdding(true)}>
          Add a note about the whole session
        </button>
      </div>
      {adding && (
        <AnnotationEditor
          onCancel={() => setAdding(false)}
          onSave={async (k, note) => {
            await onAdd(k, note);
            setAdding(false);
          }}
        />
      )}
      {shown.length === 0 && !adding && (
        <p className="dim">No annotations yet. Hover a message and click + in the margin, or open a tool call and click Annotate this call.</p>
      )}
      {shown.map((a) => (
        <AnnotationItem key={a.id} annotation={a} onSave={onSave} onDelete={onDelete} onJump={onJump} />
      ))}
    </section>
  );
}
