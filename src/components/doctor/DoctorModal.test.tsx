// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { DoctorModal } from "./DoctorModal";
import { DoctorReport } from "../../services/doctor";

vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));
vi.mock("../editor/EditorInstallation", () => ({ EditorInstallation: () => null }));
afterEach(cleanup);

const report: DoctorReport = {
  errors: 0,
  warnings: 1,
  checks: [
    {
      id: "codex-vault",
      label: "Codex Obsidian setup",
      scope: "app",
      status: "missing",
      severity: "warning",
      reason: "Loads vault instructions",
      detail: "No global vault block",
      repair: "codex",
    },
  ],
};

describe("DoctorModal Codex setup", () => {
  it("repairs Claude independently and displays its error only on its own finding", async () => {
    const onRepairVaultAgents = vi.fn().mockRejectedValue(new Error("Claude permission denied"));
    const both: DoctorReport = {
      ...report,
      warnings: 2,
      checks: [
        report.checks[0],
        {
          ...report.checks[0],
          id: "claude-vault",
          label: "Claude Code Obsidian setup",
          repair: "claude",
        },
      ],
    };
    render(
      <DoctorModal
        open
        onClose={vi.fn()}
        report={both}
        running={false}
        onRecheck={vi.fn()}
        onRepairVaultAgents={onRepairVaultAgents}
      />
    );
    fireEvent.click(screen.getByRole("button", { name: "Repair Claude Code setup" }));
    expect(await screen.findByText("Claude permission denied")).toBeInTheDocument();
    expect(screen.getAllByText("Claude permission denied")).toHaveLength(1);
    expect(onRepairVaultAgents).toHaveBeenCalledExactlyOnceWith("claude");
  });
  it("surfaces repair errors and allows retrying", async () => {
    const onRepairVaultAgents = vi.fn().mockRejectedValueOnce(new Error("Permission denied"));
    render(
      <DoctorModal
        open
        onClose={vi.fn()}
        report={report}
        running={false}
        onRecheck={vi.fn()}
        onRepairVaultAgents={onRepairVaultAgents}
      />
    );
    fireEvent.click(screen.getByRole("button", { name: "Repair Codex setup" }));
    expect(await screen.findByText("Permission denied")).toBeInTheDocument();
    onRepairVaultAgents.mockResolvedValueOnce(undefined);
    fireEvent.click(screen.getByRole("button", { name: "Repair Codex setup" }));
    await waitFor(() => expect(screen.queryByText("Permission denied")).not.toBeInTheDocument());
    expect(onRepairVaultAgents).toHaveBeenCalledTimes(2);
    expect(onRepairVaultAgents).toHaveBeenLastCalledWith("codex");
  });

  it("removes the repair action when a refreshed report is healthy", () => {
    const props = {
      open: true,
      onClose: vi.fn(),
      running: false,
      onRecheck: vi.fn(),
      onRepairVaultAgents: vi.fn(),
    };
    const { rerender } = render(<DoctorModal {...props} report={report} />);
    rerender(
      <DoctorModal
        {...props}
        report={{
          ...report,
          warnings: 0,
          checks: [{ ...report.checks[0], status: "ok", severity: "ok" }],
        }}
      />
    );
    expect(screen.queryByRole("button", { name: "Repair Codex setup" })).not.toBeInTheDocument();
  });
});
