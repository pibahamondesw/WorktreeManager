import { openUrl } from "@tauri-apps/plugin-opener";
import { Task, VaultConfig, Workspace } from "../types";
import { linearIssueUrl } from "../utils";
import { ensureTaskNote, taskNoteUri } from "./notes";

export type TaskActionId =
  | "open-linear"
  | "open-note"
  | "copy-branch"
  | "copy-path"
  | "copy-linear-id";

export interface TaskActionTarget {
  task: Task;
  workspace?: Workspace;
  vault?: VaultConfig;
}

export type TaskActionNotifier = (message: string) => void;

/** Resolves true when the action succeeded; every outcome worth reporting goes through `notify`. */
export async function runTaskAction(
  action: TaskActionId,
  target: TaskActionTarget,
  notify: TaskActionNotifier
): Promise<boolean> {
  const { task } = target;
  switch (action) {
    case "copy-branch":
      return copyText(task.branchName, "Branch name copied", notify);
    case "copy-path":
      return copyTaskPaths(task, notify);
    case "copy-linear-id":
      if (!task.linearIssueIdentifier) return reportMissingLinearIssue(notify);
      return copyText(task.linearIssueIdentifier, "Linear ID copied", notify);
    case "open-linear":
      if (!task.linearIssueIdentifier) return reportMissingLinearIssue(notify);
      return openLink(
        linearIssueUrl(task.linearIssueIdentifier, target.workspace?.linearOrgUrlKey),
        "Could not open Linear",
        notify
      );
    case "open-note":
      return openTaskNote(target, notify);
  }
}

function copyTaskPaths(task: Task, notify: TaskActionNotifier): Promise<boolean> {
  const paths = task.members.map((member) => member.path);
  const message = paths.length > 1 ? "Folder paths copied" : "Worktree path copied";
  return copyText(paths.join("\n"), message, notify);
}

async function copyText(
  text: string,
  successMessage: string,
  notify: TaskActionNotifier
): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    notify("Could not copy to clipboard");
    return false;
  }
  notify(successMessage);
  return true;
}

async function openLink(
  url: string,
  failureMessage: string,
  notify: TaskActionNotifier
): Promise<boolean> {
  try {
    await openUrl(url);
    return true;
  } catch {
    notify(failureMessage);
    return false;
  }
}

/** Creates the note first if it's missing, e.g. when the vault was configured after the task. */
async function openTaskNote(
  { task, workspace, vault }: TaskActionTarget,
  notify: TaskActionNotifier
): Promise<boolean> {
  const notePath = workspace && vault ? await ensureTaskNote(vault, workspace, task) : null;
  if (!notePath) {
    notify("Could not open the task note");
    return false;
  }
  return openLink(taskNoteUri(notePath), "Could not open the note in Obsidian", notify);
}

function reportMissingLinearIssue(notify: TaskActionNotifier): false {
  notify("This task has no Linear issue");
  return false;
}
