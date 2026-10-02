// @vitest-environment jsdom
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { invoke } from "@tauri-apps/api/core";
import { SettingsModal, SettingsSection } from "./SettingsModal";
import { DEFAULT_AGENT_ALERTS } from "../../services/agentActivity";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/path", () => ({ homeDir: vi.fn().mockResolvedValue("/Users/test") }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

function renderSettings() {
  const onThemeChange = vi.fn();
  const onVaultChange = vi.fn();
  const onAlertsChange = vi.fn();
  const onRecheck = vi.fn();
  const onClose = vi.fn();
  function Harness() {
    const [section, setSection] = useState<SettingsSection | null>("theme");
    return (
      <SettingsModal
        section={section}
        onSectionChange={setSection}
        onClose={() => {
          onClose();
          setSection(null);
        }}
        themeId="default"
        onThemeChange={onThemeChange}
        customColors={null}
        onCustomColorsChange={vi.fn()}
        vault={{ enabled: false, path: null }}
        onVaultChange={onVaultChange}
        onRepairVaultAgents={vi.fn()}
        alerts={DEFAULT_AGENT_ALERTS}
        onAlertsChange={onAlertsChange}
        report={null}
        running={false}
        severity="warning"
        onRecheck={onRecheck}
      />
    );
  }
  render(<Harness />);
  return { onThemeChange, onVaultChange, onAlertsChange, onRecheck, onClose };
}

describe("SettingsModal", () => {
  it("shows one panel at a time and preserves the existing theme and dependency actions", () => {
    const { onThemeChange, onRecheck } = renderSettings();
    expect(screen.getAllByRole("dialog")).toHaveLength(1);
    expect(screen.getAllByRole("tabpanel")).toHaveLength(1);
    fireEvent.click(screen.getByRole("button", { name: "Custom" }));
    expect(onThemeChange).toHaveBeenCalledWith("custom");
    fireEvent.click(screen.getByRole("tab", { name: /Dependencies/ }));
    expect(screen.getAllByRole("tabpanel")).toHaveLength(1);
    expect(screen.queryByRole("button", { name: "Custom" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Re-check" }));
    expect(onRecheck).toHaveBeenCalledOnce();
  });

  it("supports arrow navigation, contains focus and restores it when closed", () => {
    const trigger = document.createElement("button");
    document.body.appendChild(trigger);
    trigger.focus();
    renderSettings();
    const theme = screen.getByRole("tab", { name: "Theme" });
    expect(theme).toHaveFocus();
    fireEvent.keyDown(theme, { key: "ArrowDown" });
    const vault = screen.getByRole("tab", { name: "Obsidian vault" });
    expect(vault).toHaveFocus();
    expect(vault).toHaveAttribute("aria-selected", "true");
    const close = screen.getByRole("button", { name: "Close Settings" });
    const last = screen.getByRole("button", { name: "Enable vault" });
    last.focus();
    fireEvent.keyDown(last, { key: "Tab" });
    expect(close).toHaveFocus();
    fireEvent.keyDown(close, { key: "Tab", shiftKey: true });
    expect(last).toHaveFocus();
    fireEvent.keyDown(last, { key: "Escape" });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(trigger).toHaveFocus();
    trigger.remove();
  });

  it("shows vault failures and allows retry from the same panel", async () => {
    const { onVaultChange } = renderSettings();
    vi.mocked(invoke).mockRejectedValueOnce("Could not register the vault");
    fireEvent.click(screen.getByRole("tab", { name: "Obsidian vault" }));
    fireEvent.click(screen.getByRole("button", { name: "Enable vault" }));
    expect(await screen.findByText("Could not register the vault")).toBeVisible();
    expect(onVaultChange).not.toHaveBeenCalled();
    vi.mocked(invoke).mockResolvedValueOnce("/Users/test/Documents/worktreemanager-vault");
    fireEvent.click(screen.getByRole("button", { name: "Enable vault" }));
    await waitFor(() =>
      expect(onVaultChange).toHaveBeenCalledWith({
        enabled: true,
        path: "/Users/test/Documents/worktreemanager-vault",
      })
    );
  });

  it("loads agent hooks only in Agent alerts and surfaces failures alongside alert controls", async () => {
    const { onAlertsChange } = renderSettings();
    expect(invoke).not.toHaveBeenCalled();
    vi.mocked(invoke).mockRejectedValueOnce("Could not read agent hooks");
    fireEvent.click(screen.getByRole("tab", { name: "Agent alerts" }));
    expect(await screen.findByText("Could not read agent hooks")).toBeVisible();
    fireEvent.change(screen.getByRole("combobox", { name: "Finished sound" }), {
      target: { value: "" },
    });
    expect(onAlertsChange).toHaveBeenCalledWith({
      ...DEFAULT_AGENT_ALERTS,
      sounds: { ...DEFAULT_AGENT_ALERTS.sounds, done: null },
    });
  });
});
