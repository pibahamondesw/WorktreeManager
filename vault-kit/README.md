# Obsidian task logs — setup kit

WorktreeManager knows more about a unit of work than anything else on your machine: a **task** is a branch, a Linear issue, and one worktree per member repo. When you delete the worktree, all of that context disappears — including _why_ you took an approach, what you tried and rejected, and what you learned.

This kit adds a durable, per-task note in an Obsidian vault, and the app keeps it in sync: created when you create the task, opened with one keystroke, archived when you delete the task.

It is an **add-on**, not a vault. It grafts onto whatever vault you already have.

---

## What you get

```text
<vault>/
  task-logs/
    WOR-39-evaluar-obsidian.md    # one note per task
    _archive/                     # tasks that left something behind
```

Each note carries the task's identity in frontmatter (issue, branch, repos, worktree paths, PRs) and four sections you actually write in: **Context**, **Decisions**, **Learnings**, **Log**.

The point is not to have notes. The point is that in three months `grep` over `task-logs/` answers questions your git history cannot.

---

## Install

### Option A — graft onto a vault you already have (recommended)

1. **Create the folder.**

   ```bash
   mkdir -p "<vault>/task-logs/_archive"
   ```

2. **Register the note type.** Paste the contents of [`AGENTS.snippet.md`](AGENTS.snippet.md) into your vault's `AGENTS.md` (or `CLAUDE.md`). This is what tells agents the folder exists, how notes are named, and — most importantly — that a log is _distilled_, not a transcript.

3. **Copy the template.**

   ```bash
   cp templates/task-log.md "<vault>/templates/task-log.md"
   ```

4. **Install the skill.** Either per-vault or globally:

   ```bash
   cp -R skills/task-log "<vault>/.claude/skills/task-log"   # per-vault
   cp -R skills/task-log ~/.claude/skills/task-log           # everywhere
   ```

5. **Point the app at it.** In WorktreeManager, edit your workspace and set **Notes folder** to `<vault>/task-logs`. Leave it empty and the whole feature stays off.

### Option B — start from zero

If you have no vault yet, the fastest path is to create one and then follow Option A:

1. Install [Obsidian](https://obsidian.md).
2. `mkdir -p ~/Documents/work-vault` and use **Open folder as vault** to select it. No community plugins are needed — Mermaid renders natively.
3. Create an `AGENTS.md` at the vault root and a `CLAUDE.md` containing `@AGENTS.md`.
4. Follow Option A.

For a fuller, project-centric vault (multi-ticket projects with `investigations/`, `plans/`, `decisions/`), see [`RaulFintoc/agents-vault-template`](https://github.com/RaulFintoc/agents-vault-template) and add this kit on top of it. The two layers are complementary: **projects** span tickets and repos; a **task log** is one branch. A task log links up to its project note via `related:`.

---

## Daily use

| When                     | What happens                                                                                                                                                                                |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| You create a task        | The app writes the note with frontmatter filled in. It never overwrites an existing note, so your prose is safe.                                                                            |
| You are working          | Press `O` (or **More actions → Open notes**) to open the note in Obsidian. Run `/task-log` in Claude Code to have the log written for you.                                                  |
| You delete the task      | The note moves to `_archive/` with `status: archived`. The body is untouched. A note nobody wrote in is discarded instead, so `_archive/` keeps meaning "tasks that left something behind". |
| You remove the workspace | Same, once per task — whether or not you keep the worktrees on disk.                                                                                                                        |

### Let Claude write it

The one non-negotiable part: **if writing the log is manual, you will stop doing it within three weeks.** Use the skill.

```bash
/task-log
```

Run from inside a worktree, it resolves the right note from the branch name and appends what it learned this session — no arguments needed.

Once you trust it, promote it to automatic by adding a `Stop` hook so the log is written at the end of every session. Ask Claude to set it up, or see the [hooks docs](https://docs.claude.com/en/docs/claude-code/hooks).

### Read it back

This is the half people skip, and it is where the payoff lives. Add a line to your vault's `AGENTS.md` telling agents to search prior logs before starting non-trivial work:

```markdown
Before non-trivial work, grep `task-logs/` and `task-logs/_archive/` for the
domain terms involved. Past decisions and dead ends are recorded there.
```

---

## Without the app

The scripts do the same work from a terminal — useful if you have not set **Notes folder**, or for debugging the integration. Both are idempotent and take `TASK_LOGS` from the environment (or `--notes-path`).

```bash
export TASK_LOGS=~/Documents/work-vault/task-logs

# From inside a worktree — infers issue, branch, and repo from the branch and cwd
./scripts/new-task-note.sh

# Explicitly
./scripts/new-task-note.sh --branch pedrobahamondes/wor-39-evaluar-obsidian --repo worktreemanager

./scripts/archive-task-note.sh WOR-39-evaluar-obsidian.md
```

---

## Deleting notes

**The app never deletes a note you wrote in.** Deleting a task or removing a workspace archives it; that asymmetry is deliberate. A note that outlived its usefulness costs a few KB and one `grep` hit. A note deleted the moment you merged costs the only artifact that was supposed to survive the worktree — which is the whole point.

The one exception is a note nobody touched: on archive, a body that still holds only headings and template comments is discarded rather than filed, since there is nothing there to lose.

If you genuinely want one gone, they are plain Markdown files — delete it in Obsidian, or `rm` it. That is a vault operation, and Obsidian is better at it than a button in WorktreeManager would be.

---

## Two things to get right

**Do not duplicate Linear.** The note holds no issue description and no status. It holds what Linear cannot: what you tried that failed, what you assumed, what took you an hour to understand.

**Never put secrets in the vault.** Vaults get synced. No tokens, no `.env` contents, no credentials.
