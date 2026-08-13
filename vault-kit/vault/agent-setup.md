# Engineering vault — agent setup

This vault holds **projects** (investigations, plans, decisions that span tickets or repos) and **task logs** (one note per WorktreeManager task — a branch and its worktrees). Source repos live in their own locations; work happens in worktrees.

**At the start of every session, before substantive work:**

1. **Read** this vault's `AGENTS.md` (conventions + vault guide) if the session hasn't loaded it already — do this even when the working directory is elsewhere.
2. **Small, self-contained task?** Work directly in the repo. If the worktree was created by WorktreeManager, it has a task log in this vault's `task-logs/` folder (named after the branch/issue) — record decisions and learnings there as they happen, following the guide's "Task logs" section.
3. **Multi-ticket, cross-repo, or research-heavy task?** Open or create a project in the vault:

   ```bash
   bash <vault>/scripts/new-project.sh <slug> [--tickets PROJ-1,PROJ-2]
   ```

   and file investigations, plans, and decisions there as the work happens, following the vault guide's naming and frontmatter conventions.

The two layers interlock: a **project** spans tickets and repos, a **task log** is one branch. Link them with `related:` on both sides, and put anything that outlives the branch in the project — task logs get archived when their worktree is deleted.

**Distilled, not a transcript** — vault notes carry the conclusion, the decision and why, the finding that matters; never a chat log. Code stays in its repo; the vault only references it (`file:line`).

---

## Installing this snippet

Make your AI tool load this file (or its contents) globally, once, replacing `<vault>` with this vault's absolute path:

| Tool | How |
| --- | --- |
| Claude Code | Add a line `@<vault>/agent-setup.md` to `~/.claude/CLAUDE.md` |
| Codex CLI | Paste this file's contents into `~/.codex/AGENTS.md` |
| Cursor | Paste into a global rule (Settings → Rules) |
| Other | Any tool that supports global/user instructions: paste this file's contents there |

Claude Code users can optionally also install the `/task-log` slash command: copy `skills/task-log/` from the [WorktreeManager repo](https://github.com/pibahamondesw/WorktreeManager/tree/main/vault-kit) to `~/.claude/skills/task-log/`. It packages the task-log writing rules as an explicit command; with this snippet installed it's a convenience, not a requirement.
