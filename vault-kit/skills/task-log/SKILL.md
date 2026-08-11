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

Derive the filename from the branch:

- Branch contains a Linear issue ID (e.g. `pedrobahamondes/wor-39-evaluar-obsidian`) → `WOR-39-<slug>.md`, uppercase issue ID.
- No issue ID → `<branch-slug>.md`.

Then:

- **Note exists** → append to it. This is the normal case; WorktreeManager creates the note when the task is created.
- **Exactly one file starts with the same issue ID** → that is the note, even if the slug differs. Do not create a second one.
- **No note** → create it from `templates/task-log.md`, filling frontmatter from git (branch, repo name, worktree path, today's date). If the vault has no template, use the structure in the vault's `AGENTS.md`.
- Check `_archive/` too — if the note is archived and the worktree is back, work in the archived file and tell the user.

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
