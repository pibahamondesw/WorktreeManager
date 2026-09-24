import { OpenTaskOptions } from "../hooks/useOpenTask";
import { openPrForMember } from "../services/pullRequest";
import { CommandAction, WorkspaceAction } from "./commands";
import { PaletteItem } from "./searchPalette";
import { EditorApp, Task, Workspace } from "../types";

export interface PaletteActionDeps {
  tasks: Task[];
  workspaces: Workspace[];
  onClose: () => void;
  onSelectWorkspace: (workspaceId: string) => void;
  onWorkspaceAction: (action: WorkspaceAction) => void;
  onThemeChange: (themeId: string) => void;
  onEditorChange: (editor: EditorApp) => void;
  onNewTask: () => void;
  onOpenTask: (task: Task, options?: OpenTaskOptions) => Promise<boolean>;
  showToast: (message: string) => void;
}

/** Stay open on failure so the error toast is readable. */
export async function openPaletteTask(task: Task, deps: PaletteActionDeps): Promise<void> {
  const opened = await deps.onOpenTask(task, {
    onMessage: deps.showToast,
    onError: deps.showToast,
  });
  if (opened) deps.onClose();
}

export async function activatePaletteItem(
  item: PaletteItem,
  deps: PaletteActionDeps
): Promise<void> {
  if (item.kind === "task") return openPaletteTask(item.result.task, deps);
  return runPaletteCommand(item.command.action, deps);
}

export async function runPaletteCommand(
  action: CommandAction,
  deps: PaletteActionDeps
): Promise<void> {
  switch (action.type) {
    case "select-workspace":
      deps.onSelectWorkspace(action.workspaceId);
      deps.onClose();
      return;
    case "workspace":
      deps.onWorkspaceAction(action.action);
      deps.onClose();
      return;
    case "set-editor":
      deps.onEditorChange(action.editor);
      deps.onClose();
      return;
    case "set-theme":
      deps.onThemeChange(action.themeId);
      deps.onClose();
      return;
    case "copy-linear":
      await navigator.clipboard.writeText(action.text);
      deps.showToast("Linear ID copied");
      return;
    case "open-pr": {
      const task = deps.tasks.find((t) => t.id === action.taskId);
      const member = task?.members.find((m) => m.repoId === action.repoId);
      const workspace = task && deps.workspaces.find((w) => w.id === task.workspaceId);
      if (!task || !member || !workspace) {
        deps.showToast("Could not open the pull request");
        return;
      }
      const opened = await openPrForMember(task, member, workspace);
      if (opened) deps.onClose();
      else deps.showToast("Could not open the pull request");
      return;
    }
    case "open-claude":
    case "open-codex": {
      const task = deps.tasks.find((t) => t.id === action.taskId);
      if (!task) {
        deps.showToast(`Could not open ${action.type === "open-codex" ? "Codex" : "Claude Code"}`);
        return;
      }
      const opened = await deps.onOpenTask(task, {
        agent: action.type === "open-codex" ? "codex" : "claude",
        onMessage: deps.showToast,
        onError: deps.showToast,
      });
      if (opened) deps.onClose();
      return;
    }
    case "new-task":
      deps.onNewTask();
      deps.onClose();
  }
}
