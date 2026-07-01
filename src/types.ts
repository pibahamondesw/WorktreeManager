/** A peer repo that belongs to a Workspace. No member is privileged. */
export interface WorkspaceRepo {
  id: string;
  name: string;
  localPath: string;
  worktreeBasePath: string;
}

/** A named group of peer repos opened together. Replaces the old single-repo "Repo"/project. */
export interface Workspace {
  id: string;
  name: string;
  repos: WorkspaceRepo[];
  linearApiKey?: string | null;
}

/** One repo's worktree within a task. */
export interface TaskMember {
  repoId: string;
  repoName: string;
  /** The member repo's main clone (used for git operations on the worktree). */
  localPath: string;
  /** The worktree directory opened for this member. */
  path: string;
  branchName: string;
}

/**
 * A unit of work spanning one or more member repos on a shared branch.
 * Replaces the old single-repo "Worktree". Branch + Linear issue live at the task
 * level; each member is a per-repo worktree so no repo is hoisted to the top.
 */
export interface Task {
  id: string;
  workspaceId: string;
  branchName: string;
  linearIssueId?: string;
  linearIssueTitle?: string;
  linearIssueIdentifier?: string;
  members: TaskMember[];
  /** Generated .code-workspace file (Cursor/VS Code), when applicable. */
  workspaceFilePath?: string | null;
  createdAt: string;
}

export interface AppState {
  setup: {
    linearApiKey: string | null;
    isComplete: boolean;
  };
  workspaces: Workspace[];
  tasks: Task[];
  selectedWorkspaceId: string | null;
}

export interface LinearIssue {
  id: string;
  identifier: string;
  title: string;
  branchName: string;
  description?: string;
  projectName?: string;
  stateName?: string;
  stateType?: string;
  priority: number;
  updatedAt: string;
}

export interface WorktreeGitInfo {
  path: string;
  branch: string;
  head: string;
  bare: boolean;
}

export interface GitStatus {
  ahead: number;
  behind: number;
  dirty: boolean;
  last_commit_epoch: number;
}

export interface PullRequestInfo {
  url: string;
  title: string;
  state: string;
  number: number;
}

export interface IssueLinearInfo {
  status: { name: string; type: string } | null;
  pr: PullRequestInfo | null;
}

export type EditorApp =
  | "cursor"
  | "vscode"
  | "cursor-claude"
  | "vscode-claude"
  | "claude-code"
  | "opencode"
  | "neovim"
  | "neovim-claude"
  | "zed"
  | "zed-claude";

/**
 * Editor-specific local config directories carried into a new worktree.
 * `git worktree add` only checks out tracked files, so gitignored editor config
 * would be lost. Only the dir for the editor in use is copied.
 */
export const EDITOR_CONFIG_PATHS: Record<EditorApp, string[]> = {
  cursor: [".cursor"],
  vscode: [".vscode"],
  "cursor-claude": [".cursor", ".claude"],
  "vscode-claude": [".vscode", ".claude"],
  "claude-code": [".claude"],
  opencode: [".opencode"],
  neovim: [],
  "neovim-claude": [".claude"],
  zed: [".zed"],
  "zed-claude": [".zed", ".claude"],
};

/** Editor-agnostic local config copied into every new worktree. */
export const ALWAYS_COPIED_CONFIG_PATHS: string[] = [
  ".env",
  ".env.local",
  ".env.development",
  ".env.development.local",
];

export const EDITOR_APPS: { id: EditorApp; label: string; isCli: boolean }[] = [
  { id: "cursor", label: "Cursor", isCli: false },
  { id: "vscode", label: "VS Code", isCli: false },
  { id: "cursor-claude", label: "Cursor + Claude", isCli: false },
  { id: "vscode-claude", label: "VS Code + Claude", isCli: false },
  { id: "opencode", label: "OpenCode", isCli: false },
  { id: "claude-code", label: "Claude Code", isCli: true },
  { id: "neovim", label: "Neovim", isCli: true },
  { id: "neovim-claude", label: "Neovim + Claude", isCli: true },
  { id: "zed", label: "Zed", isCli: false },
  { id: "zed-claude", label: "Zed + Claude", isCli: false },
];

export const DEFAULT_STATE: AppState = {
  setup: {
    linearApiKey: null,
    isComplete: false,
  },
  workspaces: [],
  tasks: [],
  selectedWorkspaceId: null,
};
