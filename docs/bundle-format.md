# Session bundle format, version 2

A session bundle is one session, plus the subagents the sender chose, in a single file that another Session Player can import and replay. The importer doesn't need the agent's JSONL log, the agent installed, or any of the sender's directories. The file extension is `.spsession`, and the format is implemented in `src-tauri/src/bundle.rs`.

The file is one UTF-8 JSON object:

```json
{
  "format": "session-player.bundle",
  "version": 2,
  "exported_at": 1767261600000,
  "exporter": "Session Player 0.7.0",
  "sessions": [ /* session objects; the first is the one exported */ ],
  "attachments": [ /* attachment objects */ ]
}
```

Timestamps are milliseconds since the Unix epoch, in UTC.

## Sessions

| Field | Meaning |
|---|---|
| `id` | Stable session ID: `<source>:<native id>`, such as `claude:4f1c…`. A pi branch adds `@<leaf entry id>`. Exporting the same session again gives the same ID. |
| `source` | `claude`, `codex` or `pi` |
| `native_id` | The agent's own session ID |
| `branch_leaf` | For a pi branch: the last entry of the exported branch |
| `title`, `title_source` | The label the sender saw, and where it came from (`custom`, `ai`, `thread`, `name`, `prompt`, `branch`, …) |
| `project`, `cwd`, `git_branch` | Where the session ran, as recorded on the sender's machine. Shown as labels only. |
| `models`, `tokens` | Models used; `{input, output, cache_read}` token totals |
| `started_at` | When the session started |
| `is_subagent`, `parent_id` | Whether this is a subagent or fork, and the parent's native ID |
| `imported_from` | Set when the agent imported the session from another agent (for example, Codex importing from Claude Code) |
| `source_file` | File name of the session log it was read from. The bundle never stores the full path. |
| `omitted` | Related sessions that are **not** in the bundle: `{relation, id, title}`, where `relation` is `parent`, `subagent` or `branch` |
| `events` | The normalized transcript (see below) |
| `meta` | Optional. The session's analysis: `outcome` (`unknown`, `succeeded`, `partial` or `failed`), `outcome_note` (why), `components` (repository paths or packages worked on, such as `packages/api`), `generated` (the fields still holding what a model generated, which regenerating may replace), `generated_by` (`{model, at, revision}`) and `cleared` (the fields a person emptied on purpose, which generating leaves empty). |
| `annotations` | Optional. Annotations on the session (see below). |

### Events

Events use the same normalized model the app builds from every agent's log: `kind` (`user`, `assistant`, `thinking`, `tool_call`, `tool_result`, `command`, `system`, `compaction`, `context`), `ts`, `text`, and the optional fields `tool_name`, `call_id`, `is_error`, `exit_code`, `failed_exit`, `model`, `images`, `sidechain`, `entry`.

A `context` event is something the harness put in the agent's prompt, with its text in `text` and a `context` field `{kind, name, repeat, scope}`. The `scope` is `user`, `repo`, `local` or `managed`, or absent when the document isn't a file. The `kind` is `instructions` (CLAUDE.md, CLAUDE.local.md, AGENTS.md…), `skill`, `agent` (a subagent's definition) or `mcp` (an MCP server's instructions). The `name` is the file's path, or the skill, agent or server's name. `repeat` marks a document sent again unchanged later in the session. These documents travel in full, so the export sheet lists them for review.

Each event also has an `anchor`: `e<n>` for the conversation's events, counted in order, and `c<n>` for context documents, counted apart so that context doesn't renumber the conversation. Anchors are unique within a session, and import never changes them, so a later note or comment can refer to an event across export and import. Anchors are scoped to the bundle; they aren't positions in the sender's log.

An image in `images` is a reference, `attachment:<attachment id>`, not the image itself.

## What is included

- **The selected session.** A pi fork file is exported with the history it copied from its parent, so the transcript starts at the session root. For a pi session with branches, the bundle holds the selected branch from the session root to its last entry, including the history it shares with the branch it split from. Every other branch, including the active branch when another is selected, is listed in `omitted` with `relation: "branch"`.
- **Subagents.** Subagents of the selected session are included unless the sender unticks them in the export sheet. A subagent left out is listed in `omitted` with `relation: "subagent"`. A pi branch has no subagents of its own.
- **Never included.** The parent of a subagent or fork (listed in `omitted` with `relation: "parent"`), forks of the session, and any other session. Every session in `sessions` after the first must be a subagent of the first; import rejects a bundle that holds anything else.

## Annotations

Each annotation marks a moment in the session; how the session went as a whole is its `meta`.

| Field | Meaning |
|---|---|
| `id` | Unique, kept through export and import, so a re-import updates the same annotation |
| `kind` | `tool_error`, `human_intervention`, `agent_stuck`, `human_frustration`, `wrong_assumption`, `wasted_work`, `environment`, `missing_context`, `automatable`, `key_decision`, `ignored_instruction` or `other` |
| `note` | Free text |
| `anchors` | The events it is about, at least one, each `{anchor, kind, ts, entry}`, where `entry` is set for pi |
| `author` | `{type: "person", name}` (`name` may be null) or `{type: "model", model}` |
| `created_at`, `updated_at` | Milliseconds |
| `edited` | For a model's annotation: someone changed it, so regenerating keeps it |
| `rationale`, `confidence`, `revision` | For a model's annotation: why it was made, how sure the model was (`low`, `medium`, `high`), and the transcript it was made from |

