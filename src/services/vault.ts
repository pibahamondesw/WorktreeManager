import { invoke } from "@tauri-apps/api/core";
import { homeDir } from "@tauri-apps/api/path";
import { VaultConfig } from "../types";

export type VaultAgent = "codex" | "claude";
export const VAULT_AGENT_LABELS: Record<VaultAgent, string> = {
  codex: "Codex",
  claude: "Claude Code",
};

/**
 * Global Obsidian vault, managed by the app: scaffolded on opt-in at an
 * app-chosen path, task notes written to `<vault>/task-logs/`. The app never
 * replaces user notes; setup may append a task-log skill reference to the guide.
 */

export function defaultVaultPath(home: string): string {
  return `${home.replace(/\/+$/, "")}/Documents/worktreemanager-vault`;
}

/** Folder task notes live in, or null when the vault is disabled/unset. */
export function taskLogsPath(vault: VaultConfig): string | null {
  const path = vault.path?.trim();
  if (!vault.enabled || !path) return null;
  return `${path.replace(/\/+$/, "")}/task-logs`;
}

/** Deep link that opens the vault folder in Obsidian. */
export function vaultUri(vault: VaultConfig): string | null {
  const path = vault.path?.trim();
  if (!path) return null;
  return `obsidian://open?path=${encodeURIComponent(path)}`;
}

/**
 * Enable the vault at the managed path: scaffold the full structure (idempotent,
 * preserves existing content) and register it with Obsidian. Throws if scaffolding or registration fails —
 * this is an explicit user action and the one notes flow that must surface errors.
 * Re-enabling preserves an existing custom path. Global agent instructions are
 * installed through Operations after the vault setting has been persisted.
 */
export async function enableVault(existingPath?: string | null): Promise<VaultConfig> {
  const path = existingPath?.trim() || defaultVaultPath(await homeDir());
  await invoke<string>("scaffold_vault", { vaultPath: path });
  return { enabled: true, path };
}
