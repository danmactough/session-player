# Project sync format, version 1

`sp pull <host>` copies one repository's sessions, analysis and reports from another machine. It runs `sp export-project` on that machine over SSH and reads what it writes. This is that stream. It is never saved to a file; the format is implemented in `src-tauri/core/src/sync.rs`.

The stream is zstd-compressed. Uncompressed, it is a sequence of lines, each one JSON object, with the raw bytes of a file after each `file` line. Timestamps are milliseconds since the Unix epoch, in UTC.

## Header

The first line:

```json
{ "format": "session-player.sync", "version": 1, "repo": "github.com/danmactough/session-player", "exporter": "laptop", "exported_at": 1767261600000 }
```

| Field | Meaning |
|---|---|
| `format` | Always `session-player.sync` |
| `version` | The importer refuses a version newer than its own |
| `repo` | The repository's name (below). The importer refuses a stream for a repository other than the checkout it is importing into. |
| `exporter` | The exporting machine's host name |

## Repository names

The two machines keep the repository at different paths, so a repository is named by something both share:

- The `origin` remote's URL from `.git/config`, without scheme, user, port or `.git`, in lowercase. `git@github.com:Dan/Widgets.git` and `https://github.com/dan/widgets` are both `github.com/dan/widgets`.
- Without an origin, or when the origin is a path on the machine's own disk, the name of the main checkout's folder, in lowercase.

## Records

Every line after the header has a `record` field. Records come in this order.

### `file`

```json
{ "record": "file", "folder": "claude_projects", "rel_path": "4f1c….jsonl", "size": 48211, "session": { "source": "claude", "native_id": "4f1c…", "last_event_at": 1767261600000 } }
```

The next `size` bytes of the stream are the file, exactly as it is on disk. The next record starts right after them.

| Field | Meaning |
|---|---|
| `folder` | `claude_projects` (`~/.claude/projects`), `codex_sessions` (`~/.codex/sessions`), `codex_archived` (`~/.codex/archived_sessions`) or `pi_sessions` (`~/.pi/agent/sessions`) |
| `rel_path` | For Claude Code and pi, the path below the working directory's own folder, so `4f1c….jsonl` or `4f1c…/subagents/agent-a1.jsonl`. For Codex, the path below the agent folder, so `2026/01/02/rollout-….jsonl`. Never absolute and never containing `..`; the importer stops at one that is. |
| `session` | The session the file holds. Absent for a file an agent keeps beside a session, such as a subagent's description or a large tool result. |

What is sent:

- Every session the exporter lists under a checkout of the repository, started at or after `--since` when given. A session pulled from a third machine counts, so it can be pulled onward.
- Each session's subagents, whatever their own dates.
- For Claude Code and pi, every other file in the folder named after the session.
- Not sent: sessions imported from `.spsession` bundles, and sessions stored outside the four agent folders.

Where the importer puts a file: Claude Code and pi keep one folder per working directory, named after its path, so the file goes in the folder for the local checkout. A Codex file keeps its path. The session can then be resumed from the local checkout.

The session's own log still records the other machine's folder. The importer stores the local checkout for each session file it writes (table `session_projects`, keyed by the file's path), and the index lists the session under it.

### `meta`

```json
{ "record": "meta", "meta": { "key": "local:claude:4f1c…", "outcome": "succeeded", … } }
```

A session's analysis: the `SessionMeta` stored for it.

### `annotation`

```json
{ "record": "annotation", "current": true, "annotation": { "id": "…", "version": 2, "key": "local:claude:4f1c…", … } }
```

One version of an annotation. `current: true` is the annotation as it is now. `current: false` is an older version that a report in the stream cites; it is stored so the report still shows the text it was written from. Version numbers are kept as sent.

### `report` and `response`

```json
{ "record": "report", "report": { "id": "…", … } }
{ "record": "response", "response": { "report_id": "…", "action_index": 0, "status": "done", … } }
```

A report is sent when it covers at least one session in the stream. The versions it cites come before it, and the responses to its actions after it.

### `end`

```json
{ "record": "end" }
```

The last line. A stream without it was cut short: the importer keeps the session files it had finished copying, stores no other record, and reports the failure. Pulling again finishes the job.

## Which copy wins

Both machines may have the same thing. A pull never merges two copies; it keeps one.

| Record | Rule |
|---|---|
| Session file | Identical: nothing happens. One copy is the other plus more bytes (the session went on, or was rewound and continued): the longer one. Each has bytes the other lacks: the one whose last event is later; without timestamps, the larger. |
| File beside a session | Follows the session's own file. |
| Analysis, annotation, response | The one with the later `updated_at`. |
| The model's annotations nobody edited | Analyzing a session again replaces these under new ids, so they are taken as a set from whichever machine analyzed the session later. |
| Report | Added when absent. Reports never change. |

Deletions don't travel: an annotation or report deleted on one machine comes back from a machine that still has it.

A session file is written under a temporary name and renamed into place, so a pull that stops part-way leaves no partial session.
