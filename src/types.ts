export interface Repo {
  id: string;
  name: string;
  localPath: string;
  worktreeBasePath: string;
  linearApiKey?: string | null;
}

export interface Worktree {
  id: string;
  repoId: string;
  branchName: string;
  linearIssueId?: string;
  linearIssueTitle?: string;
  linearIssueIdentifier?: string;
  path: string;
  createdAt: string;
}

export interface AppState {
  setup: {
    linearApiKey: string | null;
    isComplete: boolean;
  };
  repos: Repo[];
  worktrees: Worktree[];
  selectedRepoId: string | null;
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
  | "neovim-claude";

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
];

export const DEFAULT_STATE: AppState = {
  setup: {
    linearApiKey: null,
    isComplete: false,
  },
  repos: [],
  worktrees: [],
  selectedRepoId: null,
};
