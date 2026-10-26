// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Task, VaultConfig, Workspace } from "../types";
import { ensureTaskNote } from "./notes";
import { runTaskAction, TaskActionId, TaskActionTarget } from "./taskActions";

vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));
vi.mock("./notes", () => ({
  ensureTaskNote: vi.fn(),
  taskNoteUri: (notePath: string) => `obsidian://open?path=${notePath}`,
}));

const writeText = vi.fn();
Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });

const member = { repoId: "r1", repoName: "api", localPath: "/repo/api", branchName: "feat/x" };
const task = {
  id: "t1",
  workspaceId: "ws1",
  branchName: "feat/x",
  linearIssueIdentifier: "WOR-190",
  members: [{ ...member, path: "/wt/api" }],
  createdAt: "2026-01-01T00:00:00Z",
} as Task;
const multiRepoTask = {
  ...task,
  members: [...task.members, { ...member, repoId: "r2", repoName: "web", path: "/wt/web" }],
};
const workspace = { id: "ws1", name: "Payments", repos: [], linearOrgUrlKey: "acme" } as Workspace;
const vault = { enabled: true } as VaultConfig;
const target: TaskActionTarget = { task, workspace, vault };

type Effect = "clipboard" | "openUrl";

interface Case {
  action: TaskActionId;
  target?: TaskActionTarget;
  setup?: () => void;
  effect?: [Effect, string];
  message: string;
  succeeded: boolean;
}

const cases: Record<string, Case> = {
  "copies the branch name": {
    action: "copy-branch",
    effect: ["clipboard", "feat/x"],
    message: "Branch name copied",
    succeeded: true,
  },
  "copies a single worktree path": {
    action: "copy-path",
    effect: ["clipboard", "/wt/api"],
    message: "Worktree path copied",
    succeeded: true,
  },
  "copies every folder path of a multi-repo task": {
    action: "copy-path",
    target: { ...target, task: multiRepoTask },
    effect: ["clipboard", "/wt/api\n/wt/web"],
    message: "Folder paths copied",
    succeeded: true,
  },
  "copies the Linear ID": {
    action: "copy-linear-id",
    effect: ["clipboard", "WOR-190"],
    message: "Linear ID copied",
    succeeded: true,
  },
  "reports a clipboard failure": {
    action: "copy-branch",
    setup: () => writeText.mockRejectedValueOnce(new Error("denied")),
    message: "Could not copy to clipboard",
    succeeded: false,
  },
  "opens the issue in the workspace's Linear org": {
    action: "open-linear",
    effect: ["openUrl", "https://linear.app/acme/issue/WOR-190"],
    message: "",
    succeeded: true,
  },
  "reports a Linear open failure": {
    action: "open-linear",
    setup: () => vi.mocked(openUrl).mockRejectedValueOnce(new Error("no browser")),
    message: "Could not open Linear",
    succeeded: false,
  },
  "refuses Linear actions on a task without an issue": {
    action: "open-linear",
    target: { ...target, task: { ...task, linearIssueIdentifier: undefined } },
    message: "This task has no Linear issue",
    succeeded: false,
  },
  "opens the task note": {
    action: "open-note",
    setup: () => vi.mocked(ensureTaskNote).mockResolvedValueOnce("/vault/t1.md"),
    effect: ["openUrl", "obsidian://open?path=/vault/t1.md"],
    message: "",
    succeeded: true,
  },
  "reports a note that could not be created": {
    action: "open-note",
    setup: () => vi.mocked(ensureTaskNote).mockResolvedValueOnce(null),
    message: "Could not open the task note",
    succeeded: false,
  },
  "reports a note without a workspace": {
    action: "open-note",
    target: { task, vault },
    message: "Could not open the task note",
    succeeded: false,
  },
  "reports an Obsidian open failure": {
    action: "open-note",
    setup: () => {
      vi.mocked(ensureTaskNote).mockResolvedValueOnce("/vault/t1.md");
      vi.mocked(openUrl).mockRejectedValueOnce(new Error("no Obsidian"));
    },
    message: "Could not open the note in Obsidian",
    succeeded: false,
  },
};

beforeEach(() => {
  writeText.mockReset().mockResolvedValue(undefined);
  vi.mocked(openUrl).mockReset().mockResolvedValue(undefined);
  vi.mocked(ensureTaskNote).mockReset();
});

describe("runTaskAction", () => {
  it.each(Object.entries(cases))("%s", async (_, testCase) => {
    testCase.setup?.();
    const notify = vi.fn();

    const succeeded = await runTaskAction(testCase.action, testCase.target ?? target, notify);

    expect(succeeded).toBe(testCase.succeeded);
    if (testCase.message) expect(notify).toHaveBeenCalledExactlyOnceWith(testCase.message);
    else expect(notify).not.toHaveBeenCalled();
    const [effect, value] = testCase.effect ?? [];
    if (effect === "clipboard") expect(writeText).toHaveBeenCalledWith(value);
    if (effect === "openUrl") expect(openUrl).toHaveBeenCalledWith(value);
    if (!testCase.succeeded && !testCase.setup) {
      expect(writeText).not.toHaveBeenCalled();
      expect(openUrl).not.toHaveBeenCalled();
    }
  });
});
