<!-- Paste everything below into your vault's AGENTS.md (or CLAUDE.md). -->

## Task logs

`task-logs/` holds one note per **task** — a single branch, its Linear issue, and the worktree(s) created for it by [WorktreeManager](https://github.com/pibahamondesw/WorktreeManager). A task is a smaller unit than a project: a project spans tickets and repos, a task log is one branch.

```text
task-logs/
  WOR-39-evaluar-obsidian.md
  _archive/                    # tasks whose worktrees have been deleted
```

**The app owns the frontmatter; you and your agents own the body.** WorktreeManager creates the note when the task is created and archives it when the task is deleted. It never overwrites an existing note.

### Naming

`<ISSUE-ID>-<kebab-slug>.md`, e.g. `WOR-39-evaluar-obsidian.md`. With no Linear issue, use the branch slug: `fix-flaky-webhook-spec.md`. Never a generic name — the folder already says it's a task log.

### Frontmatter

```yaml
---
title: "WOR-39 — Evaluar uso de Obsidian"
type: task-log
status: active          # active | archived
created: YYYY-MM-DD
updated: YYYY-MM-DD
tags: []
tickets: [WOR-39]       # Linear ticket IDs
branch: pedrobahamondes/wor-39-evaluar-uso-de-obsidian
workspace: WorktreeManager
repos: [worktreemanager]
worktrees:
  - repo: worktreemanager
    path: ~/Documents/.worktreemanager/worktrees/worktreemanager/pedrobahamondes/wor-39-...
prs: []
related: []             # wikilinks — e.g. "[[obsidian-integration_project]]"
---
```

Body sections: `## Context`, `## Decisions`, `## Learnings`, `## Log`.

### Distilled, not a transcript (the rule that matters)

A task log records the **conclusion**, not the conversation. Write:

- **Context** — what the task is actually about, in two or three sentences.
- **Decisions** — what was decided and *why*. Include what was rejected and the reason; that is the part nobody can reconstruct later.
- **Learnings** — what surprised you. A non-obvious constraint, a misleading error, a pattern that already existed and should have been reused.
- **Log** — dated one-liners, only for things a future reader needs: a blocker hit, an approach abandoned, a spec that changed.

Do **not** write: a play-by-play of the session, the issue description (that's Linear's job), file listings, or anything already obvious from `git log`.

When explaining code, cite sources as `path/to/file.ext:line` relative to the repo root, and name the symbol too — line numbers drift.

### Linking

Link the task log up to its project note when one exists, from both sides:

```yaml
related:
  - "[[obsidian-integration_project]]"
```

Recurring findings that outlive the task belong in the project or a longer-lived note, linked from here — not buried in a log that gets archived.

### Reading logs back

Before non-trivial work, grep `task-logs/` and `task-logs/_archive/` for the domain terms involved. Past decisions, dead ends, and prior art are recorded there — check before re-deriving them.

### Never put secrets in a task log

No tokens, no `.env` contents, no credentials. The vault may be synced.
