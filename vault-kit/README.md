# Obsidian vault kit

WorktreeManager knows more about a unit of work than anything else on your machine: a **task** is a branch, a Linear issue, and one worktree per member repo. When you delete the worktree, all of that context disappears — including _why_ you took an approach, what you tried and rejected, and what you learned.

Enabling the vault gives you a durable home for that context: a full Obsidian vault the app creates and keeps in sync — per-task notes written at creation, opened with one keystroke, archived on deletion — plus a project layer for work that spans tickets and repos.

---

## Enabling it

One toggle, no manual assembly:

- **Setup wizard** — check "Set up an Obsidian vault for task notes" when first configuring the app, or
- **Sidebar footer → Obsidian vault → Enable** at any time.

The app scaffolds the vault at `~/Documents/worktreemanager-vault`, registers it with Obsidian, and starts writing task notes into it. No community plugins are needed — Mermaid renders natively.

Leave the toggle off and the whole feature stays off.

## What gets created

Everything under [`vault/`](vault/) in this directory, verbatim:

```text
<vault>/
  AGENTS.md            # the vault guide: conventions, frontmatter spec, workflow
  CLAUDE.md            # @AGENTS.md
  agent-setup.md       # snippet for your AI tools' global instructions (see below)
  .gitignore
  projects/            # YYYY-MM-<slug>/ — investigations, plans, decisions, documents
  task-logs/           # <task-id>/<note>.md, written by the app
    _archive/          # tasks that left something behind
  templates/           # project, investigation, plan, decision, task-log
  scripts/
    new-project.sh     # scaffolds a project folder
    archive.sh         # archives finished projects into _archive/
    new-task-note.sh    # resolves or creates a task note
    archive-task-note.sh
  skills/task-log/     # task-note resolution and writing rules
```

**Scaffolding preserves existing files.** While the vault is enabled, startup repairs a missing vault and registers it with Obsidian. It also installs missing task-note scripts and `skills/task-log/SKILL.md`, and appends a reference to that skill to the existing `AGENTS.md` once, preserving custom instructions. These support files are repaired if missing; other individually deleted files stay deleted. Existing support files are not overwritten.

New tasks use `task-logs/<task-id>/<note>.md`; deleting them moves the whole folder to `task-logs/_archive/<task-id>/`. Task IDs distinguish separate creations of the same branch. Older tasks keep their flat paths and are always archived too; their historical name collisions are not migrated automatically.

Two layers, one system: **projects** span tickets and repos; a **task log** is one branch. A task log links up to its project note via `related:`, and anything that outlives the branch belongs in the project. The guide's "Task logs" section in `AGENTS.md` carries the full rules.

## Wiring up your agents

The vault only pays off if agents write to it and read it back. `<vault>/agent-setup.md` is a short router snippet for your AI tools' **global** instructions — it tells every session where the vault is, when to write a task log, and when to open a project. Install it once per tool (Claude Code: one `@<vault>/agent-setup.md` line in `~/.claude/CLAUDE.md`; other tools: paste — the file lists them). The vault settings modal shows the exact line with a copy button.

This is plain Markdown, so it works with any agent that supports global instructions — nothing here is Claude-specific.

### Optional: the `/task-log` skill (Claude Code)

[`skills/task-log/`](skills/task-log/) packages the task-log writing rules as an explicit slash command. With the setup snippet installed it's a convenience, not a requirement:

```bash
cp -R skills/task-log ~/.claude/skills/task-log
```

Run `/task-log` from inside a worktree and it resolves the right note from the branch name and appends what it learned this session. Once you trust it, promote it to automatic with a `Stop` hook — ask Claude to set it up, or see the [hooks docs](https://docs.claude.com/en/docs/claude-code/hooks).

---

## Daily use

| When                     | What happens                                                                                                                                                                                          |
| ------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| You create a task        | The app writes the note with frontmatter filled in. It never overwrites an existing note, so your prose is safe.                                                                                      |
| You are working          | Press `O` (or **More actions → Open notes**) to open the note in Obsidian. Your agent appends distilled decisions and learnings as they happen.                                                       |
| You delete the task      | The task folder moves to `task-logs/_archive/<task-id>/` with the note marked `status: archived`. Notes and attachments are kept, including untouched templates. |
| You remove the workspace | Same, once per task — whether or not you keep the worktrees on disk.                                                                                                                                  |

The point is not to have notes. The point is that in three months `grep` over `task-logs/` answers questions your git history cannot — and agents are instructed to do exactly that before non-trivial work.

---

## Without the app

The scripts in [`scripts/`](scripts/) do the task-note work from a terminal — useful for debugging the integration or driving it from other tooling. Both take `TASK_LOGS` from the environment (or `--notes-path`).

```bash
export TASK_LOGS=~/Documents/worktreemanager-vault/task-logs

# From inside a worktree — resolves an existing note by its worktree path
./scripts/new-task-note.sh

# Explicitly
./scripts/new-task-note.sh --task-id <task-id> --branch pedrobahamondes/wor-39-evaluar-obsidian --repo worktreemanager

./scripts/archive-task-note.sh <task-id>/WOR-39-evaluar-obsidian.md
```

---

Creation without an existing match requires `--task-id`: use the app task ID, or a new UUID for a standalone task. Reuse that ID to resolve the same note. The resolver searches recursively by worktree path, prefers active notes, and refuses ambiguous matches. Legacy flat notes remain supported.

## Deleting notes

**The app never deletes a task note.** Deleting a task or removing a workspace archives it; that asymmetry is deliberate. A note that outlived its usefulness costs a few KB and one `grep` hit. A note deleted the moment you merged costs the only artifact that was supposed to survive the worktree — which is the whole point.

Untouched notes are archived too. The app does not infer whether a note contains valuable information.

If you genuinely want one gone, they are plain Markdown files — delete it in Obsidian, or `rm` it. That is a vault operation, and Obsidian is better at it than a button in WorktreeManager would be.

---

## Two things to get right

**Do not duplicate Linear.** The note holds no issue description and no status. It holds what Linear cannot: what you tried that failed, what you assumed, what took you an hour to understand.

**Never put secrets in the vault.** Vaults get synced. No tokens, no `.env` contents, no credentials.
