// Dev-only stand-in for the Tauri backend so the UI can be previewed in a
// plain browser (`npm run dev`, then open http://localhost:1420). All data
// here is synthetic. Loaded from main.tsx only when not running inside Tauri.
import type { ActionResponse, ActionResponseInput, Annotation, AnnotationKind, Evidence, ImpactVerdict, LatestVerdict, Outcome, Report, SessionEvent, SessionMeta, SessionRow, Source } from "./types";
import { sessionKey } from "./lib/notes";

declare const __APP_VERSION__: string;
// One minor version ahead, so Settings › Check now can preview the update banner.
const NEXT_VERSION = __APP_VERSION__.replace(/^(\d+)\.(\d+)\.\d+.*/, (_, major, minor) => `${major}.${Number(minor) + 1}.0`);

const HOME = "/Users/demo";
const now = Date.now();
const H = 3_600_000;

const PROJECTS = [
  `${HOME}/code/orbit-api`,
  `${HOME}/code/field-notes`,
  `${HOME}/code/inkwell`,
  `${HOME}/dotfiles`,
  `${HOME}/code/tidepool/packages/server`,
];
const TITLES: [Source, string, string][] = [
  ["claude", "custom", "Migrate auth middleware to signed cookies"],
  ["codex", "thread", "Fix flaky pagination test"],
  ["pi", "prompt", "the health check is hammering /signin, can we exempt it from session middleware?"],
  ["claude", "ai", "Rate limiter sliding window"],
  ["codex", "prompt", "review the pending change, which applies db/schema.sql on boot"],
  ["claude", "prompt", "why does `npm test` hang after the last commit?"],
  ["pi", "name", "Plan v2 sync engine"],
  ["codex", "thread", "Add clickable hits column"],
  ["claude", "custom", "Dark mode for the editor"],
  ["pi", "prompt", "refactor the constants in lib/namespaces.js into a frozen object"],
  ["claude", "ai", "Upgrade to Node 24"],
  ["codex", "thread", "Plan #262 resolution"],
  ["claude", "prompt", "write integration tests for the export command"],
  ["pi", "prompt", "what is the status of @.scratch/v2-impl/"],
  ["claude", "custom", "Ship the release notes generator"],
  ["codex", "prompt", "update the README to match the current state of the app"],
  ["claude", "ai", "Investigate memory leak in watcher"],
  ["pi", "prompt", "Research strand B — transport and store split"],
];

const sessions: SessionRow[] = TITLES.map(([source, title_source, title], i) => {
  // Sessions 9 (a fork) and 17 (a subagent) belong to session 13.
  const project = PROJECTS[(i === 9 || i === 17 ? 13 : i) % PROJECTS.length];
  const updated = now - (i * i * 2.3 + i * 0.7) * H;
  const turns = 1 + ((i * 7) % 23);
  return {
    path: `${HOME}/.${source}/sessions/demo-${i}.jsonl`,
    source,
    native_id: `0000${i}-demo-${source}`,
    cwd: project,
    project,
    title,
    title_source,
    first_prompt: title_source === "prompt" ? title : `Context for: ${title.toLowerCase()}`,
    started_at: updated - turns * 0.2 * H,
    updated_at: updated,
    user_turns: turns,
    assistant_msgs: turns * 3,
    tool_calls: turns * 6,
    input_tokens: 12_000 * turns,
    output_tokens: 1_900 * turns,
    cache_read_tokens: 90_000 * turns,
    from_bundle: i === 5,
    models: i === 11 ? [] : [source === "claude" ? "claude-opus-5-5" : source === "codex" ? "gpt-5.4" : "glm-5.3-flash"],
    git_branch: i % 3 ? "main" : "feature/cookies",
    is_subagent: i === 17,
    parent_id: i === 17 || i === 9 ? "000013-demo-pi" : null,
    size: 40_000 * turns,
    imported_from: i === 11 ? "Claude Code" : null,
    // Session 13 has three alternate branches; b3 split from b2.
    branches:
      i === 13
        ? [
            { leaf: "b1", label: "Retry: what is the status of @.scratch/v2-impl/", started_at: updated - 3 * H, updated_at: updated - 2.5 * H, user_turns: 0, fork_turn: 1, fork_entry: "e0", parent_leaf: null },
            { leaf: "b2", label: "try the websocket approach instead", started_at: updated - 2 * H, updated_at: updated - H, user_turns: 3, fork_turn: 5, fork_entry: "e4", parent_leaf: null },
            { leaf: "b3", label: "now add reconnect backoff", started_at: updated - 1.5 * H, updated_at: updated - 0.5 * H, user_turns: 2, fork_turn: 7, fork_entry: "b2e1", parent_leaf: "b2" },
          ]
        : [],
    fork_entry: i === 9 ? "e2" : null,
    fork_turn: i === 9 ? 3 : null,
    // Every demo transcript has one failed Bash call per prompt.
    error_count: turns + 1,
    exit_count: 2,
  };
});

