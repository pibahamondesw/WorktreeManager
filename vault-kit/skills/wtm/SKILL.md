---
name: wtm
description: Use WorktreeManager (the `wtm` CLI) for every worktree, branch-per-task and parallel-work need instead of git worktree add, EnterWorktree or hand-made folders. Use when about to create a worktree or branch, switch branches in a main checkout, work on something in parallel, or when the user mentions WorktreeManager, WTM, tasks or workspaces.
---

# WorktreeManager

WorktreeManager (WTM) owns worktrees. A **workspace** groups peer repositories; a **task** is one branch with one worktree per participating repository, optionally linked to a Linear issue, with a task log in the user's vault. The app only knows what goes through it, so worktrees made elsewhere are invisible to it and leak on deletion.

## Prefer WTM over ad hoc worktrees

| Instead of | Do |
| --- | --- |
| `git worktree add`, `EnterWorktree`, sibling clones | Propose a task and create it with `wtm task create` once the user agrees (see `wtm-start-task`). |
| `git checkout -b` in the main checkout to start new work | Same: a task keeps the main checkout untouched. |
| `git worktree remove`, `rm -rf` on a worktree | `wtm task delete` (see `wtm-finish-task`). |
| Guessing paths of other repositories' worktrees | Read `members[].path` from `wtm task get <id>`. |

Never create, move or delete worktrees behind the app's back. If `wtm` is missing or the app is closed, ask the user to open WorktreeManager or create the task in the app.

## CLI

`wtm --help` is the reference and works without the app. Every other command needs the app running and prints JSON on stdout (`{"ok":true,"data":…,"warnings":[]}`), progress on stderr. Exit 0 = success, 1 = operation or connection failure, 2 = invalid arguments.

```bash
wtm workspace list                       # workspaces with their repositories and IDs
wtm task list [--workspace <id>]         # tasks with branch, issue and member paths
wtm task get <id> --git                  # plus ahead/behind/dirty per member repository
wtm task create --input task.json        # or --input - for stdin
wtm task link-issue <id> --issue WOR-123
wtm task delete <id> --keep-worktrees|--delete-worktrees [--force] [--skip-teardown]
```

Task input: `{"workspaceId":"…","branchName":"…","repoIds":["…"],"linearIssue":{"id":"<uuid>","identifier":"WOR-80","title":"…"}}`. Omit `repoIds` to include every repository. With `linearIssue`, pass Linear's exact `branchName`.

## Find the current task

Match `git rev-parse --show-toplevel` against `members[].path` in `wtm task list`. No match means the session is outside a WTM task.

## Safety rules

- Create, link and delete only when the user asked for it or agreed to a proposal. Reads are always fine.
- Never retry a mutation automatically after a disconnect or timeout: an accepted operation can still finish. Re-read `wtm task list` first.
- `--force` discards uncommitted files; use it only when the user explicitly wants to lose local changes.
- Deleting keeps Git branches and archives notes. Report `warnings` to the user; they are not failures.
- The CLI never accepts or returns Linear credentials; never ask for them.
