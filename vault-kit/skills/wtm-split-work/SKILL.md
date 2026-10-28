---
name: wtm-split-work
description: Split work that outgrows one PR into one Linear sub-issue and one WorktreeManager task per PR. Use when a plan has more than one PR, the diff grows past a reviewable size, a second repository needs an independent PR, or stacked PRs are being considered.
---

# Split work

The invariant is **1 PR = 1 Linear issue = 1 WTM task**. A shared task hides the other PRs' worktrees, and deleting it for one PR loses the rest.

## When to propose a split

Propose it, never split silently, when any of these appear:

- The plan lists independent deliverables, or a refactor that could land before the feature.
- A second repository needs its own PR (different review, deploy or merge order).
- The diff mixes mechanical changes (renames, moves) with behavior changes.
- A reviewer would need more than one sitting to read it.

A cross-repository change that must merge together stays as **one** task with several members.

## Procedure

1. Draft the units: title, one-line scope, repository, and what each one is blocked by. Each unit must be shippable on its own.
2. Show the list and wait for agreement.
3. For each unit, create a Linear sub-issue of the original issue with its blocking relations, using the user's Linear tooling.
4. Create one task per unit with `wtm-start-task`, using each sub-issue's `branchName`. For stacked PRs, base each branch on the previous unit's branch with Git inside the new worktree.
5. Record the split as a decision: in the vault project if there is one (`wtm-vault-project`), otherwise in the original task's log.

Prefix each PR title with its own issue identifier. Move code between worktrees with Git (commits, cherry-picks), never by copying files.
