// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { invoke } from "@tauri-apps/api/core";
import { VaultSettingsModal } from "./VaultSettingsModal";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/path", () => ({ homeDir: vi.fn().mockResolvedValue("/Users/test") }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("VaultSettingsModal", () => {
  it("repairs Codex for an existing vault and keeps failures actionable", async () => {
    const onRepairAgents = vi.fn().mockRejectedValueOnce(new Error("Permission denied"));
    render(
      <VaultSettingsModal
        open
        onClose={vi.fn()}
        vault={{ enabled: true, path: "/custom/vault" }}
        onVaultChange={vi.fn()}
        onRepairAgents={onRepairAgents}
      />
    );
    fireEvent.click(screen.getByRole("button", { name: "Repair agent setup" }));
    expect(await screen.findByText("Permission denied")).toBeInTheDocument();
    expect(screen.getByText(/Codex and Claude Code: vault instructions/)).toBeInTheDocument();
    onRepairAgents.mockResolvedValueOnce(undefined);
    fireEvent.click(screen.getByRole("button", { name: "Repair agent setup" }));
    await waitFor(() => expect(screen.queryByText("Permission denied")).not.toBeInTheDocument());
    expect(onRepairAgents).toHaveBeenCalledTimes(2);
  });

  it("re-enables a custom vault at its existing location", async () => {
    vi.mocked(invoke).mockResolvedValue("/custom/vault");
    const onVaultChange = vi.fn();
    render(
      <VaultSettingsModal
        open
        onClose={vi.fn()}
        vault={{ enabled: false, path: "/custom/vault" }}
        onVaultChange={onVaultChange}
        onRepairAgents={vi.fn()}
      />
    );
    fireEvent.click(screen.getByRole("button", { name: "Enable vault" }));
    await waitFor(() =>
      expect(onVaultChange).toHaveBeenCalledWith({ enabled: true, path: "/custom/vault" })
    );
    expect(invoke).toHaveBeenCalledWith("scaffold_vault", { vaultPath: "/custom/vault" });
  });

  it("retains an enabled vault and shows a Codex setup failure after saving", async () => {
    vi.mocked(invoke).mockResolvedValue("/custom/vault");
    const onVaultChange = vi
      .fn()
      .mockRejectedValue(new Error("Vault settings were saved, but Codex setup failed"));
    const props = { open: true, onClose: vi.fn(), onVaultChange, onRepairAgents: vi.fn() };
    const { rerender } = render(
      <VaultSettingsModal {...props} vault={{ enabled: false, path: "/custom/vault" }} />
    );
    fireEvent.click(screen.getByRole("button", { name: "Enable vault" }));
    await screen.findByText("Vault settings were saved, but Codex setup failed");
    rerender(<VaultSettingsModal {...props} vault={{ enabled: true, path: "/custom/vault" }} />);
    expect(
      screen.getByText("Vault settings were saved, but Codex setup failed")
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Repair agent setup" })).toBeEnabled();
  });

  it("offers cleanup retry after disabling fails to remove Codex instructions", async () => {
    const onVaultChange = vi.fn().mockRejectedValue(new Error("Codex cleanup failed"));
    const onRepairAgents = vi.fn().mockResolvedValue(undefined);
    const props = { open: true, onClose: vi.fn(), onVaultChange, onRepairAgents };
    const { rerender } = render(
      <VaultSettingsModal {...props} vault={{ enabled: true, path: "/vault" }} />
    );
    fireEvent.click(screen.getByRole("button", { name: "Disable" }));
    await screen.findByText("Codex cleanup failed");
    expect(onVaultChange).toHaveBeenCalledWith({ enabled: false, path: "/vault" });
    rerender(<VaultSettingsModal {...props} vault={{ enabled: false, path: "/vault" }} />);
    fireEvent.click(screen.getByRole("button", { name: "Retry agent cleanup" }));
    await waitFor(() => expect(onRepairAgents).toHaveBeenCalledOnce());
  });

  it("creates and enables the vault from settings after skipping initial setup", async () => {
    vi.mocked(invoke).mockResolvedValue("/Users/test/Documents/worktreemanager-vault");
    const onVaultChange = vi.fn();
    render(
      <VaultSettingsModal
        open
        onClose={vi.fn()}
        vault={{ enabled: false, path: null }}
        onVaultChange={onVaultChange}
        onRepairAgents={vi.fn()}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Enable vault" }));

    await waitFor(() => {
      expect(onVaultChange).toHaveBeenCalledWith({
        enabled: true,
        path: "/Users/test/Documents/worktreemanager-vault",
      });
    });
    expect(invoke).toHaveBeenCalledWith("scaffold_vault", {
      vaultPath: "/Users/test/Documents/worktreemanager-vault",
    });
  });

  it("shows registration failures and lets the user retry without enabling the vault", async () => {
    vi.mocked(invoke).mockRejectedValueOnce("Could not register the vault");
    const onVaultChange = vi.fn();
    render(
      <VaultSettingsModal
        open
        onClose={vi.fn()}
        vault={{ enabled: false, path: null }}
        onVaultChange={onVaultChange}
        onRepairAgents={vi.fn()}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Enable vault" }));
    expect(await screen.findByText("Could not register the vault")).toBeInTheDocument();
    expect(onVaultChange).not.toHaveBeenCalled();

    vi.mocked(invoke).mockResolvedValueOnce("/Users/test/Documents/worktreemanager-vault");
    fireEvent.click(screen.getByRole("button", { name: "Enable vault" }));
    await waitFor(() => expect(onVaultChange).toHaveBeenCalledOnce());
  });
});
