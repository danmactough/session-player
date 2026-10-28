// Dev-only stand-in for the Tauri backend so the UI can be previewed in a
// plain browser (`npm run dev`, then open http://localhost:1420). All data
// here is synthetic. Loaded from main.tsx only when not running inside Tauri.
import type { SessionEvent, SessionRow, Source } from "./types";

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
  const project = PROJECTS[i % PROJECTS.length];
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
    models: [source === "claude" ? "claude-opus-5-5" : source === "codex" ? "gpt-5.4" : "glm-5.3-flash"],
    git_branch: i % 3 ? "main" : "feature/cookies",
    is_subagent: i === 17,
    parent_id: i === 17 ? "000013-demo-pi" : null,
    size: 40_000 * turns,
  };
});

const IMG =
  "data:image/svg+xml;base64," +
  btoa(
    `<svg xmlns="http://www.w3.org/2000/svg" width="320" height="200"><rect width="320" height="200" fill="#ddd"/><text x="160" y="105" font-family="sans-serif" font-size="16" text-anchor="middle" fill="#555">screenshot</text></svg>`,
  );

function transcript(s: SessionRow): SessionEvent[] {
  const ev: SessionEvent[] = [];
  let t = s.started_at ?? now;
  const at = () => (t += 40_000);
  const tool = (name: string, input: unknown, out: string, err = false) => {
    const id = `call-${ev.length}`;
    ev.push({ kind: "tool_call", ts: at(), tool_name: name, call_id: id, text: typeof input === "string" ? input : JSON.stringify(input, null, 2) });
    ev.push({ kind: "tool_result", ts: at(), call_id: id, text: out, is_error: err });
  };
  ev.push({ kind: "command", ts: at(), text: "/model opus" });
  ev.push({ kind: "system", ts: at(), text: "Set model to opus" });
  for (let turn = 0; turn < s.user_turns; turn++) {
    ev.push({
      kind: "user",
      ts: at(),
      text: turn === 0 ? s.first_prompt ?? s.title : `Turn ${turn + 1}: looks good, now handle the edge case where the cookie is missing.\nAlso keep the old header path working for a release.`,
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
  }
  return ev;
}

type Handler = (args: Record<string, unknown>) => unknown;
const handlers: Record<string, Handler> = {
  list_sessions: () => sessions,
  search: ({ query }) =>
    sessions
      .filter((s) => s.title.toLowerCase().includes(String(query).split(" ")[0].toLowerCase()))
      .map((s) => ({ path: s.path, snippet: `… the \u0001${query}\u0002 path in ${s.title} …`, rank: -1 })),
  load_session: ({ path }) => transcript(sessions.find((s) => s.path === path)!),
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
  log_frontend: ({ level, message }) => console.log(`[${level}]`, message),
  "plugin:event|listen": () => 1,
  "plugin:event|unlisten": () => null,
};

let cb = 0;
Object.assign(window, {
  __TAURI_INTERNALS__: {
    transformCallback: () => ++cb,
    invoke: async (cmd: string, args: Record<string, unknown> = {}) => {
      const h = handlers[cmd];
      if (!h) throw new Error(`devBridge: no handler for ${cmd}`);
      return h(args);
    },
  },
  __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => {} },
});
