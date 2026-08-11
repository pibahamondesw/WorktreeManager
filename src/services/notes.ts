import { invoke } from "@tauri-apps/api/core";
import { Task, Workspace } from "../types";

/**
 * Obsidian task logs: one note per task, living in the workspace's `notesPath`
 * (an Obsidian `task-logs/` folder). See `vault-kit/README.md` for the setup.
 *
 * Every entry point here is a no-op when `notesPath` is unset, and never throws —
 * a note must not be able to break task creation or deletion.
 */

function slugify(input: string): string {
  return input
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

function today(): string {
  return new Date().toISOString().slice(0, 10);
}

function yamlList(values: string[]): string {
  return `[${values.join(", ")}]`;
}

/** Escape a value for a double-quoted YAML scalar. */
function yamlString(value: string): string {
  return `"${value.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
}

/**
 * Filename for a task's note: `<ISSUE-ID>-<branch-slug>.md`, or `<branch-slug>.md`
 * with no Linear issue. Matches `vault-kit/scripts/new-task-note.sh` so the app,
 * the scripts, and the `/task-log` skill all resolve the same file.
 */
export function taskNoteFileName(task: Task): string {
  const branchSlug = slugify(task.branchName.split("/").pop() ?? task.branchName);
  const issueId = task.linearIssueIdentifier?.trim().toUpperCase();
  if (!issueId) return `${branchSlug || "task"}.md`;

  // The branch already starts with the issue ID — don't repeat it in the filename.
  const rest = branchSlug.replace(new RegExp(`^${slugify(issueId)}-*`), "");
  return rest ? `${issueId}-${rest}.md` : `${issueId}.md`;
}

/** The note's initial content. Pure — the app writes this once, at task creation. */
export function buildTaskNote(
  task: Task,
  workspace: Workspace
): { fileName: string; contents: string } {
  const date = today();
  const issueId = task.linearIssueIdentifier?.trim().toUpperCase();
  const title = [issueId, task.linearIssueTitle?.trim() || task.branchName]
    .filter(Boolean)
    .join(" — ");

  const lines = [
    "---",
    `title: ${yamlString(title)}`,
    "type: task-log",
    "status: active",
    `created: ${date}`,
    `updated: ${date}`,
    "tags: []",
    `tickets: ${yamlList(issueId ? [issueId] : [])}`,
    `branch: ${yamlString(task.branchName)}`,
    `workspace: ${yamlString(workspace.name)}`,
    `repos: ${yamlList(task.members.map((m) => m.repoName))}`,
    task.members.length ? "worktrees:" : "worktrees: []",
    ...task.members.flatMap((m) => [`  - repo: ${m.repoName}`, `    path: ${yamlString(m.path)}`]),
    "prs: []",
    "related: []",
    "---",
    "",
    "## Context",
    "",
    "## Decisions",
    "",
    "## Learnings",
    "",
    "## Log",
    "",
  ];

  return { fileName: taskNoteFileName(task), contents: lines.join("\n") };
}

/** Deep link that opens a note in Obsidian. `path` avoids needing the vault name. */
export function taskNoteUri(notePath: string): string {
  return `obsidian://open?path=${encodeURIComponent(notePath)}`;
}

/** Absolute path a task's note would have, or null when notes are off. */
export function taskNotePath(workspace: Workspace, task: Task): string | null {
  const dir = workspace.notesPath?.trim();
  if (!dir) return null;
  return `${dir.replace(/\/+$/, "")}/${taskNoteFileName(task)}`;
}

/**
 * Create the note for a task if it doesn't exist yet. Best-effort: a failure here
 * never surfaces to the user, since the task itself was created fine.
 */
export async function ensureTaskNote(workspace: Workspace, task: Task): Promise<string | null> {
  const notesPath = workspace.notesPath?.trim();
  if (!notesPath) return null;
  const { fileName, contents } = buildTaskNote(task, workspace);
  try {
    return await invoke<string>("ensure_task_note", { notesPath, fileName, contents });
  } catch {
    return null;
  }
}

/** Move a task's note into `_archive/`. Best-effort, for the same reason. */
export async function archiveTaskNote(workspace: Workspace, task: Task): Promise<void> {
  const notesPath = workspace.notesPath?.trim();
  if (!notesPath) return;
  try {
    await invoke("archive_task_note", {
      notesPath,
      fileName: taskNoteFileName(task),
      today: today(),
    });
  } catch {
    /* note archiving is best-effort */
  }
}
