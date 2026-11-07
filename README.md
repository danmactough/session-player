<p align="center">
  <img src="design/icon.png" width="120" alt="Session Player icon">
</p>

<h1 align="center">Session Player</h1>

<p align="center">
  A Mac app for reading your coding sessions from all your agents in one place.
</p>

<p align="center">
  <a href="https://github.com/danmactough/session-player/releases/latest"><strong>Download</strong></a>
  &nbsp;·&nbsp;
  <a href="#usage">Usage</a>
  &nbsp;·&nbsp;
  <a href="#development">Development</a>
</p>

## Contents

| Usage | Development |
|---|---|
| [Install](#install)<br>[Add your sessions](#add-your-sessions)<br>[Find a session](#find-a-session)<br>[Search](#search)<br>[Read a session](#read-a-session)<br>[Spot errors](#spot-errors)<br>[Continue a session](#continue-a-session)<br>[Add notes and analysis](#add-notes-and-analysis)<br>[Share a session](#share-a-session)<br>[Settings](#settings)<br>[Keyboard shortcuts](#keyboard-shortcuts) | [Run it locally](#run-it-locally)<br>[Test](#test)<br>[Build](#build)<br>[Release](#release)<br>[Add support for a new agent](#add-support-for-a-new-agent)<br>[How it works](#how-it-works) |

## Usage

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/images/overview-dark.png">
  <img src="docs/images/overview-light.png" alt="Session Player showing agent and project filters in the left sidebar, a list of sessions in the middle, and a session's outcome, annotations and transcript on the right">
</picture>

A few words used below:

| Word | Meaning |
|---|---|
| **Agent** | A coding tool. Session Player reads sessions from Claude Code, Codex and pi. |
| **Session** | One conversation with an agent. |
| **Transcript** | The full record of a session. |
| **Project** | The folder a session ran in. |
| **Subagent** | A helper session that an agent starts to handle part of its work. |
| **Fork** | A copy of a session, made to continue in a new direction while the original stays as it was. |
| **Branch** | In pi, another path through the same session, made by going back to an earlier message and continuing from there. |
| **Btw** | A quick side question asked with `/btw` during a session. Session Player doesn't show Codex `/btw` conversations because they are not saved anywhere. |

### Install

1. Download the latest `.dmg` from [Releases](https://github.com/danmactough/session-player/releases/latest). It runs on Macs with Apple silicon.
2. Open the `.dmg` and drag Session Player to Applications.

### Add your sessions

There's usually nothing to do. The first time it opens, the app adds each of these folders that exists:

- `~/.claude/projects`
- `~/.codex/sessions`
- `~/.codex/archived_sessions`
- `~/.pi/agent/sessions`

To add another folder, open **Settings** (`⌘,`) and click **Add folder…**. The app finds session files anywhere inside it and works out which agent wrote each one.

New sessions and new messages appear on their own while the app is open.

### Find a session

The left sidebar narrows down the session list next to it.

- **Show one agent or several**: click the agent buttons at the top of the sidebar. To show only one agent, hover over its button and click **only**, or `⌥`-click the button.
- **Show one project**: click it. Click it again, or press Esc in the list, to go back to **All projects**.
- **Show several projects**: `⌘`-click adds or removes a project, `⇧`-click selects a run of them, and `⌘⇧`-click adds a run.
- **Find a project**: type in the project filter, which appears when you have more than 8 projects. The session list narrows to match.
- **Order projects**: choose **Recent**, **Oldest**, **A–Z** or **Z–A**, or drag a project to a new spot, which switches the order to **Manual**. New projects appear at the top of a manual order.
- **Hide a project**: hover over it and click **Hide**. Its sessions leave the list, the counts and search. To bring it back, use **Settings**.

Copies of a repository that Claude Code or Codex made to work in (git worktrees) are grouped with that repository. Chats in the Codex app that aren't in a project are grouped as one project, `~/Documents/Codex`.

In the session list:

- Move between sessions with `j` and `k` or the arrow keys.
- Sort by **Recent**, **Oldest**, **Most prompts** or **Most tokens**.
- Click a toggle such as "2 branches · 1 subagent" to show the sessions that came from that one: subagents, forks and pi branches.
- Subagent sessions are hidden until you clear **Hide subagent sessions** in **Settings**.
- Click **Only with errors** to keep only sessions with an error. Each session shows how many it has.

Each session's title is the one the agent gave it. If there isn't one, the title comes from an early prompt.

### Search

Press `⌘K` or `/`, then type. Search looks through titles, folder paths, prompts, replies, what the agent passed to its tools, and your notes.

If you've selected projects, search starts within them. Above the results, you can switch to **All projects**. To clear the search, press Esc or click the × in the box.

Opening a result opens the session with your search already in its find bar.

### Read a session

Click a session to read its transcript:

- **Prompts and replies** appear in order, with reasoning folded away.
- **Tool calls** are grouped into compact rows. Click a row to see what went in and what came out. File edits appear as colored before-and-after changes.
- **Files attached to a prompt** appear as folded cards. Click one to read it.
- **Instructions the agent was given**, such as `CLAUDE.md`, `AGENTS.md`, skills and agent definitions, appear as folded blocks where the agent received them. Each is marked **user**, **repo** or **local** to show where it came from. Clear the **Instructions** checkbox to hide them.
- **A ChatGPT chat that started a Codex task** appears as a folded **ChatGPT conversation** block above the request.

Press `⌘F` to find text in the session. In a window wider than 1500 pixels, an outline of the prompts appears on the right. To make more room, the fold button in the margin under the title hides the first prompt, the analysis and the session stats.

### Spot errors

An error is a tool call that couldn't do what was asked, or a failure in the agent or the model. For example: an edit that didn't apply, a timeout or a denied command.

A command that ran and failed, such as a failing test run, isn't counted. Its row shows "exit 1" or whichever code it returned. To count these too, turn on **Count failed shell commands as errors** in **Settings**.

The ↑ and ↓ error arrows next to the find box step through a session's errors and open each one.

### Continue a session

The **⋯** button at the top right of a transcript holds its actions. The first one either copies the command that resumes the session or opens it in another app; choose which under **Resume or open with**, then click it or press `⌘O` to run it. Your choice is remembered for each agent. The same menu has **Export…**, **Reveal file** and **Copy path**, and **Remove…** for an imported session.

| Action | Available for |
|---|---|
| **Copy resume command** | All sessions on this Mac |
| **Open in Claude** | Claude Code sessions in `~/.claude/projects`, if Claude is installed |
| **Open in ChatGPT** | Codex sessions in `~/.codex`, if ChatGPT is installed |
| **Open in T3 Code** | Sessions T3 Code ran or imported, if T3 Code is installed |
| **Open in Terminal** | All sessions on this Mac. Choose from the installed terminals: Terminal, iTerm, Ghostty, and cmux |

macOS asks once for permission to control each terminal. You can also show a session's file in Finder, and jump between a subagent and the session that started it.

### Add notes and analysis

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/images/annotation-dark.png">
  <img src="docs/images/annotation-light.png" alt="A transcript scrolled to an annotated reply, with a Key decision note shown directly below it and arrows for stepping through the session's annotations">
</picture>

Each session can record how it went. Use **Analyze…** in the session list's **⋯** Actions menu (`⌘Y`) to have Claude Haiku fill this in, or add it yourself:

- **Outcome**: succeeded, partial, failed or unknown, with the reason.
- **Components**: the parts of the codebase it worked on, such as `packages/api`.
- **Annotations**: your notes on particular moments.

**Add an annotation**: hover over a message and click **+** in the margin, or open a tool call and click **Annotate this call**. Pick a type, then add a note.

**Use them**: the list under the transcript's title shows every annotation, and **Show** jumps to its place in the transcript. The ↑ and ↓ annotation arrows step through them. Use the **Filter** menu under the session list's heading to find matching sessions: hover **Analyzed**, **Outcome** or **Annotation type** and tick the options you want. You can tick several options in each group, and a session must match every group. The button shows how many options are ticked, and the **×** beside it clears them all. The transcript's annotation list has its own **Annotation type** filter.

**Views**: the icons down the window's left edge switch between **Transcripts** (reading sessions), **Statistics** and **Insights**; the gear and the question mark at the bottom open Settings and the keyboard shortcuts.

**See them across sessions**: click **Statistics**. Its counts cover the sessions the list shows, or only the sessions you select: how many were analyzed or annotated, how many have each outcome, how many annotations of each type there are, who wrote them, and how confident Analyze was. Click an outcome or annotation type to narrow the list to it, and click it again to clear that filter.

Each view keeps its own projects, filters, search, expanded rows and scroll position, so switching back to **Transcripts** finds the list and the open transcript exactly as you left them. Transcripts always has one session open.

**Analyze sessions**: choose **Analyze…** from the **⋯** Actions button above the session list, or press `⌘Y`. The open session (in Transcripts) or the sessions already selected (in Statistics) start selected. Change the selection as in Insights: click one, ⌘-click to add or remove one, ⇧-click a run, or ⌘A for every session the list shows under its filters and search. **Filter › Analyzed › Not analyzed** narrows the list to sessions not analyzed yet. Click **Analyze • (n)** in the bar under the list to queue them and run them one at a time, or **Cancel** (or Esc) to leave without analyzing. When some of them were analyzed before, the app asks whether to **Analyze Again** or analyze them **Only If Changed**, which skips a session whose transcript hasn't changed since its last analysis. While the queue runs, a spinner and "Analyzing 3/10" show in the small tray at the bottom of the left column, where "Indexed" appears. Click it to see each session's result; × cancels one and **Cancel all** stops the rest. When every session is done, the tray says "Analyze · Done", or how many failed, until you open it and click **Clear**. The queue isn't kept when the app quits, but the Analyze log is: **Log** at the top of the queue, **Open log** under a report that failed, and **Settings › Annotations › Open Log** all open it. It records each Analyze and report run and each call to `claude`, with `claude`'s own error output when one fails.

**Customize what Analyze and reports look for**: the **Analyze** and **Get insights** buttons each have a chevron on the right. Its **Customize prompt…** opens the text that tells the model what to focus on, such as the kinds of problems to look for or who the report is for. Edit it and click **Analyze** or **Get insights** to run once with your text. Tick **Set as default** to keep it, so the plain button uses it from then on; **Restore built-in** puts back the app's own text. Only this focus can change: the rules about citing evidence and the shape of the reply stay the same. A report written with your own text says **Custom prompt** under its title; click it to read the text.

**Select sessions in Statistics**: click a session to count only it, ⌘-click to add or remove one, ⇧-click to select a run, or ⌘A for every session in the list. Press Esc or click an empty part of the list to count the whole list again.

**Write a report**: click **Insights**. Under **New Insight Report**, click **Create**; the button then reads **Select**. Choose sessions in the list that appears (click, ⌘-click, ⇧-click or ⌘A for all; the Filter menu and **Recent** / **Oldest** sort work as elsewhere). Once some are chosen the button reads **Get insights**: click it. Sessions that haven't been analyzed are analyzed first (the report list shows "Analyzing N sessions first"), then the report is written on all of them. Tick **Also re-analyze sessions that already have an analysis** to analyze every chosen session again first; the app remembers that choice. A model (Claude Sonnet) reads their outcomes, components and annotations and writes a summary, key takeaways and recommended actions: changes to the codebase, skills, prompts and instruction files, the environment, or work to automate. Insights lists saved reports under **Build a report**, sorted by **Recent** or **Oldest**; a report being written shows there with its progress, and opens when it's done. A report you open fills the space to the right. Each takeaway and action lists what it rests on: click one to see the annotation or outcome as the report saw it (marked if it has been edited or deleted since), and **Open in Transcripts** to go to that session. A report never changes once written; write another to include newer notes. The **⋯** button at the top right of a report has **Copy as Markdown** and **Save as .md…** to export it, and **Delete…** to remove it.

**Change the outcome or components**: click **Edit**.

**Analyze** needs Claude Code installed. Before it sends anything, it shows a notice that transcripts go to Anthropic, with **OK** and **Cancel**. Tick **Don't show this again** and click OK to stop seeing it; **Settings › Annotations** can turn it back on. Everything Analyze writes can be edited or deleted. Running it again replaces the outcome and components, including ones you wrote or emptied, and its own annotations that nobody has edited; annotations people wrote or edited are kept.

These notes are stored in the app, not in the agent's files.

### Share a session

**Send**: open the session, click the **⋯** button at the top right of the transcript, choose **Export…**, and leave ticked any subagents you want to include. Before saving, the app lists exactly what goes into the file and what stays out. It saves a `.spsession` file that anyone with Session Player can import. Exporting needs your name: if Settings doesn't have one yet, the export sheet asks for it. The file records your name, and the recipient sees it as the author of annotations you wrote before you set a name.

**Receive**: choose **Import…** from the **⋯** Actions button above the session list, or press `⌘⇧I`. Imported sessions are marked "imported". They can be read, searched and exported, but not resumed. To delete one, choose **Remove…** from the **⋯** menu at the top right of its transcript.

### Settings

Open **Settings** with `⌘,`, the gear at the bottom of the window's left edge, or **Session Player → Settings…**.

### Keyboard shortcuts

Press `?`, or click the question mark under the gear at the bottom of the window's left edge, to see every keyboard shortcut.

---

## Development

Session Player is a Tauri 2 app with a Rust backend and a React frontend.

### Run it locally

You need Node.js (CI uses version 24), Rust 1.90 or newer, and the Xcode Command Line Tools.

```bash
npm install
npm run app
```

`npm run app` starts a dev build with hot reload.

To work on layout and styling without the native shell, run `npm run dev` and open http://localhost:1420. Outside Tauri, `src/devBridge.ts` stands in for the backend with synthetic sessions.

### Test

```bash
npm run test:core   # parsers, indexing, annotations, bundles
npm run test:app    # the Tauri command layer
npm run lint        # the frontend
```

`npm test` runs only the same tests as `npm run test:app`.

To check the parsers against the sessions on your own Mac:

```bash
cd src-tauri
cargo test --release -p session-player-core --lib -- --ignored --nocapture index_local_sessions   # index real folders into /tmp, print every label
cargo test --release -p session-player-core --lib -- --ignored --nocapture parse_largest_sessions  # timing + payload size for the biggest files
```

CI runs the tests and the linter on every push to `main` and every pull request. The macOS build runs only when someone comments `/build` on a pull request.

### Build

```bash
npm run bundle
```

The app is written to `src-tauri/target/release/bundle/macos/Session Player.app`.

### Release

```bash
./build-and-publish.sh patch
```

Use `patch`, `minor`, `major` or an exact `x.y.z`. The script bumps the version with `npm version`, commits the bump, builds a signed and notarized `.dmg`, and publishes it as GitHub release `v<version>`.

- Without a bump argument, it releases the current version.
- `--no-publish` stops after building and verifying.
- It refuses to run with uncommitted changes, off `main`, or when the tag already exists.
- The version lives only in `package.json`. `src-tauri/tauri.conf.json` reads it from there.

The one-time setup (Developer ID certificate, notarization password in the keychain) is described at the top of the script.

### Add support for a new agent

Supporting a new agent means teaching the backend to read its session files, then adding it to each place the app names an agent.

1. **Write the session log parser.** Add `src-tauri/core/src/parse/<agent>.rs` with a `parse(path)` function that returns a `Parsed` (`src-tauri/core/src/model.rs`). It turns each log line into an `Event`: prompt, reply, reasoning, tool call, tool result and so on. It also fills in the session's ID, folder, title, models and token counts. Optional features depend on what the agent records: subagent links (`is_subagent`, `parent_id`), context documents (`context_event`), and errors (`is_error`, `exit_code`). If the agent attaches files to prompts in its own format, convert them in `parse::normalize_prompt`.
2. **Register the parser.** Add the agent to `Source` and `Source::as_str` in `src-tauri/core/src/model.rs`. In `src-tauri/core/src/parse/mod.rs`, teach `detect` to recognize the agent's files from their first 8 lines, and add it to `parse_file`.
3. **Find its sessions.** Add its session folder to `default_roots` in `src-tauri/core/src/index.rs`. If it keeps titles or other details outside the session file, read them in `enrich`. If it runs sessions in worktrees, fold them into their repository in `project_of`.
4. **Add the sidebar filter and icon.**
   - `src/types.ts`: add it to `Source`.
   - `src/App.tsx`: add it to `ALL_SOURCES`, which creates the button at the top left, and to the zero counts.
   - `src/components/ProjectNav.tsx`: add it to `SOURCES`, which draws the colored dots on each project.
   - `src/lib/format.ts`: add its name to `SOURCE_LABEL`.
   - `src/components/AppIcon.tsx`: add its glyph to `GLYPHS` and `GLYPH_FOR`.
   - `src/index.css`: add a `--<agent>` color in all three theme blocks, the `.src-<agent>` rules, and the `.glyph-<agent>` rule.
5. **Add the resume command.** Change both `resumeCommand` in `src/lib/format.ts` and `resume_command` in `src-tauri/src/terminals.rs`, and keep the two the same. If the agent has a desktop app, add it to `targets` and `SCHEMES` in `src-tauri/src/apps.rs`, and to `HOME_APP` in `src/components/OpenButton.tsx`.
6. **Update the text that lists the agents.** These are the note in `src/components/SettingsSheet.tsx`, the `SYSTEM` prompt in `src-tauri/core/src/generate.rs`, `source` in `docs/bundle-format.md`, the sample data in `src/devBridge.ts`, and this README.
7. **Add tests.** Add a small synthetic log to `src-tauri/core/tests/fixtures` and test it in `src-tauri/core/tests/parse.rs`, then check the parser against real sessions with the commands under [Test](#test).

The compilers catch some of these changes: `cargo build` reports missing arms in `as_str` and `parse_file`, and `npm run build` reports a missing `SOURCE_LABEL` entry, zero count or `resumeCommand` case. They don't catch the lists and string matches, such as `ALL_SOURCES`, `SOURCES`, `GLYPH_FOR` and `resume_command`.

### How it works

#### Layout

```
src-tauri/core/src/            no Tauri dependency, so its tests need no GUI libraries
  parse/{claude,codex,pi}.rs   one parser per format → normalized events
  index.rs                     scan, incremental re-index (mtime+size), SQLite + FTS5
  bundle.rs                    export/import of .spsession bundles, the imported-session library
  annotations.rs               session metadata and annotations, kept apart from the index tables
  generate.rs                  suggesting metadata and annotations with Claude Haiku via the claude CLI
  reports.rs                   reports on sets of sessions: takeaways and recommended actions, kept for good
src-tauri/src/
  lib.rs                       Tauri commands, background indexing, file watcher, spimg:// image protocol
  apps.rs                      desktop apps that can open a session
  terminals.rs                 terminals that can run a resume command
src/
  App.tsx                      sidebar, filters, search, keyboard navigation
  components/                  SessionList (virtualized), Transcript (virtualized), SettingsSheet
  devBridge.ts                 synthetic backend for previewing the UI in a browser
```

#### Index

- The index lives at `~/Library/Application Support/com.danmactough.sessionplayer/index.sqlite`, along with the list of folders and UI preferences. The Analyze log is `~/Library/Logs/com.danmactough.sessionplayer/analyze.log`, moved to `analyze.log.1` when it passes about 2 MB. Bump `INDEX_VERSION` in `index.rs` after a parser change to force a rebuild.
- Every `.jsonl` or `.jsonl.zst` file below a session folder is checked, and its format is detected from its first lines. Codex's compressed logs are streamed through Zstandard when indexed or opened, including full transcripts retained after its paginated-history migration.
- A file watcher re-indexes changed files about 1.5s after an agent writes to them.
- Search uses SQLite FTS5 full-text search over prompts, replies and tool inputs.
- Annotations and analysis are stored in the app's database, apart from the index tables, and survive **Rebuild index**.
- Each change to an annotation saves a new version, and a saved version never changes, so a report can cite one. A version is kept only while it is the current one or a report cites it. Deleting an annotation that a report cites keeps the cited versions for the report and hides the annotation everywhere else.

#### Titles

| Source | First choice | Fallbacks |
|---|---|---|
| Claude Code | `custom-title` (renamed by you) | `ai-title`, `agent-name`, `summary` record |
| Codex | `thread_name` from `~/.codex/session_index.jsonl` | — |
| pi | `session_info` name | — |

When none exist, the label comes from the first prompt that has at least three words (within the first three prompts), so a session that opens with "add" or "hooks" gets a more descriptive label. After that come the first slash command and then a subagent's type. Slash-command markup becomes `/command args`, and context the agent injected into a prompt (`<environment_context>`, `<system-reminder>`, inlined pi skills) is stripped.

#### Projects

- Claude Code worktrees (`<repo>/.claude/worktrees/*`) and `<repo>/.worktrees/*` fold into their repository.
- Codex worktrees (`~/.codex/worktrees/<hash>/<name>`) fold into the one project with the same folder name.
- The Codex app gives each chat outside a project its own folder (`~/Documents/Codex/2026-10-28/<task>`). Those chats are found from the app's list of projectless threads in `~/.codex/.codex-global-state.json` or by working inside that folder, and show as one project, `~/Documents/Codex` (or the folder set as `projectless-workspace-root`).

#### Session list

Changing what a list shows (project, project filter, agent toggles, **Only with errors**, sort, search) starts it at the top. Changes that happen under you (expanding a session, new sessions arriving, hiding or dragging a project) keep the top-most visible row in place and reflow the rest around it.

#### Transcripts

- Transcripts are re-parsed from the source file when opened (tens of ms even for 30 MB files). Embedded screenshots are served through a `spimg://` protocol instead of the IPC payload, so they load only when scrolled into view.
- Attachment cards come from pi's `<file name="…">` blocks from `@file` arguments, Claude Code's `@file` attachments and IDE "opened file" notices, and Codex's "Files mentioned" / IDE context preamble (active file, selection, open tabs). The backend rewrites all of these into pi's `<file>` form (`parse::normalize_prompt`), so one renderer handles every agent.
- `apply_patch` renders as a colored patch.
- Codex's code mode, where the model writes a JavaScript `exec` script that calls the real tools, shows the commands, file edits and MCP calls each script made (from Codex's completed-item records) instead of the script. A script that called no tools, or failed on its own, stays.
- Codex's guardian review subagent, which only approves or refuses the main agent's riskier actions, isn't listed.
- pi stores sessions as a tree. The session's own row shows the branch that ends at the last entry. Every other branch with at least one message is listed under the branch it split from (branches can have branches), and pi fork files are listed under their parent session. A branch or fork transcript starts at its split point with a link back to it; the source transcript shows a "splits off here" marker at that point, linking forward. Token totals count every branch.

#### Instructions shown in a transcript

- They come from Claude Code's instruction attachments, skill messages and subagent logs, Codex's AGENTS.md and skill messages, and pi's skills and its subagent runs' agents and project instructions. A document sent again unchanged later in the session says so.
- Each shows its file name (a skill shows its folder's name). Claude Code records the scope. For Codex and pi, a file inside the session's folder counts as the repo's, and any other file as the user's.
- pi doesn't record the system prompt of a main session, so its AGENTS.md isn't shown there. Claude Code's own system prompt isn't included.
- Exports carry them, and the export sheet lists each one.

#### Errors

- An error is an edit whose old text didn't match, a read past the end of a file, a timeout, a denied or sandboxed command, an unknown tool, or a harness or model/API error. A Codex call counts when its output opens with "<tool> failed" (`exec_command failed for …`).
- **Count failed shell commands as errors** adds non-zero exits to the counts, the filter and the navigator, except exit 1 from a search or compare command (`grep`, `rg`, `diff`, `cmp`, `test`, `git diff`, `git grep`), which means "no match" or "differs", and 141, which means the output was cut short by a pipe such as `| head`. For those only the last command of the line is checked.

#### Opening sessions in other apps

Until you pick an action, the button uses the agent's own app.

- **Claude**: a session it already ran opens with `claude://code/continue?session=local_…` (from its records in `~/Library/Application Support/Claude/claude-code-sessions`), and any other is imported with `claude://resume?session=<id>`.
- **ChatGPT**: `codex://threads/<id>`.
- **T3 Code**: `t3code://threads/<environment>/<thread>`, from `~/.t3/userdata/statev2.sqlite`.
- **Terminals**: the chosen one opens a new window (a new workspace in cmux) and runs the resume command there.
- **Icons**: agents and apps use the glyphs T3 Code uses for them (Claude, OpenAI for Codex and ChatGPT, pi, T3).

#### Analyze

- **Analyze** asks Claude Haiku for the outcome, components and annotations. It runs the `claude` command from Claude Code with no tools and the transcript passed as data.
- It also sends the context the agent was given, each document once (up to 20,000 characters each and 60,000 in all), so it can tell when the agent ignored an instruction.
- It runs with the variables your login shell exports and the `env` block of your Claude settings (`~/.claude/settings.json` and `settings.local.json`, or under `$CLAUDE_CONFIG_DIR`). When those name credentials of their own (`ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, an `apiKeyHelper`, or Bedrock or Vertex), Analyze uses only them, so an LLM gateway such as LiteLLM gets its base URL, token, `ANTHROPIC_CUSTOM_HEADERS` and model mappings. Otherwise it uses your Claude Code sign-in.
- A field you empty stays empty until you fill it in yourself. Results generated from an earlier version of a growing transcript are marked as out of date. An annotation whose event isn't in the transcript any more is marked "event not found", not dropped.
- Long sessions are sent in parts and the results merged. Analysis runs in the background and can be cancelled.

#### Reports

- A report reads the outcomes, components and annotations of a set of sessions (not their titles or transcripts) and writes a title, a summary, key takeaways and recommended actions. Each action has a category (codebase, skill, instructions, environment or automation), a priority and the change to make.
- It runs the `claude` command the same way Analyze does, with Claude Sonnet unless another model is chosen. Sessions are numbered `s1`…, and annotations `a1`…, so every takeaway and action cites what it rests on; one citing nothing it was given is dropped. Large sets are sent in groups of whole sessions and the results merged.
- Reports are saved in the app's database for good and never change. Each records the version of every annotation it was given, so editing or deleting an annotation later doesn't change what the report shows. Deleting a report releases versions no other report cites.
- Code: `src-tauri/core/src/reports.rs`.

#### Bundles

- The export sheet lists the sessions, the related sessions left out (the parent of a subagent, other pi branches, unticked subagents), and every attachment, with screenshots included and file cards whose contents weren't recorded marked as omitted. Saving writes the bundle shown, unchanged.
- Export downloads images that replies link from the web into the bundle, and imported transcripts never load images from the web.
- Importing a session already in the library with different contents asks whether to replace or keep the library's copy.
- The file format and the inclusion rules are in [docs/bundle-format.md](docs/bundle-format.md).