const IMG =
  "data:image/svg+xml;base64," +
  btoa(
    `<svg xmlns="http://www.w3.org/2000/svg" width="320" height="200"><rect width="320" height="200" fill="#ddd"/><text x="160" y="105" font-family="sans-serif" font-size="16" text-anchor="middle" fill="#555">screenshot</text></svg>`,
  );

function transcript(s: SessionRow, prefix = "e", turns = s.user_turns, setup = true): SessionEvent[] {
  const ev: SessionEvent[] = [];
  let t = s.started_at ?? now;
  const at = () => (t += 40_000);
  const tool = (name: string, input: unknown, out: string, err = false) => {
    const id = `call-${ev.length}`;
    ev.push({ kind: "tool_call", ts: at(), tool_name: name, call_id: id, text: typeof input === "string" ? input : JSON.stringify(input, null, 2) });
    ev.push({ kind: "tool_result", ts: at(), call_id: id, text: out, is_error: err });
  };
  if (setup) {
    ev.push({ kind: "context", ts: at(), text: "# Rules\n\n- Run `npm test` before committing.\n- Prefer small PRs.", context: { kind: "instructions", name: `${HOME}/.claude/CLAUDE.md`, scope: "user" } });
    ev.push({ kind: "context", ts: at(), text: "Use pnpm, not npm.", context: { kind: "instructions", name: `${s.cwd}/CLAUDE.local.md`, scope: "local" } });
    ev.push({ kind: "command", ts: at(), text: "/model opus" });
    ev.push({ kind: "system", ts: at(), text: "Set model to opus" });
  }
  for (let turn = 0; turn < turns; turn++) {
    const entry = `${prefix}${turn}`;
    const first = ev.length;
    ev.push({
      kind: "user",
      ts: at(),
      text: turn === 0 && s.is_subagent
        ? `<file name="/var/folders/3h/demo/T/pi-subagent-6esdB2/task.md">\nTask: You are a delegated subagent running from a fork of the parent session.\n\n## Task\n\nImplement **issues 1–4** from the plan:\n\n- CLI skeleton\n- rendered Markdown display\n- \`changed-on-disk\` detection\n</file>`
        : turn === 0 && s.source === "codex"
          ? `<file name="apps/server/scripts/import-project.ts" note="active file · selection">parseClaudeSessionFile</file>\n<file name="/var/folders/3h/demo/T/codex-clipboard-0940ce70.png" note="image"></file>\n<open_tabs>apps/server/scripts/import-project.ts\nAGENTS.md\nscripts/tsconfig.json</open_tabs>\n\n${s.first_prompt ?? s.title}`
          : turn === 0 ? s.first_prompt ?? s.title : `Turn ${turn + 1}: looks good, now handle the edge case where the cookie is missing.\nAlso keep the old header path working for a release.`,
      images: turn === 1 ? [IMG] : undefined,
    });
    ev.push({ kind: "thinking", ts: at(), text: "The user wants the middleware to skip the health check.\n\nI should look at how sessions are wired before changing anything." });
    ev.push({ kind: "assistant", ts: at(), text: "I'll check how the session middleware is mounted first." });
    tool("Bash", { command: "rg -n \"session\\(\" src/ | head -20", description: "Find session middleware" }, "src/app.ts:41:app.use(session(opts))\nsrc/app.ts:88:  // session(opts) is mounted before routes");
    tool("Read", { file_path: `${s.cwd}/src/app.ts` }, Array.from({ length: 70 }, (_, i) => `${i + 1}\tconst line${i} = ${i};`).join("\n"));
    tool(
      "Edit",
      { file_path: `${s.cwd}/src/app.ts`, old_string: "app.use(session(opts));\napp.get('/signin', signin);", new_string: "app.use(unless(['/healthz'], session(opts)));\napp.get('/healthz', (_, res) => res.send('ok'));\napp.get('/signin', signin);" },
      "The file has been updated.",
    );
    tool("Bash", { command: "npm test -- auth.spec.ts" }, "FAIL auth.spec.ts\n  ● rejects missing cookie\n    expected 401, got 200", true);
    if (s.source === "codex") {
      tool("apply_patch", "*** Begin Patch\n*** Update File: src/auth.ts\n@@\n-  if (!cookie) return next();\n+  if (!cookie) return res.sendStatus(401);\n*** End Patch", "Success. Updated the following files:\nM src/auth.ts");
    }
    ev.push({
      kind: "assistant",
      ts: at(),
      text:
        "The health check now bypasses the session store. Summary:\n\n- `/healthz` is mounted **before** `session()`\n- Missing cookies return `401`\n\n| Route | Session | Status |\n|---|---|---|\n| `/healthz` | no | 200 |\n| `/signin` | yes | 200 |\n\n```ts\napp.use(unless(['/healthz'], session(opts)));\n```\n\nNext: run `npm test` and confirm the auth suite passes.",
    });
    if (turn === 3) ev.push({ kind: "compaction", ts: at(), text: "## Summary\nWe moved the health check ahead of the session middleware and fixed the 401 path." });
    if (turn === 2) ev.push({ kind: "system", ts: at(), text: "Interrupted by user" });
    for (const e of ev.slice(first)) e.entry = entry;
  }
  // A one-line error notice at the very end, like pi's aborted-operation message.
  ev.push({ kind: "system", ts: at(), text: "This operation was aborted", is_error: true });
  return ev;
}

