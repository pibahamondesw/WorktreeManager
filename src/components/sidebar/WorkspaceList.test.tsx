// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import "@testing-library/jest-dom/vitest";
import { WorkspaceList } from "./WorkspaceList";
import { Task, Workspace } from "../../types";

vi.mock("./AddWorkspaceModal", () => ({ AddWorkspaceModal: () => null }));
vi.mock("./VaultSettingsModal", () => ({ VaultSettingsModal: () => null }));

const workspaces: Workspace[] = [
  { id: "ws-active", name: "Active workspace", repos: [] },
  { id: "ws-idle", name: "Idle workspace", repos: [] },
];

const task: Task = {
  id: "task-active",
  workspaceId: "ws-active",
  branchName: "feature/active",
  members: [],
  createdAt: "2026-09-21T00:00:00Z",
};

afterEach(cleanup);

describe("WorkspaceList session indicator", () => {
  it("marks only workspaces containing an active session", () => {
    render(
      <WorkspaceList
        workspaces={workspaces}
        tasks={[task]}
        agentSessions={{ "task-active": { kind: "running" } }}
        selectedWorkspaceId="ws-idle"
        onSelect={vi.fn()}
        onAdd={vi.fn()}
        onUpdate={vi.fn()}
        onRemove={vi.fn()}
        onReorder={vi.fn()}
        vault={{ enabled: false, path: null }}
        onCollapse={vi.fn()}
        doctorSeverity={null}
        onOpenSettings={vi.fn()}
        onOpenAutomations={vi.fn()}
        automationsNeedAttention={false}
      />
    );

    expect(screen.getByTitle("Active agent session in this workspace")).toBeInTheDocument();
    expect(screen.getByText("Active workspace").parentElement).toContainElement(
      screen.getByTitle("Active agent session in this workspace")
    );
    expect(screen.getByText("Idle workspace").parentElement).not.toContainElement(
      screen.getByTitle("Active agent session in this workspace")
    );
  });

  it("shows a red error above waiting and working agents", () => {
    render(
      <WorkspaceList
        workspaces={workspaces}
        tasks={[task, { ...task, id: "waiting" }, { ...task, id: "working" }]}
        agentSessions={{ "task-active": { kind: "running", hasError: true } }}
        agentActivities={{
          waiting: { state: "waiting", agent: "claude", surface: "chat", unread: true, at: 1 },
          working: { state: "working", agent: "codex", surface: "chat", unread: false, at: 1 },
        }}
        selectedWorkspaceId="ws-idle"
        onSelect={vi.fn()}
        onAdd={vi.fn()}
        onUpdate={vi.fn()}
        onRemove={vi.fn()}
        onReorder={vi.fn()}
        vault={{ enabled: false, path: null }}
        onCollapse={vi.fn()}
        doctorSeverity={null}
        onOpenSettings={vi.fn()}
        onOpenAutomations={vi.fn()}
        automationsNeedAttention={false}
      />
    );
    expect(
      screen.getByTitle("An agent in this workspace has an error that requires attention")
    ).toHaveClass("bg-danger");
    expect(
      screen.queryByTitle("An agent in this workspace needs your input")
    ).not.toBeInTheDocument();
  });

  it("rolls agent attention up to the workspace", () => {
    render(
      <WorkspaceList
        workspaces={workspaces}
        tasks={[task, { ...task, id: "task-working" }]}
        agentActivities={{
          "task-working": {
            state: "working",
            agent: "codex",
            surface: "chat",
            unread: false,
            at: 1,
          },
          "task-active": {
            state: "waiting",
            agent: "claude",
            surface: "editor",
            unread: true,
            at: 1,
          },
        }}
        selectedWorkspaceId="ws-idle"
        onSelect={vi.fn()}
        onAdd={vi.fn()}
        onUpdate={vi.fn()}
        onRemove={vi.fn()}
        onReorder={vi.fn()}
        vault={{ enabled: false, path: null }}
        onCollapse={vi.fn()}
        doctorSeverity={null}
        onOpenSettings={vi.fn()}
        onOpenAutomations={vi.fn()}
        automationsNeedAttention={false}
      />
    );

    expect(screen.getByText("Active workspace").parentElement).toContainElement(
      screen.getByTitle("An agent in this workspace needs your input")
    );
    expect(screen.getByTitle("An agent in this workspace needs your input")).toHaveClass(
      "bg-warning"
    );
    expect(screen.queryByTitle("An agent in this workspace is working")).not.toBeInTheDocument();
  });
});

describe("WorkspaceList automations entry point", () => {
  const renderList = (automationsNeedAttention: boolean, onOpenAutomations = vi.fn()) =>
    render(
      <WorkspaceList
        workspaces={workspaces}
        tasks={[]}
        selectedWorkspaceId="ws-idle"
        onSelect={vi.fn()}
        onAdd={vi.fn()}
        onUpdate={vi.fn()}
        onRemove={vi.fn()}
        onReorder={vi.fn()}
        vault={{ enabled: false, path: null }}
        onCollapse={vi.fn()}
        doctorSeverity={null}
        onOpenSettings={vi.fn()}
        onOpenAutomations={onOpenAutomations}
        automationsNeedAttention={automationsNeedAttention}
      />
    );

  it("opens automations from the footer and flags them only when they need attention", () => {
    const onOpenAutomations = vi.fn();
    renderList(false, onOpenAutomations);
    expect(screen.queryByText("Automations", { selector: "h2" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Automations" }));
    expect(onOpenAutomations).toHaveBeenCalledOnce();

    cleanup();
    renderList(true);
    expect(screen.getByRole("button", { name: "Automations (needs attention)" })).toContainElement(
      document.querySelector(".bg-warning") as HTMLElement
    );
  });
});
