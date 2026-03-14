import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Modal } from "../ui/Modal";
import { Button } from "../ui/Button";
import { Repo, Worktree } from "../../types";

interface RemoveRepoModalProps {
  open: boolean;
  onClose: () => void;
  repo: Repo;
  worktrees: Worktree[];
  onConfirm: (deleteFromDisk: boolean) => void;
}

export function RemoveRepoModal({
  open,
  onClose,
  repo,
  worktrees,
  onConfirm,
}: RemoveRepoModalProps) {
  const [removing, setRemoving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleRemove = async (deleteFromDisk: boolean) => {
    setRemoving(true);
    setError(null);

    if (deleteFromDisk && worktrees.length > 0) {
      const errors: string[] = [];
      for (const wt of worktrees) {
        try {
          await invoke<string>("git_worktree_remove", {
            repoPath: repo.localPath,
            worktreePath: wt.path,
          });
        } catch (e) {
          errors.push(`${wt.branchName}: ${e}`);
        }
      }
      if (errors.length > 0) {
        setError(
          `Some worktrees could not be removed from disk:\n${errors.join("\n")}\n\nThe project will still be removed from the app.`
        );
      }
    }

    onConfirm(deleteFromDisk);
    setRemoving(false);
  };

  return (
    <Modal open={open} onClose={onClose} title={`Remove "${repo.name}"?`}>
      <div className="p-6 space-y-4">
        {worktrees.length > 0 ? (
          <>
            <p className="text-sm text-text-secondary">
              This project has {worktrees.length} active worktree
              {worktrees.length !== 1 ? "s" : ""}:
            </p>
            <div className="rounded-lg bg-bg-tertiary border border-border overflow-hidden">
              {worktrees.map((wt) => (
                <div key={wt.id} className="px-3 py-2 border-b border-border/50 last:border-0">
                  <p className="text-xs font-mono text-accent">{wt.linearIssueIdentifier}</p>
                  <p className="text-sm text-text-primary truncate">{wt.linearIssueTitle}</p>
                  <p className="text-xs text-text-muted font-mono truncate mt-0.5">{wt.path}</p>
                </div>
              ))}
            </div>
            <p className="text-sm text-text-secondary">
              What would you like to do with these worktrees?
            </p>
          </>
        ) : (
          <p className="text-sm text-text-secondary">
            This project has no active worktrees. It will be removed from the app.
          </p>
        )}

        {error && (
          <div className="rounded-lg bg-danger/10 border border-danger/20 px-3 py-2">
            <p className="text-sm text-danger select-text whitespace-pre-wrap">{error}</p>
          </div>
        )}

        <div className="flex flex-col gap-2 pt-2">
          {worktrees.length > 0 && (
            <Button
              variant="ghost"
              onClick={() => handleRemove(true)}
              loading={removing}
              className="w-full justify-center text-danger hover:bg-danger/10"
            >
              Remove project and delete worktrees from disk
            </Button>
          )}
          <Button
            variant="ghost"
            onClick={() => handleRemove(false)}
            disabled={removing}
            className="w-full justify-center"
          >
            {worktrees.length > 0
              ? "Remove project only (keep worktrees on disk)"
              : "Remove project"}
          </Button>
          <Button
            variant="ghost"
            onClick={onClose}
            disabled={removing}
            className="w-full justify-center text-text-muted"
          >
            Cancel
          </Button>
        </div>
      </div>
    </Modal>
  );
}
