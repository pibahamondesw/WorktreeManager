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

export type EditorApp = "cursor" | "vscode" | "claude-code" | "opencode";

export const EDITOR_APPS: { id: EditorApp; label: string; isCli: boolean }[] = [
  { id: "cursor", label: "Cursor", isCli: false },
  { id: "vscode", label: "VS Code", isCli: false },
  { id: "opencode", label: "OpenCode", isCli: false },
  { id: "claude-code", label: "Claude Code", isCli: true },
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
