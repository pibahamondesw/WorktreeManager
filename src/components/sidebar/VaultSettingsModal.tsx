import { useEffect, useState } from "react";
import { homeDir } from "@tauri-apps/api/path";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Modal } from "../ui/Modal";
import { Button } from "../ui/Button";
import { VaultConfig } from "../../types";
import { defaultVaultPath, enableVault, vaultUri } from "../../services/vault";

interface VaultSettingsModalProps {
  open: boolean;
  onClose: () => void;
  vault: VaultConfig;
  onVaultChange: (vault: VaultConfig) => void | Promise<void>;
  onRepairAgents: () => Promise<void>;
}

/**
 * Global Obsidian vault settings. Enabling scaffolds the full vault structure
 * (never overwriting existing files) — the one notes flow whose errors surface.
 */
export function VaultSettingsModal({ open, onClose, ...props }: VaultSettingsModalProps) {
  return (
    <Modal open={open} onClose={onClose} title="Obsidian vault">
      <VaultSettings {...props} onClose={onClose} />
    </Modal>
  );
}

export function VaultSettings({
  onClose,
  vault,
  onVaultChange,
  onRepairAgents,
}: Omit<VaultSettingsModalProps, "open">) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [targetPath, setTargetPath] = useState(vault.path ?? "");

  useEffect(() => {
    if (vault.path) {
      setTargetPath(vault.path);
      return;
    }
    homeDir()
      .then((home) => setTargetPath(defaultVaultPath(home)))
      .catch(() => setError("Could not find the home directory"));
  }, [vault.path]);

  const handleEnable = async () => {
    setBusy(true);
    setError(null);
    try {
      await onVaultChange(await enableVault(vault.path));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const handleDisable = async () => {
    setBusy(true);
    setError(null);
    try {
      await onVaultChange({ enabled: false, path: vault.path });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const handleRepairAgents = async () => {
    setBusy(true);
    setError(null);
    try {
      await onRepairAgents();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="p-6 space-y-4">
      {!vault.enabled ? (
        <>
          <p className="text-sm text-text-secondary">
            Create an Obsidian vault for your tasks: one note per task with frontmatter kept in sync
            by the app, plus a project layer for work that spans tickets and repos. The vault ships
            its own guide (<span className="font-mono">AGENTS.md</span>), templates, and scripts.
          </p>
          <div className="rounded-lg bg-bg-tertiary border border-border px-3 py-2">
            <p className="text-xs text-text-muted">Vault location</p>
            <p className="text-sm font-mono text-text-primary truncate select-text">{targetPath}</p>
          </div>
          <p className="text-xs text-text-muted">
            The folder is created and registered with Obsidian automatically (if Obsidian is
            running, it closes briefly to pick up the new vault). Files you already have are never
            overwritten.
          </p>
          <p className="text-xs text-text-muted">
            Codex and Claude Code vault instructions are installed automatically, preserving your
            personal instructions. Start new agent sessions after setup.
          </p>
          {error && <p className="text-sm text-danger select-text">{error}</p>}
          {error && vault.path && (
            <Button variant="secondary" onClick={() => void handleRepairAgents()} disabled={busy}>
              Retry agent cleanup
            </Button>
          )}
          <div className="flex justify-end gap-3 pt-2">
            <Button variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button onClick={() => void handleEnable()} disabled={busy}>
              {busy ? "Creating…" : "Enable vault"}
            </Button>
          </div>
        </>
      ) : (
        <>
          <div className="rounded-lg bg-bg-tertiary border border-border px-3 py-2">
            <p className="text-xs text-text-muted">Vault location</p>
            <p className="text-sm font-mono text-text-primary truncate select-text">{vault.path}</p>
          </div>

          <div className="space-y-2">
            <p className="text-xs text-text-muted">
              Codex and Claude Code: vault instructions are installed automatically. Dependencies
              checks each agent separately. Repair sets up both agents for an existing vault; start
              new sessions afterward.
            </p>
            <Button variant="secondary" onClick={() => void handleRepairAgents()} disabled={busy}>
              {busy ? "Updating…" : "Repair agent setup"}
            </Button>
          </div>

          {error && <p className="text-sm text-danger select-text">{error}</p>}

          <div className="flex items-center justify-between gap-3 pt-2">
            <Button variant="danger" onClick={() => void handleDisable()} disabled={busy}>
              Disable
            </Button>
            <Button
              onClick={() => {
                const uri = vaultUri(vault);
                if (!uri) return;
                openUrl(uri).catch((e) =>
                  setError(typeof e === "string" ? e : "Could not open Obsidian — is it installed?")
                );
              }}
            >
              Open in Obsidian
            </Button>
          </div>
          <p className="text-xs text-text-muted">
            Disabling stops note creation and removes only the agent instructions managed by
            WorktreeManager. Vault files and personal instructions are preserved.
          </p>
        </>
      )}
    </div>
  );
}
