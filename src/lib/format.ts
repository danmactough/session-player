import type { SessionRow, Source } from "../types";

let home = "";
export function setHome(h: string) {
  home = h.replace(/\/$/, "");
}

export function tildify(p: string | null | undefined): string {
  if (!p) return "";
  if (home && (p === home || p.startsWith(home + "/"))) return "~" + p.slice(home.length);
  return p;
}

export function basename(p: string): string {
  const parts = p.replace(/\/$/, "").split("/");
  return parts[parts.length - 1] || p;
}

export function dirname(p: string): string {
  const t = tildify(p);
  const i = t.lastIndexOf("/");
  return i > 0 ? t.slice(0, i) : "";
}

export const SOURCE_LABEL: Record<Source, string> = {
  claude: "Claude Code",
  codex: "Codex",
  pi: "pi",
};

export function relTime(ms: number | null | undefined, now = Date.now()): string {
  if (!ms) return "—";
  const s = Math.round((now - ms) / 1000);
  if (s < 45) return "just now";
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h}h ago`;
  const d = Math.round(h / 24);
  if (d < 7) return `${d}d ago`;
  const date = new Date(ms);
  const sameYear = date.getFullYear() === new Date(now).getFullYear();
  return date.toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    ...(sameYear ? {} : { year: "numeric" }),
  });
}

export function fullDate(ms: number | null | undefined): string {
  if (!ms) return "—";
  return new Date(ms).toLocaleString(undefined, {
    weekday: "short",
    month: "short",
    day: "numeric",
    year: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

export function clock(ms: number | null | undefined): string {
  if (!ms) return "";
  return new Date(ms).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
}

export function duration(ms: number): string {
  if (!isFinite(ms) || ms <= 0) return "—";
  const m = Math.round(ms / 60000);
  if (m < 1) return "<1m";
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 48) return `${h}h ${m % 60}m`;
  return `${Math.round(h / 24)}d`;
}

export function compact(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${(n / 1000).toFixed(n < 10_000 ? 1 : 0)}k`;
  return `${(n / 1_000_000).toFixed(n < 10_000_000 ? 1 : 0)}M`;
}

export function bytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${Math.round(n / 1024)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

/** Buckets for the date dividers in the session list. */
export function dayBucket(ms: number | null, now = new Date()): string {
  if (!ms) return "Undated";
  const d = new Date(ms);
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const day = 86_400_000;
  if (ms >= startOfToday) return "Today";
  if (ms >= startOfToday - day) return "Yesterday";
  if (ms >= startOfToday - 6 * day) return "This week";
  if (ms >= startOfToday - 29 * day) return "Earlier this month";
  return d.toLocaleDateString(undefined, {
    month: "long",
    ...(d.getFullYear() === now.getFullYear() ? {} : { year: "numeric" }),
  });
}

export function resumeCommand(s: SessionRow): string {
  const cd = s.cwd ? `cd ${shellQuote(s.cwd)} && ` : "";
  switch (s.source) {
    case "claude":
      return `${cd}claude --resume ${s.native_id}`;
    case "codex":
      return `${cd}codex resume ${s.native_id}`;
    case "pi":
      return `${cd}pi --session ${shellQuote(s.path)}`;
  }
}

function shellQuote(s: string): string {
  return /^[\w@%+=:,./-]+$/.test(s) ? s : `'${s.replace(/'/g, `'\\''`)}'`;
}

/** A short, human description of what a tool call did, e.g. the command or file. */
export function toolSummary(name: string | undefined, input: string): string {
  let obj: Record<string, unknown> | null = null;
  try {
    const parsed = JSON.parse(input);
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) obj = parsed;
  } catch {
    // Codex custom tools (apply_patch) take raw text.
  }
  if (!obj) {
    const m = input.match(/\*\*\* (?:Update|Add|Delete) File: (.+)/);
    if (m) return tildify(m[1].trim());
    return firstLine(input);
  }
  const pick = (...keys: string[]) => {
    for (const k of keys) {
      const v = obj![k];
      if (typeof v === "string" && v.trim()) return v;
      if (Array.isArray(v) && v.length && typeof v[0] === "string") return v.join(" ");
    }
    return undefined;
  };
  const n = (name || "").toLowerCase();
  const path = pick("file_path", "path", "notebook_path", "filePath", "file");
  if (path && /read|write|edit|notebook|ls/.test(n)) return tildify(path);
  const cmd = pick("command", "cmd");
  if (cmd) return firstLine(cmd);
  const q = pick("pattern", "query", "url", "prompt", "description", "skill", "action", "subject");
  if (q) return firstLine(q);
  if (path) return tildify(path);
  for (const v of Object.values(obj)) {
    if (typeof v === "string" && v.trim()) return firstLine(v);
  }
  return "";
}

function firstLine(s: string): string {
  const line = s.trim().split("\n")[0];
  return line.length > 160 ? line.slice(0, 160) + "…" : line;
}

/** Normalizes tool names across agents so the UI can style them consistently. */
export function toolLabel(name: string | undefined): string {
  if (!name) return "tool";
  if (name.startsWith("mcp__")) {
    const parts = name.split("__");
    return parts[parts.length - 1];
  }
  return name;
}