const devMeta = new Map<string, SessionMeta>();
/** Saved reports, newest first. */
let devReports: Report[] = [];
const devResponses = new Map<string, ActionResponse>();

/** A report built from the synthetic notes, shaped like the backend's. */
function devReport(picked: { key: string; title: string }[]): Report {
  const sessions = picked.map(({ key, title }) => {
    const meta = devMeta.get(key);
    return {
      key,
      title,
      outcome: meta?.outcome ?? ("unknown" as Outcome),
      outcome_note: meta?.outcome_note ?? null,
      components: meta?.components ?? [],
      annotations: devNotes.filter((a) => a.key === key).map((a) => ({ id: a.id, version: a.version ?? 1 })),
    };
  });
  const cited = devNotes.filter((a) => sessions.some((s) => s.key === a.key));
  const impact: ImpactVerdict[] = [...devResponses.values()]
    .filter((r) => r.status === "done")
    .flatMap((r) => {
      const earlier = devReports.find((d) => d.id === r.report_id);
      if (!earlier || !sessions.length) return [];
      return [{
        action: { report_id: r.report_id, action_index: r.action_index },
        action_title: earlier.actions[r.action_index].title,
        verdict: "did_not_work" as const,
        reasoning: "The same failing test command still shows up after this was marked done.",
        evidence: [{ type: "session" as const, key: sessions[0].key }],
      }];
    });
  const evidence = (kinds: AnnotationKind[]): Evidence[] =>
    cited.filter((a) => kinds.includes(a.kind)).map((a) => ({ type: "annotation", id: a.id, version: a.version ?? 1, key: a.key }));
  const outcomes: Partial<Record<Outcome, number>> = {};
  for (const s of sessions) outcomes[s.outcome] = (outcomes[s.outcome] ?? 0) + 1;
  const kinds: Partial<Record<AnnotationKind, number>> = {};
  for (const a of cited) kinds[a.kind] = (kinds[a.kind] ?? 0) + 1;
  return {
    id: `dev-report-${Date.now()}`,
    created_at: Date.now(),
    title: "Tool errors and missing context slow sessions down",
    model: "claude-sonnet-5-5",
    prompt_version: 1,
    sessions,
    stats: { sessions: sessions.length, outcomes, kinds, annotations: cited.length },
    summary: "Most of these sessions lost time to failing test runs the agent retried instead of reading. Two went wrong because the agent didn't know how the project runs its tests.",
    takeaways: [
      { text: "The agent reruns a failing test command several times before reading its output.", evidence: evidence(["tool_error"]) },
      { text: "The agent doesn't know the project runs its tests with npm run test:unit, and guesses npm test instead.", evidence: evidence(["missing_context", "ignored_instruction"]) },
    ].filter((t) => t.evidence.length),
    actions: [
      { title: "Document the test setup", category: "instructions" as const, priority: "high" as const, change: "Add a **Testing** section to `AGENTS.md` that names `npm run test:unit` and says to read a failure before rerunning.", success_check: "Notes about guessing the test command stop appearing.", evidence: evidence(["missing_context", "ignored_instruction"]) },
      { title: "Script running the changed tests", category: "automation" as const, priority: "medium" as const, change: "Add a `scripts/test-changed.sh` that runs only the tests for changed files and prints the first failure.", evidence: evidence(["tool_error", "agent_stuck"]) },
    ].filter((a) => a.evidence.length),
    impact,
    rejected: 0,
  };
}

