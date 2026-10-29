# Session Player

A native macOS app (Tauri 2 + React) for reading coding-agent sessions from **Claude Code**, **Codex** and **pi** in one place. Sessions are grouped by the directory they ran in, labeled with the best title the transcript offers, and indexed in SQLite so listing, sorting and full-text search stay instant.

## Run it

```bash
npm install
npm run app        # dev build with hot reload
npm run bundle     # release build → src-tauri/target/release/bundle/macos/Session Player.app
npm test           # Rust parser/indexer tests
```

On first launch the app adds whichever of these exist: `~/.claude/projects`, `~/.codex/sessions`, `~/.codex/archived_sessions`, `~/.pi/agent/sessions`. Settings also picks the appearance: System (follows macOS), Light or Dark. Add or remove folders in **Settings** (the gear in the bottom-left corner, **Session Player → Settings…**, or `⌘,`). Any folder works: every `.jsonl` file below it is checked, and its format is detected from its first lines.

## Releasing

`./build-and-publish.sh patch` (or `minor`, `major`, or an exact `x.y.z`) bumps the version with `npm version`, commits the bump, then builds a signed, notarized `.dmg` and publishes it as GitHub release `v<version>`. Without a bump argument it releases the current version. The version lives only in `package.json`; `src-tauri/tauri.conf.json` reads it from there. The script refuses to run with uncommitted changes, off `main`, or when the tag already exists. `--no-publish` stops after building and verifying. One-time setup (Developer ID certificate, notarization password in the keychain) is described at the top of the script.

## Using it

- **Rail**: toggle sources, pick a project. Project = the session's working directory; Claude Code worktrees (`<repo>/.claude/worktrees/*`) fold into their repo, and Codex worktrees (`~/.codex/worktrees/<hash>/<name>`) fold into the one project with the same folder name. A `YYYY-MM-DD` folder with a folder below it is dropped from the project path, so Codex's scheduled runs (`~/Documents/Codex/2026-10-28/<task>`, a new folder each day) show as one project, `~/Documents/Codex/<task>`. Text typed in the project filter (shown when there are more than 8 projects) also narrows **All projects** and its session list to the matching projects. Sort projects by recent activity (either direction), name (A–Z or Z–A), or by hand: drag a project to a new spot and the sort switches to **Manual**; projects that appear later go at the top. Hover a project and click **Hide** to drop it and its sessions from the list, the counts and search; unhide it from **Settings**.
- **Search** (`⌘K` or `/`): matches titles and paths instantly, plus full-text matches over prompts, replies and tool inputs (SQLite FTS5) with highlighted snippets. With a project selected, search starts scoped to it; switch between that project and **All projects** above the results. Clear with the × in the box, **Clear search**, or Esc. Opening a result carries the query into the transcript's find bar.
- **List**: `j`/`k` or arrow keys move through sessions; `?` (or the **?** button next to the gear) lists every keyboard shortcut. Sort by recent, oldest, most prompts or most tokens. Subagent runs, forks and pi branches are nested under the session they came from; click the "2 branches · 1 subagent" toggle to show them. Subagent sessions are left out unless you clear **Hide subagent sessions** in Settings, and **Only with errors** under the agent toggles keeps only sessions with an error (each row shows its error count). Changing what a list shows (project, project filter, agent toggles, **Only with errors**, sort, search) starts it at the top. Changes that happen under you (expanding a session, new sessions arriving, hiding or dragging a project) keep the top-most visible row in place and reflow the rest around it.
- **Transcript**: prompts, replies (Markdown), collapsed reasoning, and tool calls grouped into compact rows; click a row for its input and output. Edits render as diffs, `apply_patch` as a colored patch, and files attached to a prompt as collapsed attachment cards (click to show the raw contents): pi's `<file name="…">` blocks from `@file` arguments, Claude Code's `@file` attachments and IDE "opened file" notices, and Codex's "Files mentioned" / IDE context preamble (active file, selection, open tabs). The backend rewrites all of these into pi's `<file>` form (`parse::normalize_prompt`), so one renderer handles every agent. The ↑/↓ error arrows next to the find box step through errors, expanding each failed call. Selecting a session moves keyboard focus to its transcript: Space / Shift-Space page down and up, `]` / `[` jump to the next and previous prompt, `}` / `{` to the next and previous response. These keys, `j`/`k` in the list, and the error and find steppers all wrap from the end back to the start. `⌘F` finds within the session, Enter/Shift-Enter steps through matches. At ≥1500px wide a prompt outline appears on the right.
- **Errors**: an error is a tool call that couldn't do what was asked (an edit whose old text didn't match, a read past the end of a file, a timeout, a denied or sandboxed command, an unknown tool) or a harness or model/API error. A Codex call counts when its output opens with "<tool> failed" (`exec_command failed for …`). A shell command that ran and exited non-zero, such as a failing test run, is not an error: its row shows "exit N" instead. **Count failed shell commands as errors** in Settings adds those to the counts, the filter and the navigator, except exit 1 from a search or compare command (`grep`, `rg`, `diff`, `cmp`, `test`, `git diff`, `git grep`), which means "no match" or "differs", and 141, which means the output was cut short by a pipe such as `| head`. For those only the last command of the line is checked.
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
  components/                  SessionList (virtualized), Transcript (virtualized), SettingsSheet
  devBridge.ts                 synthetic backend for previewing the UI in a browser
```

- The index lives at `~/Library/Application Support/com.danmactough.sessionplayer/index.sqlite`, along with the list of folders and UI preferences. Bump `INDEX_VERSION` in `index.rs` after a parser change to force a rebuild, or use **Rebuild index** in Settings.
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
