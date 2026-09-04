---
name: task-log
description: Write the Obsidian task log for the current worktree — append distilled decisions, learnings, and log entries to the note for this branch's task. Use when the user asks to write/update the task log or bitácora, at the end of a work session, or when a decision worth recording has been made.
---

# Task log

Append what this session established to the task log note for the current worktree. The note is a **distilled record**, not a transcript.

## 1. Resolve the note

```bash
git rev-parse --abbrev-ref HEAD     # the branch
git rev-parse --show-toplevel       # the worktree path
```

The task-logs folder is `$TASK_LOGS` if set; otherwise find it (`task-logs/` at the root of the user's vault) and ask if it is ambiguous.

New tasks store their note at `task-logs/<task-id>/<ISSUE-ID>-<slug>.md`, or `<branch-slug>.md` without an issue. Archiving moves the entire task folder to `task-logs/_archive/<task-id>/`. Every new task gets a fresh ID, even when it reuses a branch name. Older tasks retain their flat notes in `task-logs/` or `task-logs/_archive/`.

Resolve by **worktree identity**, not by filename or ticket alone:

1. Search Markdown files recursively under `task-logs/`, excluding `_archive/`, and inspect the frontmatter `worktrees[].path` for the current worktree's absolute path. If the app task ID is known, use its folder and verify `task_id` first.
2. Use the unique matching active note. Only if none matches, search `_archive/` the same way; tell the user when writing an archived note. Never choose an archived note over an active match just because its name matches.
3. Multiple matches require checking `task_id`, branch, workspace and worktree paths. If still ambiguous, ask which task is intended; never pick the first ticket/name match.
4. If there is no note for an app-managed task, use the app's **Open notes** action to create it with the correct identity. Do not guess a folder from the branch or create a flat replacement. For a standalone task, use a new UUID with `scripts/new-task-note.sh --notes-path <vault>/task-logs --task-id <uuid>` from its worktree.

The vault's `scripts/new-task-note.sh --notes-path <vault>/task-logs` can resolve existing notes from the current worktree; it refuses ambiguous matches. Renamed notes can be located by their frontmatter. Use vault-relative paths in wikilinks when filenames repeat across task folders.

Notes are always archived, including untouched templates; never delete one because it appears empty. The storage rules here supersede older flat-filename or empty-note guidance in the vault.

## 2. Decide what is worth writing

Review the session and keep only what a future reader needs. Most sessions produce two or three bullets total. An empty section is better than a padded one.

| Section | Write | Skip |
| --- | --- | --- |
| `## Context` | Two or three sentences on what the task is about. Fill only if empty. | The Linear issue description. |
| `## Decisions` | What was decided and **why**. What was rejected and the reason. | Decisions with no alternative considered. |
| `## Learnings` | What surprised you: a non-obvious constraint, a misleading error, existing prior art that should be reused. | Anything obvious from reading the code. |
| `## Log` | Dated one-liners: a blocker hit, an approach abandoned, a spec that changed. | A play-by-play of the session. |

Never write: file listings, command transcripts, restated diffs, anything already in `git log`, or content that duplicates Linear.

Never write secrets — no tokens, no `.env` contents, no credentials. Vaults get synced.

## 3. Write

- **Append** to the existing sections; never rewrite or reorder what is already there.
- The only frontmatter field you may change is `updated:` — set it to today. Add to `tickets:`, `prs:`, or `related:` only when you have a value that is genuinely missing.
- Cite code as `path/to/file.ext:line` relative to the repo root, and name the symbol as well — line numbers drift.
- `## Log` entries are prefixed with the date: `- 2026-08-04 — Dropped the polling approach; the webhook already carries the state.`
- Match the vault's configured language (see its `AGENTS.md`). Frontmatter keys and section headings stay in English.
- Link up to a project note in `related:` when one exists — `"[[<slug>_project]]"`.
- Prose is not hard-wrapped: one line per paragraph or bullet.

Then tell the user the note path and, in one line, what you added.

## 4. When there is nothing to write

Say so and write nothing. A log padded with filler is worse than a short one — it trains the user to stop reading it.
