# Session Player

A native macOS app (Tauri 2 + React) for reading coding-agent sessions from **Claude Code**, **Codex** and **pi** in one place. Sessions are grouped by the directory they ran in, labeled with the best title the transcript offers, and indexed in SQLite so listing, sorting and full-text search stay instant.

## Run it

```bash
npm install
npm run app        # dev build with hot reload
npm run bundle     # release build → src-tauri/target/release/bundle/macos/Session Player.app
npm test           # Rust parser/indexer tests
```

On first launch the app adds whichever of these exist: `~/.claude/projects`, `~/.codex/sessions`, `~/.codex/archived_sessions`, `~/.pi/agent/sessions`. Add or remove folders from **Sources** in the bottom-left corner. Any folder works: every `.jsonl` file below it is checked, and its format is detected from its first lines.

## Using it

- **Rail**: toggle sources, pick a project. Project = the session's working directory; Claude Code worktrees (`<repo>/.claude/worktrees/*`) fold into their repo, and Codex worktrees (`~/.codex/worktrees/<hash>/<name>`) fold into the one project with the same folder name.
- **Search** (`⌘K` or `/`): matches titles and paths instantly, plus full-text matches over prompts, replies and tool inputs (SQLite FTS5) with highlighted snippets. With a project selected, search starts scoped to it; switch between that project and **All projects** above the results. Clear with the × in the box, **Clear search**, or Esc. Opening a result carries the query into the transcript's find bar.
- **List**: `j`/`k` or arrow keys move through sessions. Sort by recent, oldest, most prompts or most tokens. Subagent runs, forks and pi branches are nested under the session they came from; click the "2 branches · 1 subagent" toggle to show them. Subagents are hidden unless **Subagents** is on, and **With errors** keeps only sessions with a failed tool call or a model/API error (each row shows its error count). When filters change or a session expands, the top-most visible row stays put and the rest of the list reflows around it.
- **Transcript**: prompts, replies (Markdown), collapsed reasoning, and tool calls grouped into compact rows; click a row for its input and output. Edits render as diffs, `apply_patch` as a colored patch, and files attached to a prompt as collapsed attachment cards (click to show the raw contents): pi's `<file name="…">` blocks from `@file` arguments, Claude Code's `@file` attachments and IDE "opened file" notices, and Codex's "Files mentioned" / IDE context preamble (active file, selection, open tabs). The backend rewrites all of these into pi's `<file>` form (`parse::normalize_prompt`), so one renderer handles every agent. The ↑/↓ error arrows next to the find box step through failed tool calls and error notices, expanding each failed call. A Codex call counts as failed on a non-zero exit code or output that opens with "<tool> failed" (`exec_command failed for …`). `⌘F` finds within the session, Enter/Shift-Enter steps through matches. At ≥1500px wide a prompt outline appears on the right.
- **Actions**: copy a resume command (`claude --resume`, `codex resume`, `pi --session`), reveal the file in Finder, jump between a subagent and its parent.

The index updates on its own: a file watcher re-indexes changed files about 1.5s after an agent writes to them.

## How labels are chosen

| Source | First choice | Fallbacks |
|---|---|---|
| Claude Code | `custom-title` (renamed by you) | `ai-title`, `agent-name`, `summary` record |
| Codex | `thread_name` from `~/.codex/session_index.jsonl` | — |
| pi | `session_info` name | — |

When none exist, the label comes from the first prompt that has at least three words (within the first three prompts), so a session that opens with "add" or "hooks" gets a more descriptive label. Slash-command markup becomes `/command args`, injected context (`<environment_context>`, `<system-reminder>`, inlined pi skills) is stripped, and titles that are just a branch name are ignored.

## Layout

```
src-tauri/src/
  parse/{claude,codex,pi}.rs   one parser per format → normalized events
  index.rs                     scan, incremental re-index (mtime+size), SQLite + FTS5
  lib.rs                       Tauri commands, background indexing, file watcher, spimg:// image protocol
src/
  App.tsx                      rail, filters, search, keyboard navigation
  components/                  SessionList (virtualized), Transcript (virtualized), SourcesSheet
  devBridge.ts                 synthetic backend for previewing the UI in a browser
```

- The index lives at `~/Library/Application Support/com.danmactough.sessionplayer/index.sqlite`, along with the list of folders and UI preferences. Bump `INDEX_VERSION` in `index.rs` after a parser change to force a rebuild, or use **Rebuild index** in Sources.
- Transcripts are re-parsed from the source file when opened (tens of ms even for 30 MB files). Embedded screenshots are served through a `spimg://` protocol instead of the IPC payload, so they load only when scrolled into view.
- pi stores sessions as a tree. The session's own row shows the branch that ends at the last entry. Every other branch with at least one message is listed under the branch it split from (branches can have branches), and pi fork files are listed under their parent session. A branch or fork transcript starts at its split point with a link back to it; the source transcript shows a "splits off here" marker at that point, linking forward. Token totals count every branch.

### Previewing the UI in a browser

`npm run dev` and open http://localhost:1420. Outside Tauri, `src/devBridge.ts` stands in for the backend with synthetic sessions, which makes layout and styling work possible without the native shell.

### Checking the parsers against your own data

```bash
cd src-tauri
cargo test --release -- --ignored --nocapture index_local_sessions   # index real folders into /tmp, print every label
cargo test --release -- --ignored --nocapture parse_largest_sessions  # timing + payload size for the biggest files
```