/** Sessions being "analyzed", and whether each was cancelled. */
const devGenerating = new Map<string, boolean>();
let devNotes: Annotation[] = [];

// A few analyzed sessions, so outcome badges, the note filters and the
// Statistics and Insights views have something to show.
const SEEDED: [number, Outcome, AnnotationKind[]][] = [
  [0, "succeeded", ["key_decision", "tool_error"]],
  [1, "failed", ["agent_stuck", "agent_stuck", "tool_error"]],
  [2, "partial", ["ignored_instruction", "missing_context"]],
  [3, "succeeded", ["tool_error"]],
  [6, "failed", ["wrong_assumption", "human_frustration", "human_intervention"]],
  [7, "succeeded", ["automatable"]],
  [10, "partial", ["environment", "tool_error", "tool_error"]],
];
for (const [i, outcome, kinds] of SEEDED) {
  const key = sessionKey(sessions[i]);
  const generated_by = { model: "claude-haiku-5-5", at: now - H, revision: "dev" };
  devMeta.set(key, { key, outcome, outcome_note: null, components: [], updated_at: now - H, generated: ["outcome"], generated_by });
  kinds.forEach((kind, j) => {
    const byPerson = j === 0 && i % 2 === 0;
    devNotes.push({
      id: `dev-seed-${i}-${j}`,
      version: 1,
      key,
      kind,
      note: `Synthetic ${kind.replace(/_/g, " ")} note.`,
      anchors: [{ anchor: `e${j + 1}`, kind: "assistant", ts: null }],
      author: byPerson ? { type: "person", name: "Demo" } : { type: "model", model: "claude-haiku-5-5" },
      created_at: now - H,
      updated_at: now - H,
      confidence: byPerson ? undefined : (["high", "medium", "low"] as const)[(i + j) % 3],
    });
  });
}

