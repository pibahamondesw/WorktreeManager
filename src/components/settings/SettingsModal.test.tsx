// @vitest-environment jsdom
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { SettingsModal, SettingsSection } from "./SettingsModal";
import { saveToolVisibility } from "../chat/toolVisibilityPreference";
import { DEFAULT_AGENT_ALERTS } from "../../services/agentActivity";
import { defaultTypography } from "../../appearance";
import { invokeWithAgentCatalog } from "../../services/agents.fixture";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args?: unknown) => invokeWithAgentCatalog(invoke, command, args),
}));
vi.mock("@tauri-apps/api/path", () => ({ homeDir: vi.fn().mockResolvedValue("/Users/test") }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));

afterEach(() => {
  saveToolVisibility("all");
  cleanup();
  vi.clearAllMocks();
  vi.restoreAllMocks();
});

function renderSettings() {
  const onThemeChange = vi.fn();
  const onVaultChange = vi.fn();
  const onAlertsChange = vi.fn();
  const onKeepRunningInBackgroundChange = vi.fn();
  const onCompactIdleChatsChange = vi.fn();
  const onRecheck = vi.fn();
  const onClose = vi.fn();
  function Harness() {
    const [section, setSection] = useState<SettingsSection | null>("theme");
    const [alerts, setAlerts] = useState(DEFAULT_AGENT_ALERTS);
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
        typography={defaultTypography()}
        onTypographyChange={vi.fn()}
        contrast={100}
        onContrastChange={vi.fn()}
        vault={{ enabled: false, path: null }}
        onVaultChange={onVaultChange}
        onRepairVaultAgents={vi.fn()}
        alerts={alerts}
        onAlertsChange={(next) => {
          setAlerts(next);
          onAlertsChange(next);
        }}
        keepRunningInBackground
        onKeepRunningInBackgroundChange={onKeepRunningInBackgroundChange}
        compactIdleChats
        onCompactIdleChatsChange={onCompactIdleChatsChange}
        report={null}
        running={false}
        severity="warning"
        onRecheck={onRecheck}
      />
    );
  }
  render(<Harness />);
  return {
    onThemeChange,
    onVaultChange,
    onAlertsChange,
    onKeepRunningInBackgroundChange,
    onCompactIdleChatsChange,
    onRecheck,
    onClose,
  };
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
    const typography = screen.getByRole("tab", { name: "Typography" });
    expect(typography).toHaveFocus();
    fireEvent.keyDown(typography, { key: "ArrowDown" });
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

  it("turns off keeping the app running in the background", () => {
    const { onKeepRunningInBackgroundChange } = renderSettings();
    vi.mocked(invoke).mockResolvedValue(null);
    fireEvent.click(screen.getByRole("tab", { name: "Agent alerts" }));
    const toggle = screen.getByRole("checkbox", { name: "Keep running in the background" });
    expect(toggle).toBeChecked();
    fireEvent.click(toggle);
    expect(onKeepRunningInBackgroundChange).toHaveBeenCalledWith(false);
  });

  it("tests notifications when Notify is enabled and opens macOS settings when permission is denied", async () => {
    vi.spyOn(navigator, "userAgent", "get").mockReturnValue("Macintosh");
    invoke
      .mockResolvedValueOnce(null)
      .mockResolvedValueOnce(false)
      .mockResolvedValueOnce(undefined);
    renderSettings();
    fireEvent.click(screen.getByRole("tab", { name: "Agent alerts" }));
    const [notify] = screen.getAllByRole("checkbox", { name: "Notify" });
    fireEvent.click(notify);
    expect(invoke).not.toHaveBeenCalledWith("agent_test_notification", undefined);
    fireEvent.click(notify);
    expect(invoke).toHaveBeenCalledWith("agent_test_notification", undefined);
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("agent_open_notification_settings", undefined)
    );
    expect(screen.getByRole("status")).toHaveTextContent(/Allow notifications for WorktreeManager/);
  });

  it("can retry a failed Mac notification test and open settings when a banner is missed", async () => {
    vi.spyOn(navigator, "userAgent", "get").mockReturnValue("Macintosh");
    invoke.mockResolvedValueOnce(null).mockRejectedValueOnce("Notification Center is unavailable");
    renderSettings();
    fireEvent.click(screen.getByRole("tab", { name: "Agent alerts" }));
    const testNotification = screen.getByRole("button", { name: "Send test notification" });
    fireEvent.click(testNotification);
    expect(await screen.findByText("Notification Center is unavailable")).toBeVisible();
    invoke.mockResolvedValueOnce(true);
    fireEvent.click(testNotification);
    expect(await screen.findByText(/Test notification sent/)).toBeVisible();
    expect(invoke).not.toHaveBeenCalledWith("agent_open_notification_settings", undefined);
    fireEvent.click(screen.getByRole("button", { name: "Open notification settings" }));
    expect(invoke).toHaveBeenCalledWith("agent_open_notification_settings", undefined);
  });

  it("turns off compacting idle Claude chats", () => {
    const { onCompactIdleChatsChange } = renderSettings();
    vi.mocked(invoke).mockResolvedValue(null);
    fireEvent.click(screen.getByRole("tab", { name: "Agent alerts" }));
    const toggle = screen.getByRole("checkbox", {
      name: "Compact idle Claude chats before the cache expires",
    });
    expect(toggle).toBeChecked();
    fireEvent.click(toggle);
    expect(onCompactIdleChatsChange).toHaveBeenCalledWith(false);
  });

  it("shows hooks that report to another app and replaces them only after confirming", async () => {
    const target = "'/dev/target/debug/worktree-manager' --cli --local";
    const status = {
      claude: { state: "otherApp", target },
      codex: { state: "installed" },
    };
    vi.mocked(invoke).mockResolvedValueOnce(status);
    renderSettings();
    fireEvent.click(screen.getByRole("tab", { name: "Agent alerts" }));
    expect(await screen.findByText(target)).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Use this app" }));
    expect(screen.getByText(/stop receiving alerts/)).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByText(/stop receiving alerts/)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Use this app" }));
    expect(invoke).toHaveBeenCalledTimes(1);
    vi.mocked(invoke)
      .mockResolvedValueOnce(undefined)
      .mockResolvedValueOnce({ ...status, claude: { state: "installed" } });
    fireEvent.click(screen.getByRole("button", { name: "Replace hooks" }));
    await waitFor(() => expect(screen.queryByText(target)).toBeNull());
    expect(invoke).toHaveBeenCalledWith("agent_hooks_install", {
      agent: "claude",
      replaceOtherApp: true,
    });
    expect(screen.getAllByRole("button", { name: "Remove hooks" })).toHaveLength(2);
  });
});

it("places preferences last in settings navigation", () => {
  renderSettings();
  expect(screen.getAllByRole("tab").at(-1)).toHaveTextContent("Preferences");
});

it("saves which tool calls every chat shows from preferences", () => {
  renderSettings();
  fireEvent.click(screen.getByRole("tab", { name: "Preferences" }));
  fireEvent.change(screen.getByRole("combobox", { name: "Tool calls in chat" }), {
    target: { value: "zen" },
  });
  expect(screen.getByRole("combobox", { name: "Tool calls in chat" })).toHaveValue("zen");
  expect(localStorage.getItem("worktreemanager.toolVisibility")).toBe("zen");
});
