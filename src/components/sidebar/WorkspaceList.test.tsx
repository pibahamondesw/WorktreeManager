// @vitest-environment jsdom
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import "@testing-library/jest-dom/vitest";
import { WorkspaceList } from "./WorkspaceList";
import { Task, Workspace } from "../../types";
import { DEFAULT_AGENT_ALERTS } from "../../services/agentActivity";

vi.mock("./AddWorkspaceModal", () => ({ AddWorkspaceModal: () => null }));
vi.mock("./VaultSettingsModal", () => ({ VaultSettingsModal: () => null }));
vi.mock("./AgentAlertsModal", () => ({ AgentAlertsModal: () => null }));

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
        themeId="default"
        onThemeChange={vi.fn()}
        customColors={null}
        onCustomColorsChange={vi.fn()}
        vault={{ enabled: false, path: null }}
        onVaultChange={vi.fn()}
        onCollapse={vi.fn()}
        doctorSeverity={null}
        onOpenDoctor={vi.fn()}
        agentAlerts={DEFAULT_AGENT_ALERTS}
        onAgentAlertsChange={vi.fn()}
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
        themeId="default"
        onThemeChange={vi.fn()}
        customColors={null}
        onCustomColorsChange={vi.fn()}
        vault={{ enabled: false, path: null }}
        onVaultChange={vi.fn()}
        onCollapse={vi.fn()}
        doctorSeverity={null}
        onOpenDoctor={vi.fn()}
        agentAlerts={DEFAULT_AGENT_ALERTS}
        onAgentAlertsChange={vi.fn()}
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