type Handler = (args: Record<string, unknown>) => unknown;
const handlers: Record<string, Handler> = {
  list_sessions: () => sessions,
  search: ({ query }) =>
    sessions
      .filter((s) => s.title.toLowerCase().includes(String(query).split(" ")[0].toLowerCase()))
      .map((s) => ({ path: s.path, snippet: `… the \u0001${query}\u0002 path in ${s.title} …`, rank: -1 })),
  load_session: ({ path, branch }) => {
    const s = sessions.find((x) => x.path === path)!;
    if (!branch) return transcript(s).map((e, i) => ({ ...e, anchor: `e${i}` }));
    const b = s.branches.find((x) => x.leaf === branch)!;
    return transcript({ ...s, first_prompt: b.label }, `${branch}e`, Math.max(1, b.user_turns), false);
  },
  get_roots: () => [
    { path: `${HOME}/.claude/projects`, exists: true, sessions: 7 },
    { path: `${HOME}/.codex/sessions`, exists: true, sessions: 5 },
    { path: `${HOME}/.pi/agent/sessions`, exists: true, sessions: 6 },
  ],
  suggested_roots: () => [`${HOME}/.codex/archived_sessions`],
  home_dir: () => HOME,
  get_setting: () => null,
  set_setting: () => null,
  add_root: () => null,
  remove_root: () => null,
  reindex: () => null,
  reveal: () => null,
  open_targets: ({ source, nativeId }) => [
    ...(source === "claude" ? [{ id: "claude", label: "Open in Claude", url: "claude://code/continue?session=local_demo" }] : []),
    ...(source === "codex" ? [{ id: "chatgpt", label: "Open in ChatGPT", url: `codex://threads/${nativeId}` }] : []),
    { id: "t3", label: "Open in T3 Code", url: `t3code://threads/demo/${nativeId}` },
  ],
  open_in_app: ({ url }) => console.log("open", url),
  export_preview: ({ path }) => {
    const s = sessions.find((x) => x.path === path)!;
    return {
      token: 1,
      summary: {
        bytes: 48_213,
        file_name: `${s.title}.spsession`,
        sessions: [
          {
            id: `${s.source}:${s.native_id}`,
            title: s.title,
            is_subagent: false,
            prompts: s.user_turns,
            events: s.user_turns * 9,
            has_details: false,
            annotations: 0,
            context: [
              { kind: "instructions", name: `${HOME}/.claude/CLAUDE.md` },
              { kind: "instructions", name: `${s.cwd}/CLAUDE.local.md` },
            ],
          },
        ],
        omitted: [],
        attachments: [
          { id: "a0", session: `${s.source}:${s.native_id}`, anchor: "e3", kind: "image", name: null, mime: "image/png", bytes: 182_331, status: "included", reason: null },
          { id: "a1", session: `${s.source}:${s.native_id}`, anchor: "e7", kind: "file", name: `${s.cwd}/package.json`, mime: null, bytes: null, status: "omitted", reason: "the agent recorded only the path, not the contents" },
        ],
      },
      subagents: [],
    };
  },
  export_save: ({ dest }) => dest,
  export_cancel: () => null,
  remove_imported: () => null,
  // Pretends to analyze in two parts over about two seconds, then records an outcome.
  generate_notes: ({ key, focus }) => {
    console.log("dev analyze focus:", focus);
    const k = String(key);
    if (devGenerating.has(k)) throw new Error("Already generating notes for this session");
    devGenerating.set(k, false);
    const emit = (event: string, payload: unknown) => listeners[event]?.forEach((id) => callbacks[id]?.({ event, id, payload }));
    setTimeout(() => emit("generate-progress", { key: k, done: 1, total: 2 }), 900);
    setTimeout(() => {
      const cancelled = devGenerating.get(k);
      devGenerating.delete(k);
      if (!cancelled) {
        const generated_by = { model: "claude-haiku-5-5", at: Date.now(), revision: "dev" };
        devMeta.set(k, { key: k, outcome: "succeeded", outcome_note: null, components: [], updated_at: Date.now(), generated: ["outcome"], generated_by });
      }
      emit("generate-done", { key: k, error: null, cancelled, rejected: 0 });
    }, 1800);
    return null;
  },
  cancel_generate: ({ key }) => {
    if (devGenerating.has(String(key))) devGenerating.set(String(key), true);
    return null;
  },
  generating: () => [...devGenerating.keys()],
  open_analyze_log: () => {
    console.log("open analyze log");
    return null;
  },
  default_prompts: () => ({
    analyze:
      "The analysis is for a team that studies how coding agents are used. Mark the moments that explain how the session went: where the agent went wrong or got stuck, where the user had to step in, and the decisions that shaped the result. Prefer a few well-supported annotations over many weak ones.",
    report:
      "The report is for a developer who wants to improve how their coding agents work. Look for problems that recur across sessions and the changes that would stop them recurring. Prefer a few well-supported takeaways and actions over many weak ones, and don't invent problems the notes don't show.",
  }),
  // Pretends to write a report in about two seconds.
  generate_report: ({ sessions, focus }) => {
    console.log("dev report focus:", focus);
    const job = `dev-job-${Date.now()}`;
    const emit = (event: string, payload: unknown) => listeners[event]?.forEach((id) => callbacks[id]?.({ event, id, payload }));
    setTimeout(() => emit("report-progress", { job, done: 0, total: 1 }), 100);
    setTimeout(() => {
      const report = devReport(sessions as { key: string; title: string }[]);
      if (typeof focus === "string") report.focus = focus;
      devReports = [report, ...devReports];
      emit("report-done", { job, report: report.id, error: null, cancelled: false });
    }, 2000);
    return job;
  },
  cancel_report: () => null,
  list_reports: () =>
    devReports.map((r) => ({ id: r.id, created_at: r.created_at, title: r.title, model: r.model, sessions: r.sessions.length, actions: r.actions.length })),
  get_report: ({ id }) => devReports.find((r) => r.id === id) ?? null,
  report_annotations: ({ id }) =>
    (devReports.find((r) => r.id === id)?.sessions ?? []).flatMap((s) =>
      s.annotations.flatMap(({ id }) => {
        const a = devNotes.find((n) => n.id === id);
        return a ? [{ annotation: a, current_version: a.version ?? 1 }] : [];
      }),
    ),
  save_markdown: ({ dest, text }) => {
    console.log("save markdown to", dest, String(text).length, "chars");
    return dest;
  },
  action_responses: ({ id }) => [...devResponses.values()].filter((r) => r.report_id === id).sort((a, b) => a.action_index - b.action_index),
  set_action_response: ({ input }) => {
    const i = input as ActionResponseInput;
    const k = `${i.report_id}:${i.action_index}`;
    const note = i.note?.trim() || null;
    if (!i.status && !note) {
      devResponses.delete(k);
      return null;
    }
    const done = i.status === "done";
    const saved: ActionResponse = { ...i, note, user_impact: done ? i.user_impact : null, done_at: done ? (devResponses.get(k)?.done_at ?? Date.now()) : null, updated_at: Date.now() };
    devResponses.set(k, saved);
    return saved;
  },
  action_verdicts: ({ id }) => {
    const latest = new Map<number, LatestVerdict>();
    for (const r of [...devReports].reverse())
      for (const v of r.impact)
        if (v.action.report_id === id)
          latest.set(v.action.action_index, { action_index: v.action.action_index, verdict: v.verdict, reasoning: v.reasoning, report_id: r.id, report_title: r.title, created_at: r.created_at });
    return [...latest.values()];
  },
  delete_report: ({ id }) => {
    devReports = devReports.filter((r) => r.id !== id);
    return null;
  },
  session_notes: ({ key }) => ({ revision: devMeta.get(String(key))?.generated_by ? "dev" : null, meta: devMeta.get(String(key)) ?? null, annotations: devNotes.filter((a) => a.key === key).map((a) => ({ ...a, anchors: a.anchors.map((r) => ({ ...r, found: true })) })) }),
  save_meta: ({ meta }) => {
    const m = { ...(meta as SessionMeta), updated_at: Date.now() };
    devMeta.set(m.key, m);
    return m;
  },
  save_annotation: ({ annotation }) => {
    const a = annotation as Annotation;
    const saved = a.id ? { ...a, version: (a.version ?? 0) + 1, updated_at: Date.now() } : { ...a, id: `dev-${Date.now()}`, version: 1, created_at: Date.now(), updated_at: Date.now() };
    devNotes = [...devNotes.filter((x) => x.id !== saved.id), saved];
    return saved;
  },
  delete_annotation: ({ id }) => {
    devNotes = devNotes.filter((x) => x.id !== id);
    return null;
  },
  note_summaries: () => [...new Set([...devMeta.keys(), ...devNotes.map((a) => a.key)])].map((key) => ({
    key,
    outcome: devMeta.get(key)?.outcome ?? "unknown",
    components: devMeta.get(key)?.components ?? [],
    kinds: [...new Set(devNotes.filter((a) => a.key === key).map((a) => a.kind))],
    kind_counts: devNotes.filter((a) => a.key === key).reduce<Record<string, number>>((c, a) => ({ ...c, [a.kind]: (c[a.kind] ?? 0) + 1 }), {}),
    person_annotations: devNotes.filter((a) => a.key === key && a.author.type === "person").length,
    model_annotations: devNotes.filter((a) => a.key === key && a.author.type === "model").length,
    confidence: devNotes
      .filter((a) => a.key === key && a.confidence)
      .reduce<Record<string, number>>((c, a) => ({ ...c, [a.confidence!]: (c[a.confidence!] ?? 0) + 1 }), {}),
    analyzed: !!devMeta.get(key)?.generated_by || devNotes.some((a) => a.key === key && a.author.type === "model"),
  })),
  library_errors: () => [{ path: `${HOME}/Library/Application Support/com.danmactough.sessionplayer/library/Y2xhdWRlOmRlbW8.spsession`, error: "Y2xhdWRlOmRlbW8.spsession is damaged: bundle is malformed" }],
  import_bundle: () => ({ imported: ["claude:demo"], unchanged: [], kept: [], conflicts: [], empty: [], kept_notes: 0 }),
  terminal_apps: () => [
    { id: "terminal", label: "Terminal" },
    { id: "iterm", label: "iTerm" },
    { id: "ghostty", label: "Ghostty" },
    { id: "cmux", label: "cmux" },
  ],
  open_in_terminal: ({ app }) => console.log("open in", app),
  log_frontend: ({ level, message }) => console.log(`[${level}]`, message),
  "plugin:event|listen": ({ event, handler }) => {
    (listeners[event as string] ??= []).push(handler as number);
    return handler;
  },
  "plugin:event|unlisten": () => null,
  // Confirmation dialogs (such as Analyze's consent) answer yes, so flows can be previewed.
  // Shows the dialog in the page, since the browser has no native one with custom buttons.
  // Set window.__dialogAnswer to a button's index to answer without showing it (for scripted checks).
  "plugin:dialog|message": ({ message, title, buttons }) => {
    const b = buttons as { OkCustom?: string; OkCancelCustom?: [string, string]; YesNoCancelCustom?: [string, string, string] } | string | undefined;
    const labels =
      typeof b === "object"
        ? (b.YesNoCancelCustom ?? b.OkCancelCustom ?? (b.OkCustom ? [b.OkCustom] : ["OK"]))
        : b === "YesNo"
          ? ["Yes", "No"]
          : b === "YesNoCancel"
            ? ["Yes", "No", "Cancel"]
            : b === "OkCancel"
              ? ["Ok", "Cancel"]
              : ["Ok"];
    const scripted = (window as { __dialogAnswer?: number }).__dialogAnswer;
    if (scripted !== undefined) return labels[Math.min(scripted, labels.length - 1)];
    return devDialog(String(title ?? ""), String(message), labels);
  },
  "plugin:app|version": () => __APP_VERSION__,
  // Settings > Check now always finds this release, so the update banner can be previewed.
  "plugin:updater|check": () => ({ rid: 1, currentVersion: __APP_VERSION__, version: NEXT_VERSION, date: null, body: "", rawJson: {} }),
  "plugin:updater|download": () => 2,
  "plugin:updater|install": () => console.log("install update"),
  "plugin:process|restart": () => console.log("restart"),
  "plugin:resources|close": () => null,
};

