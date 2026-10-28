---
name: wtm-finish-task
description: Close a WorktreeManager task after its PR merges — confirm the merge, distill the task log, promote lasting knowledge to the project, update Linear, and propose deleting the task. Use when the task's PR is merged, the user says the work is done, or asks to clean up a task.
---

# Finish a task

## 1. Confirm it is done

For every member repository (`wtm task get <id> --git`):

- The PR is merged (`gh pr view --json state,mergedAt`), or the user confirms the work is abandoned.
- No uncommitted or unpushed work remains (`dirty`, `ahead`). If something is left, show it and stop.

## 2. Keep what matters

- Append the outcome to the task log with the `task-log` skill: final decisions, learnings, and the merged PR in `prs:`.
- Move anything that outlives the branch (decisions, follow-ups, gotchas) into the vault project when there is one (`wtm-vault-project`).
- Create follow-up issues for deferred work the user agrees to, and mention them in the log.
- Move the Linear issue to its done state if an integration has not already.

## 3. Propose deletion

Ask before deleting, and say what will happen:

```bash
wtm task delete <id> --delete-worktrees
```

- Branches are kept, and the task's vault folder is archived, never deleted.
- Approved teardown scripts run first; a failing one keeps the task. Report it instead of adding `--skip-teardown` on your own.
- `--force` discards uncommitted files; never add it without an explicit request.
- Use `--keep-worktrees` when the user wants the files on disk but the task gone.

If the session runs inside a worktree being deleted, tell the user to continue from another directory afterwards.
