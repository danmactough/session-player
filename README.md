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
| [Install](#install)<br>[Choose a server](#choose-a-server)<br>[Add your sessions](#add-your-sessions)<br>[Find a session](#find-a-session)<br>[Search](#search)<br>[Read a session](#read-a-session)<br>[Continue a session](#continue-a-session)<br>[Add notes](#add-notes)<br>[Analyze sessions](#analyze-sessions)<br>[See patterns across sessions](#see-patterns-across-sessions)<br>[Get Insights](#get-insights)<br>[Share a session](#share-a-session)<br>[Use the command line](#use-the-command-line)<br>[Use it in a browser](#use-it-in-a-browser)<br>[Settings](#settings)<br>[Keyboard shortcuts](#keyboard-shortcuts) | [Run it locally](#run-it-locally)<br>[Test](#test)<br>[Build](#build)<br>[Release](#release)<br>[Add support for a new agent](#add-support-for-a-new-agent)<br>[How it works](#how-it-works)<br>[HTTP API](docs/api.md)<br>[License](#license) |

## Usage

<img src="docs/images/overview-light.png" alt="Session Player showing view icons on the left edge, agent and project filters beside them, a list of sessions in the middle, and a session's outcome, annotations and transcript on the right">

The icons on the window's left edge switch between three views: **Transcripts** for reading sessions, **Statistics** for counts across them, and **Insights** for reports.

### Install

Download the latest `.dmg` from [Releases](https://github.com/danmactough/session-player/releases/latest) and drag Session Player to Applications. It runs on Macs with Apple silicon.

### Choose a server

The app reads your sessions through a server, `sp serve`, which keeps running in the background so indexing, analysis and reports go on with the window closed. The first time the app opens, it asks which server to use:

- **This computer.** The app installs the server as a login item for your user (a LaunchAgent; no administrator password), starts it, and connects. If it is installed but stopped, the app starts it. The server listens only on this computer until you pair another device with it.
- **A paired server.** Run `sp pair` on the other computer and paste the string it prints. The app then reads that computer's sessions. See [Work from another machine](#work-from-another-machine).

The app shows one server at a time. To switch, open **Settings**. After an update moves the app, it rewrites the login item to use the new copy of `sp`.

### Add your sessions

Session Player finds sessions in the usual folders for Claude Code, Codex and pi on its own, and shows new ones as they're written. To add another folder, open **Settings** (`⌘,`).

### Find a session

The sidebar narrows the list by agent and project; `⌘`-click to choose several. **Only with errors** keeps sessions that hit an error. Subagents, forks and pi branches are grouped under the session they came from.

### Search

Press `⌘K` to search titles, prompts, replies, tool calls and your notes.

### Read a session

Click a session to read it: prompts and replies, tool calls with what went in and came out, file edits as before-and-after changes, and the instruction files the agent was given, such as `CLAUDE.md`. Errors, such as an edit that didn't apply or a denied command, are marked, and the arrows by the find box step through them.

### Continue a session

The **⋯** menu at the top right of a transcript copies the command that resumes the session, or opens it in Claude, ChatGPT, T3 Code or your terminal (`⌘O`).

### Add notes

<img src="docs/images/annotation-light.png" alt="A transcript scrolled to an annotated reply, with a Key decision note shown directly below it and arrows for stepping through the session's annotations">

Each session can record its **outcome** (succeeded, partial, failed or unknown), the **components** of the codebase it touched, and **annotations** on particular moments, such as a tool error or a key decision. Hover over a message and click **+** to add one.

### Analyze sessions

Analyze has Claude fill in the outcome, components and annotations for you. Choose **Analyze…** from the **⋯** menu above the session list (`⌘Y`), pick sessions, and click **Analyze**. It needs Claude Code installed.

### See patterns across sessions

<img src="docs/images/statistics-light.png" alt="The Statistics view, with counts of sessions, outcomes and annotation types for the sessions in the list">

**Statistics** counts outcomes and annotation types across the sessions in the list, or the ones you select.

### Get Insights

<img src="docs/images/report-light.png" alt="An Insights report with a summary, recommended actions with their evidence, key takeaways and the sessions it covers">

In **Insights**, choose sessions and click **Get insights**. Claude reads their analysis and recommends changes to your code, agent instructions, skills, setup or automation, each linked to the notes it's based on. Mark each recommendation done, in progress, declined or thinking about it, with a note. Later reports on the same projects take your responses into account and say whether what you did worked.

### Share a session

Choose **Export…** from the **⋯** menu at the top right of a transcript to save it as a `.spsession` file, notes included. Anyone with Session Player can open it with **Import…** (`⌘⇧I`).

### Use the command line

`sp` analyzes sessions and writes reports from a terminal. It uses the app's database, so its results show up in the app, and it also runs on a machine without the app, including a Linux server with no display.

To install it:

- **On a Mac**, `sp` comes inside the app. Choose **File > Install Command Line Tool** once. It links `~/.local/bin/sp` to the app's own copy, so updating the app updates `sp`. From a terminal, `"/Applications/Session Player.app/Contents/MacOS/sp" install` does the same.

- **On Linux**, including a server with no display:

  ```bash
  curl -fsSL https://raw.githubusercontent.com/danmactough/session-player/main/install.sh | sh
  ```

  It downloads the `sp` for the machine's processor (x86-64 or ARM64) from the latest release, checks it against its checksum, and puts it at `~/.local/bin/sp`. Run it again to update. Then run `sp index` to read the machine's sessions.

Then:

- If `~/.local/bin` isn't on your `PATH`, the install says so; add it.
- `sp analyze`, `sp report` and `sp doctor` run the `claude` command, so Claude Code must be installed and signed in on the machine. The other commands don't need it.
- On Linux the database is in `~/.local/share/com.danmactough.sessionplayer`. Set `SESSION_PLAYER_DATA_DIR` to keep it somewhere else.

The commands:

```bash
sp index                              # read new session files into the database (every command does this first)
sp sessions --since 7d                # the current repository's sessions from the last 7 days
sp analyze                            # analyze the repository's most recently active session
sp analyze 3f2a91c0                   # analyze one session, by the start of its id or by its file's path
sp analyze --since 2026-11-01         # analyze every session since a date that has no analysis yet
sp report --since 2026-11-01          # write a report on those sessions and print it as Markdown
sp reports                            # list saved reports
sp show                               # print the repository's newest report again
sp doctor                             # start Claude Code on the newest report's recommended actions
sp pull laptop --since 30d            # copy the repository's sessions, analysis and reports from another machine
sp sessions laptop --since 30d        # list the sessions another machine has for the repository
```

- Commands work on the repository the current folder is in. `--project <folder>` names another.
- `sp index` does what the app does when it opens: it reads new and changed session files from the agents' folders. Other commands do it first anyway, so it is for preparing a machine with no app ahead of time. `--full` reads every file again.
- `--since` takes a date (`2026-11-01`), a date and time (`2026-11-01T09:30`) or an age (`12h`, `7d`, `2w`).
- `analyze` and `report` take `--prompt "<text>"` or `--prompt-file <file>` (`-` for standard input) for what to focus on. Without either they use the prompt saved in the app, and the built-in one when there is none.
- `report` first analyzes the sessions in range that have no analysis, unless given `--no-analyze`. `--model` picks the model and `--out <file>` writes the Markdown to a file.
- `doctor` starts a Claude Code session in the repository. Claude reads the report, proposes a concrete change for each action not yet done or declined, asks which ones to make, makes those, and records each as done or declined with `sp action`, so the next report knows what you did. `--print` prints the prompt instead of starting Claude.
- `sp action <report> <number> --status done|in-progress|declined|considering --note "<text>"` records a response yourself.

To work on one repository from several machines, pull its sessions from the others:

1. Install `sp` on each machine, as above. Both ways put it at `~/.local/bin/sp`, where a pull from another machine looks for it.
2. Check that `ssh <host>` reaches the other machine without asking for anything. Over Tailscale, the host is the machine's Tailscale name.
3. In a checkout of the repository, run `sp pull <host>`.

- The checkouts can be at different paths. They count as one repository when their `origin` URLs match, or, without an origin, when their folders have the same name.
- Pulled sessions are listed under the local checkout and go into the agents' own folders, so `claude --resume <id>` and the app's Continue work on them. Analysis, annotations, reports and the responses to their actions come too.
- When both machines have a copy of something, the newer one is kept. For a session, that is the copy that continues the other, or the one with the later last event when it was continued on both machines; the other machine's extra turns are dropped.
- `sp sessions <host>` lists what the other machine has. `sp pull <host> --dry-run` shows what a pull would do without writing anything: each file it would add or replace, where, and each file it would leave because the copy here is newer.
- Pulling again copies only what changed. Deletions don't travel: something deleted here comes back from a machine that still has it.

#### Run the server

`sp` runs each command through the server on this machine when one is running, so the app and the terminal share one index and one set of jobs. With no server running, it works on the database directly.

```bash
sp serve                              # run the server in this terminal until Ctrl-C
sp service install                    # install it as a per-user service (launchd on a Mac, systemd on Linux) and start it
sp service status                     # also: start, stop, uninstall
sp --local analyze                    # run in this process, even with a server running
sp cancel <job>                       # stop a job running on the server
```

- The server listens on `127.0.0.1:47800`. Settings, such as listening for other machines, are in `server.json` in the data folder; see [docs/api.md](docs/api.md#configuration-serverjson).
- `sp service install` writes `~/Library/LaunchAgents/com.danmactough.sessionplayer.server.plist` on a Mac and `~/.config/systemd/user/session-player.service` on Linux. Its output goes to `server.log` in the logs folder. The service's `PATH` includes `~/.local/bin` and `~/.claude/local`, so it finds `claude`.
- Jobs belong to the server. Pressing Ctrl-C during `sp analyze` or `sp report` over the server leaves the job running and prints its id; `sp cancel <id>` stops it.

#### Work from another machine

Pairing lets one machine use another's server over the network, with a token for each device and a certificate pinned at pairing.

1. On the machine with the sessions, set `external_bind` and `advertise` in `server.json` (see [docs/api.md](docs/api.md#configuration-serverjson)) and restart the server.
2. There, run `sp pair`. It prints a pairing string starting with `sp1.`, good once, for 5 minutes.
3. On the other machine, run `sp connect <string> --name studio`, or paste the string into the app's server dialog.

Then:

```bash
sp --server studio sessions --since 7d   # run any command on the paired server
sp pull studio --since 30d               # copy the repository's sessions from it over HTTPS instead of SSH
sp devices                               # on the server: list paired devices
sp unpair <device>                       # on the server: revoke one
```

- `sp pull <name>` uses the paired server when `<name>` is one saved by `sp connect`, and SSH otherwise.
- The server opens its network listener (port 47801) only while a device is paired or a pairing string is waiting to be used.
- `sp pull`, `sp pair`, `sp devices` and `sp unpair` act on this machine's own server, so a paired device can't run them on it; `sp pull` refuses `--server`.
- Plain HTTP on the network listener (`"tls": { "mode": "plain" }` in `server.json`) sends tokens and sessions unencrypted, so `sp serve` refuses it unless started with `sp serve --allow-insecure-http`.

### Use it in a browser

`sp serve` also serves the app's interface, so you can use Session Player from a browser, including on Linux.

1. With the server running, open http://127.0.0.1:47800.
2. The page asks you to pair the browser. Run `sp pair` and paste the string.

The browser stays paired until you remove it with `sp unpair` (`sp devices` lists it under the name you gave). To use a server on another machine, set it up as in [Work from another machine](#work-from-another-machine) and open `https://<advertise>:47801`. The certificate is self-signed, so the browser warns about it once. Native features, such as Reveal in Finder and opening a session in a terminal, are only in the app; export saves a download and import takes an upload.

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

`npm run app` starts a dev build with hot reload. A dev build has no `sp` inside it, so its server dialog can't install or start a server. Run one yourself first:

```bash
cargo run --manifest-path src-tauri/Cargo.toml -p session-player-cli -- serve
```

To work on layout and styling without the native shell, run that server, then `npm run dev`, and open http://localhost:1420. Vite passes `/api` to the server on port 47800, rewriting `Origin` so the server accepts it, and the page asks you to pair the browser (`sp pair`).

### Test

```bash
npm run test:core   # parsers, indexing, annotations, bundles, sync
npm run test:app    # the Tauri command layer
npm run test:cli    # the sp command
npm run test:server # the HTTP server: auth, pairing, events, images, the service unit
npm run test:client # the HTTP client: pinning, pairing, pull
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

The app is written to `src-tauri/target/release/bundle/macos/Session Player.app`. This build has no `sp` inside it; only a release does.

To build `sp` on its own, which needs none of the app's GUI libraries and also works on Linux:

```bash
npm run build
cargo build --release --manifest-path src-tauri/Cargo.toml -p session-player-cli
```

It is written to `src-tauri/target/release/sp`. `sp` embeds the web interface from `dist/` when it is compiled, so run `npm run build` first. Without `dist/`, the build writes a placeholder page there and `sp serve` shows that instead of the app.

### Release

```bash
./build-and-publish.sh patch
```

Use `patch`, `minor`, `major` or an exact `x.y.z`. The script bumps the version with `npm version`, commits the bump, builds a signed and notarized `.dmg`, and publishes it as GitHub release `v<version>`.

- The script builds the web interface, then `sp` (which embeds it), and bundles `sp` into the app as `Contents/MacOS/sp`, signed and notarized with it.
- Publishing the release starts the Release CLI workflow (`.github/workflows/release-cli.yml`), which builds statically linked `sp` binaries for Linux on x86-64 and ARM64 and adds them to the release. `install.sh` downloads them from there. To add them to an existing release, run the workflow by hand with the release's tag.

- Without a bump argument, it releases the current version.
- `--no-publish` stops after building and verifying.
- It refuses to run with uncommitted changes, off `main`, or when the tag already exists.
- The version lives only in `package.json`. `src-tauri/tauri.conf.json` reads it from there, and `npm version` copies it into `src-tauri/Cargo.toml` for the app and for `sp`.

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
6. **Update the text that lists the agents.** These are the note in `src/components/SettingsSheet.tsx`, the `SYSTEM` prompt in `src-tauri/core/src/generate.rs`, `source` in `docs/bundle-format.md`, and this README.
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
  sync.rs                      copying a repository's sessions, analysis and reports between machines
  ops/                         every operation the server, the CLI and the app run (see docs/api.md)
src-tauri/server/src/          sp serve: HTTP server, auth, pairing, TLS, event stream, file watcher, service unit, embedded web UI
src-tauri/client/src/          HTTP client with certificate pinning, finding the local server, paired servers, pull over HTTPS
src-tauri/cli/src/             the sp command; builds without the GUI stack
  main.rs                      commands, session and report lookup, the doctor prompt, pull over SSH
  backend.rs                   runs an operation in process, on the local server, or on a paired one
  markdown.rs                  a report as Markdown
src-tauri/src/
  lib.rs                       Tauri commands: native features, the local server, paired servers
  relay.rs                     passes the page's requests to paired servers, so their tokens never reach the page
  apps.rs                      desktop apps that can open a session
  terminals.rs                 terminals that can run a resume command
src/
  App.tsx                      sidebar, filters, search, keyboard navigation
  components/                  SessionList (virtualized), Transcript (virtualized), SettingsSheet
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

- A session in a git repository belongs to that repository's main checkout, whether it ran in a subdirectory or in a linked worktree at any path. Submodules stay separate projects.
- Worktrees deleted from disk can't be looked up in git. Claude Code worktrees (`<repo>/.claude/worktrees/*`) and `<repo>/.worktrees/*` still fold into their repository by path.
- Codex worktrees (`~/.codex/worktrees/<hash>/<name>`) fold into the one project with the same folder name.
- A session pulled from another machine with `sp pull` records that machine's folder. It is listed under the checkout it was pulled into.
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
- Each run replaces the outcome and components. The prompt's rules are fixed; only its "what to focus on" paragraph can be customized. Each run and each `claude` call is written to `~/Library/Logs/com.danmactough.sessionplayer/analyze.log`. Results generated from an earlier version of a growing transcript are marked as out of date. An annotation whose event isn't in the transcript any more is marked "event not found", not dropped.
- Long sessions are sent in parts and the results merged. Analysis runs in the background and can be cancelled.

#### Reports

- A report reads the outcomes, components and annotations of a set of sessions (not their titles or transcripts) and writes a title, a summary, key takeaways and recommended actions. Each action has a category (codebase, skill, instructions, environment or automation), a priority and the change to make.
- It runs the `claude` command the same way Analyze does, with Claude Sonnet unless another model is chosen. Sessions are numbered `s1`…, and annotations `a1`…, so every takeaway and action cites what it rests on; one citing nothing it was given is dropped. Large sets are sent in groups of whole sessions and the results merged.
- Each recommended action can have a status (done, in progress, declined, thinking about it) and a note, kept beside the report. A new report is told about responses from any report sharing a project with it, so it doesn't repeat a declined or failed recommendation without saying what changed.
- Each action has a success check. Mark a Done action **Didn't help** yourself, or let a later report judge it: sessions started after the action was marked Done are compared to its check, and the report gives a verdict of worked, didn't work or too early. A verdict that it worked or didn't must cite those later sessions, or it is dropped.
- Reports are saved in the app's database for good and never change. Each records the version of every annotation it was given, so editing or deleting an annotation later doesn't change what the report shows. Deleting a report releases versions no other report cites.
- Code: `src-tauri/core/src/reports.rs`.

#### Bundles

- The export sheet lists the sessions, the related sessions left out (the parent of a subagent, other pi branches, unticked subagents), and every attachment, with screenshots included and file cards whose contents weren't recorded marked as omitted. Saving writes the bundle shown, unchanged.
- Export downloads images that replies link from the web into the bundle, and imported transcripts never load images from the web.
- Importing a session already in the library with different contents asks whether to replace or keep the library's copy.
- The file format and the inclusion rules are in [docs/bundle-format.md](docs/bundle-format.md).

#### Sync

- `sp pull <host>` runs `sp export-project` on the other machine over SSH and stores what it writes: session files into the agents' folders, everything else into the database.
- Reports cite annotations by id and version, so annotations keep their version numbers when pulled.
- The stream's format and the rules for which copy wins are in [docs/sync-format.md](docs/sync-format.md).
- Code: `src-tauri/core/src/sync.rs`.

## License

[MIT](LICENSE)
