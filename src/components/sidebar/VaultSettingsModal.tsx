import { useEffect, useState } from "react";
import { homeDir } from "@tauri-apps/api/path";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Modal } from "../ui/Modal";
import { Button } from "../ui/Button";
import { CopyIcon } from "../ui/Icons";
import { VaultConfig } from "../../types";
import { agentSetupLine, defaultVaultPath, enableVault, vaultUri } from "../../services/vault";

interface VaultSettingsModalProps {
  open: boolean;
  onClose: () => void;
  vault: VaultConfig;
  onVaultChange: (vault: VaultConfig) => void;
}

/**
 * Global Obsidian vault settings. Enabling scaffolds the full vault structure
 * (never overwriting existing files) — the one notes flow whose errors surface.
 */
export function VaultSettingsModal({
  open,
  onClose,
  vault,
  onVaultChange,
}: VaultSettingsModalProps) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [targetPath, setTargetPath] = useState(vault.path ?? "");

  useEffect(() => {
    if (vault.path) {
      setTargetPath(vault.path);
      return;
    }
    homeDir().then((home) => setTargetPath(defaultVaultPath(home)));
  }, [vault.path]);

  const handleEnable = async () => {
    setBusy(true);
    setError(null);
    try {
      onVaultChange(await enableVault());
    } catch (e) {
      setError(typeof e === "string" ? e : "Could not create the vault");
    } finally {
      setBusy(false);
    }
  };

  const handleDisable = () => {
    // Path is retained so re-enabling reuses the same folder. Files are untouched.
    onVaultChange({ enabled: false, path: vault.path });
  };

  const setupLine = agentSetupLine(vault);
  const handleCopySetupLine = () => {
    if (!setupLine) return;
    void navigator.clipboard.writeText(setupLine);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  return (
    <Modal open={open} onClose={onClose} title="Obsidian vault">
      <div className="p-6 space-y-4">
        {!vault.enabled ? (
          <>
            <p className="text-sm text-text-secondary">
              Create an Obsidian vault for your tasks: one note per task with frontmatter kept in
              sync by the app, plus a project layer for work that spans tickets and repos. The vault
              ships its own guide (<span className="font-mono">AGENTS.md</span>), templates, and
              scripts.
            </p>
            <div className="rounded-lg bg-bg-tertiary border border-border px-3 py-2">
              <p className="text-xs text-text-muted">Vault location</p>
              <p className="text-sm font-mono text-text-primary truncate select-text">
                {targetPath}
              </p>
            </div>
            <p className="text-xs text-text-muted">
              The folder is created and registered with Obsidian automatically (if Obsidian is
              running, it closes briefly to pick up the new vault). Files you already have are never
              overwritten.
            </p>
            {error && <p className="text-sm text-danger select-text">{error}</p>}
            <div className="flex justify-end gap-3 pt-2">
              <Button variant="ghost" onClick={onClose}>
                Cancel
              </Button>
              <Button onClick={handleEnable} disabled={busy}>
                {busy ? "Creating…" : "Enable vault"}
              </Button>
            </div>
          </>
        ) : (
          <>
            <div className="rounded-lg bg-bg-tertiary border border-border px-3 py-2">
              <p className="text-xs text-text-muted">Vault location</p>
              <p className="text-sm font-mono text-text-primary truncate select-text">
                {vault.path}
              </p>
            </div>

            {setupLine && (
              <div className="space-y-1">
                <p className="text-xs text-text-muted">
                  Wire up your agents: add this line to your AI tool's global instructions (e.g.{" "}
                  <span className="font-mono">~/.claude/CLAUDE.md</span>). Details in the vault's{" "}
                  <span className="font-mono">agent-setup.md</span>.
                </p>
                <button
                  onClick={handleCopySetupLine}
                  className="w-full flex items-center justify-between gap-2 rounded-lg bg-bg-tertiary border border-border px-3 py-2 text-left hover:bg-bg-hover transition-colors cursor-pointer"
                  title="Copy to clipboard"
                >
                  <span className="text-xs font-mono text-text-primary truncate">{setupLine}</span>
                  <span className="flex items-center gap-1 text-xs text-text-muted flex-shrink-0">
                    <CopyIcon /> {copied ? "Copied" : "Copy"}
                  </span>
                </button>
              </div>
            )}

            {error && <p className="text-sm text-danger select-text">{error}</p>}

            <div className="flex items-center justify-between gap-3 pt-2">
              <Button variant="danger" onClick={handleDisable} disabled={busy}>
                Disable
              </Button>
              <Button
                onClick={() => {
                  const uri = vaultUri(vault);
                  if (!uri) return;
                  openUrl(uri).catch((e) =>
                    setError(
                      typeof e === "string" ? e : "Could not open Obsidian — is it installed?"
                    )
                  );
                }}
              >
                Open in Obsidian
              </Button>
            </div>
            <p className="text-xs text-text-muted">
              Disabling stops note creation; nothing on disk is touched.
            </p>
          </>
        )}
      </div>
    </Modal>
  );
}
