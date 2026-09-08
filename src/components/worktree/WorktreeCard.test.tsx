// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));
vi.mock("../../services/notes", () => ({
  archiveTaskNote: vi.fn(),
  ensureTaskNote: vi.fn(),
  taskNoteUri: vi.fn(),
}));

import { WorktreeCard } from "./WorktreeCard";
import { Task, VaultConfig, Workspace } from "../../types";

const workspace = { id: "ws1", name: "Payments", repos: [] } as unknown as Workspace;
const vault = { enabled: false } as VaultConfig;
const task = {
  id: "t1",
  workspaceId: "ws1",
  branchName: "feat/x",
  members: [{ repoId: "r1", repoName: "api", localPath: "/repo", path: "/wt", branchName: "feat/x" }],
  createdAt: "2026-01-01T00:00:00Z",
} as unknown as Task;

afterEach(cleanup);

describe("WorktreeCard session indicator", () => {
  it("shows a running dot when the task has a live agent session", () => {
    render(
      <WorktreeCard
        task={task}
        workspace={workspace}
        vault={vault}
        onDelete={vi.fn()}
        repoSlugs={{}}
        sessionStatus={{ kind: "running" }}
      />
    );
    expect(screen.getByTitle("Agent session running")).toBeInTheDocument();
  });

  it("shows nothing without a session", () => {
    render(<WorktreeCard
        task={task}
        workspace={workspace}
        vault={vault}
        onDelete={vi.fn()}
        repoSlugs={{}}
      />);
    expect(screen.queryByTitle(/Agent session/)).not.toBeInTheDocument();
  });
});
