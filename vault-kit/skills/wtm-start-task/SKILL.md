---
name: wtm-start-task
description: Start work on a Linear issue, or a free-form change, as a WorktreeManager task — pick the workspace and repositories, create the task with Linear's branch name, and continue in its worktrees. Use when the user pastes or names an issue, says "let's start X", or asks for a change while the session is on a main checkout or outside any WTM task.
---

# Start a task

One task = one branch = one Linear issue = one PR per repository. Read the `wtm` skill first for the CLI rules.

## 1. Check where you are

If the current directory already belongs to a task for this issue (see `wtm` → "Find the current task"), continue there. If the issue already has a task elsewhere (`linearIssueIdentifier` in `wtm task list`), point the user to its paths instead of creating another one.

## 2. Propose before creating

Gather the following, propose it in one short message, and wait for agreement:

- **Issue** — fetch it with the user's Linear tooling: `id` (UUID), `identifier`, `title`, `branchName`. Without an issue, ask whether to create one; free-form tasks work but lose the Linear link.
- **Workspace** — `wtm workspace list`; choose the one whose repositories the change touches. Ask if more than one fits.
- **Repositories** — only the ones the change needs. A cross-repository change that merges together is one task with several members; changes that ship as separate PRs need separate issues (see `wtm-split-work`).

## 3. Create

```bash
wtm task create --input - <<'JSON'
{"workspaceId":"<id>","branchName":"<Linear branchName>","repoIds":["<id>"],
 "linearIssue":{"id":"<uuid>","identifier":"<ISSUE-1>","title":"<title>"}}
JSON
```

Creation waits for setup scripts. Report every `warnings` entry; setup warnings usually mean the repository needs `wtm-repo-setup`. After a connection error, check `wtm task list` before trying again.

## 4. Continue in the task

- Work only inside the returned `members[].path`; never edit the main checkout.
- The app writes the task log; resolve it with the `task-log` skill and fill `## Context` if it is empty.
- If the issue belongs to a larger effort, link or create a vault project (`wtm-vault-project`).
- Move the issue to its in-progress state if the user's workflow expects it.

Tell the user the branch, the worktree paths and the next step you propose.
