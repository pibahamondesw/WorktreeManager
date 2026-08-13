import { invoke } from "@tauri-apps/api/core";
import { homeDir } from "@tauri-apps/api/path";
import { VaultConfig } from "../types";

/**
 * Global Obsidian vault, managed by the app: scaffolded on opt-in at an
 * app-chosen path, task notes written to `<vault>/task-logs/`. The app never
 * overwrites a file that already exists in the vault.
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

/** Line to add to an agent's global instructions (e.g. ~/.claude/CLAUDE.md). */
export function agentSetupLine(vault: VaultConfig): string | null {
  const path = vault.path?.trim();
  if (!path) return null;
  return `@${path.replace(/\/+$/, "")}/agent-setup.md`;
}

/**
 * Enable the vault at the managed path: scaffold the full structure (idempotent,
 * never overwrites) and register it with Obsidian. Throws if scaffolding fails —
 * this is an explicit user action and the one notes flow that must surface errors.
 * A hand-edited custom `path` in the store is honored everywhere else, but Enable
 * always targets the managed default so the flow stays predictable.
 */
export async function enableVault(): Promise<VaultConfig> {
  const path = defaultVaultPath(await homeDir());
  await invoke<string>("scaffold_vault", { vaultPath: path });
  return { enabled: true, path };
}
