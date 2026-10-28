---
name: wtm-repo-setup
description: Make a repository ready for WorktreeManager tasks — add it to a workspace and declare setup, teardown and copied local files in .worktreemanager.toml and .worktreeinclude. Use when a task is created with setup warnings, a new worktree lacks dependencies or env files, or the user wants to onboard a repository or workspace.
---

# Repository setup

New worktrees start from a clean checkout. Setup scripts and include files make them usable without manual steps.

## Add the repository to a workspace

```bash
wtm workspace list
wtm workspace create --input -    # {"name":"Payments","repos":[{"localPath":"/repos/api"}]}
wtm workspace update <id> --input -
```

`repos` in an update replaces the whole list: resend existing members with their `id`, or they are removed. Propose the change before running it.

## Declare scripts

`.worktreemanager.toml` at the repository root, committed so the team shares it:

```toml
[scripts]
setup = '''
pnpm install --frozen-lockfile
'''
teardown = "docker compose down"
```

- `setup` runs automatically from the root of each new worktree and blocks task creation. Keep it idempotent and fast.
- `teardown` runs before worktrees are removed and needs approval in the app. Use it to stop services or free ports.
- Existing Conductor, Superset or Cursor worktree configs are read too; do not duplicate them.
- Without any config the app detects Doppler, Node and Python. Add a file only when detection is not enough.

## Copy local files

`.worktreeinclude` lists Git-ignored files from the main checkout to copy into new worktrees, in `.gitignore` syntax:

```text
.env*
config/master.key
```

Only ignored files qualify; tracked files already arrive through checkout. Never commit the secrets themselves.

## Verify

Create a throwaway task with `wtm-start-task`, check that `warnings` is empty and the project runs from the new worktree, then remove it with `wtm-finish-task`.
