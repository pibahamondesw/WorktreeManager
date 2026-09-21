// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Task, Workspace } from "../types";
import { PaletteActionDeps, runPaletteCommand } from "./runCommand";

const mocks = vi.hoisted(() => ({ openPrForMember: vi.fn() }));
vi.mock("../services/pullRequest", () => ({ openPrForMember: mocks.openPrForMember }));

const task: Task = {
  id: "t-1",
  workspaceId: "ws-1",
  branchName: "feat/x",
  members: [
    {
      repoId: "r1",
      repoName: "web",
      localPath: "/repos/web",
      path: "/wt/web",
      branchName: "feat/x",
    },
  ],
  createdAt: "2026-01-01T00:00:00Z",
};

const workspace: Workspace = { id: "ws-1", name: "Payments", repos: [] };

function deps(overrides: Partial<PaletteActionDeps> = {}): PaletteActionDeps {
  return {
    tasks: [task],
    workspaces: [workspace],
    onClose: vi.fn(),
    onSelectWorkspace: vi.fn(),
    onWorkspaceAction: vi.fn(),
    onThemeChange: vi.fn(),
    onEditorChange: vi.fn(),
    onNewTask: vi.fn(),
    onOpenTask: vi.fn().mockResolvedValue(true),
    showToast: vi.fn(),
    ...overrides,
  };
}

beforeEach(() => {
  mocks.openPrForMember.mockReset().mockResolvedValue(true);
});

describe("runPaletteCommand", () => {
  it("closes after a navigation command", async () => {
    const d = deps();
    await runPaletteCommand({ type: "select-workspace", workspaceId: "ws-1" }, d);
    expect(d.onSelectWorkspace).toHaveBeenCalledWith("ws-1");
    expect(d.onClose).toHaveBeenCalledOnce();
  });

  it.each([
    ["open-claude", "claude"],
    ["open-codex", "codex"],
  ] as const)("opens the requested agent for %s", async (type, agent) => {
    const d = deps();
    await runPaletteCommand({ type, taskId: task.id }, d);
    expect(d.onOpenTask).toHaveBeenCalledWith(task, {
      surface: { kind: "terminal", agent },
      onMessage: d.showToast,
      onError: d.showToast,
    });
    expect(d.onClose).toHaveBeenCalledOnce();
  });

  it("keeps the palette open if the Codex task no longer exists", async () => {
    const d = deps();
    await runPaletteCommand({ type: "open-codex", taskId: "missing" }, d);
    expect(d.onOpenTask).not.toHaveBeenCalled();
    expect(d.showToast).toHaveBeenCalledWith("Could not open Codex");
    expect(d.onClose).not.toHaveBeenCalled();
  });

  it("copies without closing", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });
    const d = deps();
    await runPaletteCommand({ type: "copy-linear", text: "WOR-67" }, d);
    expect(writeText).toHaveBeenCalledWith("WOR-67");
    expect(d.showToast).toHaveBeenCalledWith("Linear ID copied");
    expect(d.onClose).not.toHaveBeenCalled();
  });

  it("opens a matching PR and closes", async () => {
    const d = deps();
    await runPaletteCommand({ type: "open-pr", taskId: "t-1", repoId: "r1" }, d);
    expect(mocks.openPrForMember).toHaveBeenCalledWith(task, task.members[0], workspace);
    expect(d.onClose).toHaveBeenCalledOnce();
  });

  it("stays open when the PR target is missing", async () => {
    const d = deps();
    await runPaletteCommand({ type: "open-pr", taskId: "missing", repoId: "r1" }, d);
    expect(d.showToast).toHaveBeenCalledWith("Could not open the pull request");
    expect(d.onClose).not.toHaveBeenCalled();
  });
});
