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
  it("creates and enables the vault from settings after skipping initial setup", async () => {
    vi.mocked(invoke).mockResolvedValue("/Users/test/Documents/worktreemanager-vault");
    const onVaultChange = vi.fn();
    render(
      <VaultSettingsModal
        open
        onClose={vi.fn()}
        vault={{ enabled: false, path: null }}
        onVaultChange={onVaultChange}
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