`key` is empty in a bundle; import files metadata and annotations under the imported session. A pi branch exports only the annotations about events on that branch. Import rejects a bundle with an annotation that points at no event.

On import, a new session's metadata and annotations are added. When a session is replaced after a conflict, the bundle's metadata replaces the library's, and each annotation replaces the one with the same `id`; annotations the recipient added stay. Keeping the library's copy, or re-importing unchanged content, leaves the recipient's notes as they are. Readers of an older version of this format ignore `meta` and `annotations`, so the format version is unchanged.

## Attachments

| Field | Meaning |
|---|---|
| `id` | `a<n>`, numbered per session, so a session's attachment IDs are the same in every bundle it appears in. Unique together with `session`. |
| `session`, `anchor` | The session and event it is attached to |
| `kind` | `image` or `file` |
| `name` | The file's path as the agent recorded it (files only) |
| `mime`, `bytes` | Media type (images: `image/png`, `image/jpeg`, `image/gif`, `image/webp` or `image/svg+xml`) and decoded size |
| `status` | `included` or `omitted` |
| `reason` | Why an omitted attachment is not in the bundle |
| `data` | Base64 image data. Present only for an included image. |

- **Images** (screenshots pasted into a prompt, images returned by tools) are stored in `data`. An image the app doesn't load (over 4 MB of base64) is listed as `omitted`, with a reason.
- **Remote images**, meaning images that a reply, reasoning block or compaction summary links from the web (Markdown `![alt](https://…)` or HTML `<img src="https://…">`) and image links an agent recorded instead of the image itself, are downloaded during export (PNG, JPEG, GIF, WebP or SVG, up to 3 MB each). `name` holds the original address. In the event text, the address is replaced with `attachment:<id>`, so the bundle never points at the web. A download that fails is listed as `omitted` with the reason. Session Player never loads anything from the web in an imported transcript. An image that wasn't downloaded shows as "remote image not included". Inline SVG in replies still renders, but its references to outside addresses (`<image href>`, `<feImage>`, `url(https://…)` and similar) are removed. References to elements in the same document (`#id`) and `data:` URLs stay.
- **Files** attached to a prompt (pi `@file`, Claude Code `@file` and IDE notices, Codex "files mentioned") keep their contents inline in the prompt's `<file name="…">…</file>` card, where the transcript shows them. Their manifest entry records whether the contents came along: `included` with a size, or `omitted` when the agent recorded only the path.

## Validation

Import rejects a file, and writes nothing, when:

- it isn't JSON, or `format` isn't `session-player.bundle`
- `version` is missing, or is newer than the app supports (the message asks the user to update Session Player)
- it is larger than 512 MB, or a required field is missing or has the wrong type
- it has no sessions, a session ID appears twice, or an anchor repeats within a session
- a session's `id` isn't the one its `source`, `native_id` and `branch_leaf` produce, or is longer than 180 characters
- a session after the first isn't a subagent of the first
- an image reference points to a missing attachment, or to an attachment of another session
- an attachment belongs to a session that isn't in the bundle, an included image has no data or an unsupported media type, its data isn't valid base64, or an omitted attachment carries data

## Library and duplicates

Import splits a bundle into one single-session bundle per session and stores it in the app's library, `~/Library/Application Support/com.danmactough.sessionplayer/library/`. The library file name is the session ID encoded as URL-safe base64, so no ID can name a path outside the library. The indexer reads the library alongside the session folders. Imported sessions are marked "imported" and keep the title and project the sender saw. They can be read, searched and exported again, but not resumed or opened in an agent app.

Import skips a session with no prompt, reply or tool call, which the session list wouldn't show, and reports it as empty. When the exported session itself is empty, nothing in the bundle is imported, so its subagents are never imported without it.

When a session ID is already in the library:

- **Same contents:** the session is skipped and reported as already in the library. The comparison covers the session and its attachments, not `exported_at` or `exporter`.
- **Different contents:** import stops before writing anything and lists the conflicts. The user then chooses **Replace library copies**, which writes the bundle's version of every conflicting session, or **Keep library copies**, which imports only the sessions that don't conflict.

**Removing.** **Remove…** in an imported session's header deletes its library file, and those of the subagents imported with it. A library file that can't be read doesn't appear in the session list. Settings lists such files under **Imported sessions**, each with a **Remove** button. Importing a session whose library copy is damaged stops with a message pointing there.

An imported session and a local session with the same ID are different copies. Both are listed, and each nests only under a parent of its own kind.

## Versioning

Version 2 added `context` events and the `ignored_instruction` annotation kind. A version 1 reader rejects a version 2 bundle with a message to update, instead of failing on the new values.


`version` changes when a reader for an older version can no longer read the file correctly. New optional fields don't need a new version. A reader rejects versions newer than it knows. It should keep reading older versions.
