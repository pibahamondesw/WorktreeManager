---
name: wtm-where-am-i
description: Orient a session inside a WorktreeManager task — which task and issue it is, the state of every member worktree, open PRs, the task log and project — and propose the next step. Use at the start of a session in a WTM worktree, when resuming work, or when the user asks "where are we", "what's next" or "status".
---

# Where am I

Build a short status, then propose one next action.

## Gather

1. **Task** — match `git rev-parse --show-toplevel` against `members[].path` in `wtm task list`. Outside a task, say so and offer `wtm-start-task`.
2. **Git** — `wtm task get <id> --git` gives `ahead`, `behind` and `dirty` per member repository, keyed by `repoId`. For dirty members, check `git status --short`.
3. **Issue** — `linearIssueIdentifier`; fetch its status and latest comments with the user's Linear tooling when available.
4. **PRs** — in each member path, `gh pr view --json number,url,state,isDraft,reviewDecision,statusCheckRollup` when `gh` is installed.
5. **Notes** — the task log (`task-log` skill) and, through its `related:`, the vault project. Read `## Decisions` and the last `## Log` lines.

## Report

At most six lines: task and issue, state per repository, PR and CI state, last recorded decision, then **one** proposed next step.

| Situation | Propose |
| --- | --- |
| No commits yet | Restate the issue's goal and a first step. |
| Dirty or ahead, no PR | Finish, commit and open the PR. |
| CI failing or review comments | Address them. |
| PR merged | `wtm-finish-task`. |
| Behind the base branch | Rebase or merge before continuing. |
| Scope grew | `wtm-split-work`. |

Do not act on the proposal until the user agrees.
