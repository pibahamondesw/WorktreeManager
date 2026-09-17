import { CUSTOM_THEME_ID, themes } from "../themes";
import { EditorApp, EDITOR_APPS, Task, Workspace } from "../types";

export type WorkspaceAction =
  | { kind: "vault" }
  | { kind: "themes" }
  | { kind: "edit"; workspaceId: string }
  | { kind: "remove"; workspaceId: string };

export type CommandAction =
  | { type: "select-workspace"; workspaceId: string }
  | { type: "workspace"; action: WorkspaceAction }
  | { type: "set-editor"; editor: EditorApp }
  | { type: "set-theme"; themeId: string }
  | { type: "copy-linear"; text: string }
  | { type: "open-pr"; taskId: string; repoId: string }
  | { type: "open-claude"; taskId: string }
  | { type: "open-codex"; taskId: string }
  | { type: "new-task" };

export type CommandGroup = "workspace" | "settings" | "action";

export interface CommandShortcut {
  key: string;
  label: string;
  meta?: boolean;
  shift?: boolean;
}

export interface PaletteCommand {
  id: string;
  label: string;
  /** Right-aligned context (workspace name, "current", Linear ID). */
  hint?: string;
  group: CommandGroup;
  /** Lowercased blob scored against free-text terms. */
  keywords: string;
  /** Shown on a blank query (after tasks) so the palette is discoverable. */
  emptyVisible: boolean;
  shortcut?: CommandShortcut;
  action: CommandAction;
}

interface BuildCommandsArgs {
  workspaces: Workspace[];
  selectedWorkspaceId: string | null;
  tasks: Task[];
  themeId: string;
  editorApp: EditorApp;
}

function workspaceHint(
  workspace: Workspace | undefined,
  selectedWorkspaceId: string | null
): string | undefined {
  if (!workspace) return undefined;
  return workspace.id === selectedWorkspaceId ? "this workspace" : workspace.name;
}

function taskLabel(task: Task): string {
  return task.linearIssueIdentifier || task.linearIssueTitle || task.branchName;
}

/** Every command the palette can run, ranked later by `searchPalette`. */
export function buildCommands({
  workspaces,
  selectedWorkspaceId,
  tasks,
  themeId,
  editorApp,
}: BuildCommandsArgs): PaletteCommand[] {
  const commands: PaletteCommand[] = [];
  const workspaceById = new Map(workspaces.map((w) => [w.id, w]));
  const current = workspaces.find((w) => w.id === selectedWorkspaceId);

  if (current) {
    commands.push({
      id: "new-task",
      label: "New task",
      group: "action",
      keywords: `new task create ${current.name}`.toLowerCase(),
      emptyVisible: true,
      shortcut: { key: "n", label: "N" },
      action: { type: "new-task" },
    });
  }

  for (const [index, workspace] of workspaces.entries()) {
    const isCurrent = workspace.id === selectedWorkspaceId;
    commands.push({
      id: `switch-ws:${workspace.id}`,
      label: `Switch to ${workspace.name}`,
      hint: isCurrent ? "current" : undefined,
      group: "workspace",
      keywords: `switch workspace change ${workspace.name}`.toLowerCase(),
      emptyVisible: !isCurrent,
      shortcut: index <= 9 ? { key: String(index), label: `⌘${index}`, meta: true } : undefined,
      action: { type: "select-workspace", workspaceId: workspace.id },
    });
    commands.push({
      id: `edit-ws:${workspace.id}`,
      label: `Edit ${workspace.name}`,
      hint: isCurrent ? "current" : undefined,
      group: "workspace",
      keywords: `edit workspace settings ${workspace.name}`.toLowerCase(),
      emptyVisible: isCurrent,
      action: { type: "workspace", action: { kind: "edit", workspaceId: workspace.id } },
    });
    commands.push({
      id: `remove-ws:${workspace.id}`,
      label: `Remove ${workspace.name}`,
      hint: isCurrent ? "current" : undefined,
      group: "workspace",
      keywords: `remove delete workspace ${workspace.name}`.toLowerCase(),
      emptyVisible: isCurrent,
      action: { type: "workspace", action: { kind: "remove", workspaceId: workspace.id } },
    });
  }

  commands.push({
    id: "vault",
    label: "Obsidian vault",
    hint: "settings",
    group: "settings",
    keywords: "obsidian vault notes settings",
    emptyVisible: true,
    action: { type: "workspace", action: { kind: "vault" } },
  });

  commands.push({
    id: "theme-picker",
    label: "Theme",
    hint: "picker",
    group: "settings",
    keywords: "theme appearance color picker",
    emptyVisible: true,
    action: { type: "workspace", action: { kind: "themes" } },
  });

  const themeOptions = [
    ...themes.map((t) => ({ id: t.id, name: t.name })),
    { id: CUSTOM_THEME_ID, name: "Custom" },
  ];
  for (const theme of themeOptions) {
    const isCurrent = theme.id === themeId;
    commands.push({
      id: `theme:${theme.id}`,
      label: `Theme: ${theme.name}`,
      hint: isCurrent ? "current" : undefined,
      group: "settings",
      keywords: `theme appearance ${theme.name}`.toLowerCase(),
      emptyVisible: false,
      action: { type: "set-theme", themeId: theme.id },
    });
  }

  for (const editor of EDITOR_APPS) {
    const isCurrent = editor.id === editorApp;
    commands.push({
      id: `editor:${editor.id}`,
      label: `Editor: ${editor.label}`,
      hint: isCurrent ? "current" : undefined,
      group: "settings",
      keywords: `editor picker change ${editor.label} ${editor.id}`.toLowerCase(),
      emptyVisible: true,
      action: { type: "set-editor", editor: editor.id },
    });
  }

  for (const task of tasks) {
    const workspace = workspaceById.get(task.workspaceId);
    const hint = workspaceHint(workspace, selectedWorkspaceId);
    const label = taskLabel(task);
    const taskKeywords = [
      task.linearIssueIdentifier,
      task.linearIssueTitle,
      task.branchName,
      workspace?.name,
    ]
      .filter(Boolean)
      .join(" ")
      .toLowerCase();

    if (task.linearIssueIdentifier) {
      commands.push({
        id: `copy-linear:${task.id}`,
        label: `Copy Linear ID · ${task.linearIssueIdentifier}`,
        hint,
        group: "action",
        keywords: `copy linear id identifier ${taskKeywords}`,
        emptyVisible: false,
        action: { type: "copy-linear", text: task.linearIssueIdentifier },
      });
    }

    const showRepo = task.members.length > 1;
    for (const member of task.members) {
      commands.push({
        id: `open-pr:${task.id}:${member.repoId}`,
        label: showRepo ? `Open PR · ${label} · ${member.repoName}` : `Open PR · ${label}`,
        hint,
        group: "action",
        keywords:
          `open pr pull request create github ${member.repoName} ${taskKeywords}`.toLowerCase(),
        emptyVisible: false,
        action: { type: "open-pr", taskId: task.id, repoId: member.repoId },
      });
    }

    commands.push({
      id: `open-codex:${task.id}`,
      label: `Open in Codex · ${label}`,
      hint,
      group: "action",
      keywords: `open codex terminal agent ${taskKeywords}`,
      emptyVisible: false,
      action: { type: "open-codex", taskId: task.id },
    });

    commands.push({
      id: `open-claude:${task.id}`,
      label: `Open in Claude Code · ${label}`,
      hint,
      group: "action",
      keywords: `open in claude code ${taskKeywords}`,
      emptyVisible: false,
      action: { type: "open-claude", taskId: task.id },
    });
  }

  return commands;
}