/** A plain in-page stand-in for the native message dialog; resolves to the label clicked. */
function devDialog(title: string, message: string, labels: string[]): Promise<string> {
  return new Promise((resolve) => {
    const back = document.createElement("div");
    back.style.cssText = "position:fixed;inset:0;z-index:9999;display:flex;align-items:center;justify-content:center;background:rgba(0,0,0,.25)";
    const box = document.createElement("div");
    box.style.cssText = "width:340px;padding:18px;border-radius:10px;background:var(--raised);color:var(--ink);box-shadow:0 12px 40px rgba(0,0,0,.25);font:13px var(--sans)";
    const h = document.createElement("div");
    h.style.cssText = "font-weight:600;margin-bottom:6px";
    h.textContent = title;
    const p = document.createElement("div");
    p.style.cssText = "color:var(--ink-2);line-height:1.45;margin-bottom:14px";
    p.textContent = message;
    const row = document.createElement("div");
    row.style.cssText = "display:flex;flex-direction:column;gap:6px";
    labels.forEach((label, i) => {
      const btn = document.createElement("button");
      btn.textContent = label;
      btn.className = i === 0 ? "primary" : "ghost";
      btn.style.cssText = "padding:5px 10px;border-radius:6px;border:1px solid var(--line)";
      btn.onclick = () => {
        back.remove();
        resolve(label);
      };
      row.append(btn);
    });
    box.append(h, p, row);
    back.append(box);
    document.body.append(back);
  });
}

let cb = 0;
const callbacks: Record<number, (e: unknown) => void> = {};
const listeners: Record<string, number[]> = {};
Object.assign(window, {
  // Fires an app event from the console, e.g. __emit("index-done", { elapsed_ms: 391, … }).
  __emit: (event: string, payload: unknown) => listeners[event]?.forEach((id) => callbacks[id]?.({ event, id, payload })),
  __TAURI_INTERNALS__: {
    transformCallback: (f: (e: unknown) => void) => {
      callbacks[++cb] = f;
      return cb;
    },
    invoke: async (cmd: string, args: Record<string, unknown> = {}) => {
      const h = handlers[cmd];
      if (!h) throw new Error(`devBridge: no handler for ${cmd}`);
      return h(args);
    },
  },
  __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => {} },
});
