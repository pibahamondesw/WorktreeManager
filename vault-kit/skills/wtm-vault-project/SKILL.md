---
name: wtm-vault-project
description: Open or resume a project in the WorktreeManager Obsidian vault and file investigations, plans and decisions there, linked to task logs. Use when work spans several issues or repositories, is research-heavy, or produces a plan or decision that should outlive one branch.
---

# Vault project

A **project** spans issues and repositories; a **task log** is one branch. Task logs are archived with their task, so anything that outlives the branch belongs in the project. Follow the vault's `AGENTS.md` for naming, frontmatter and language.

## When to propose one

- The work needs more than one task (see `wtm-split-work`).
- An investigation, comparison or design should stay readable later.
- A decision affects several repositories or future issues.

Small, single-branch work stays in its task log.

## Open or resume

1. Look for an existing project first: a `projects/*/*_project.md` whose title or `tickets:` match. Resume it if found.
2. Otherwise create one from the vault root:

   ```bash
   bash scripts/new-project.sh <slug> --tickets ISSUE-1,ISSUE-2
   ```

3. Fill the project note's goal and scope in a few lines; set `repos:` and `linear_project:` when known.

## File notes

| Folder | Holds |
| --- | --- |
| `investigations/` | Findings with evidence and `path:line` references. |
| `plans/` | Plans for agents or people, with `status: draft → approved → executing → done`. |
| `decisions/` | One decision per note: context, options, choice and why. |
| `documents/` | Images, PDFs and human input. |

Copy the matching file from `templates/`, name it after its subject (never `plan.md`), and set `updated:` when editing. Notes hold distilled conclusions, never transcripts.

## Link both ways

Add `"[[<slug>_project]]"` to the task log's `related:` and the new notes to the project's `related:`. Never overwrite existing note content; append.
