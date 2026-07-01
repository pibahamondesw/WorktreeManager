import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Modal } from "../ui/Modal";
import { Button } from "../ui/Button";
import { Task, Workspace } from "../../types";

interface RemoveWorkspaceModalProps {
  open: boolean;
  onClose: () => void;
  workspace: Workspace;
  tasks: Task[];
  onConfirm: (deleteFromDisk: boolean) => void;
}

export function RemoveWorkspaceModal({
  open,
  onClose,
  workspace,
  tasks,
  onConfirm,
}: RemoveWorkspaceModalProps) {
  const [removing, setRemoving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleRemove = async (deleteFromDisk: boolean) => {
    setRemoving(true);
    setError(null);

    if (deleteFromDisk && tasks.length > 0) {
      const errors: string[] = [];
      for (const task of tasks) {
        for (const m of task.members) {
          try {
            await invoke<string>("git_worktree_remove", {
              repoPath: m.localPath,
              worktreePath: m.path,
            });
          } catch (e) {
            errors.push(`${task.branchName} / ${m.repoName}: ${e}`);
          }
        }
        if (task.workspaceFilePath) {
          try {
            await invoke("delete_workspace_file", { path: task.workspaceFilePath });
          } catch {
            /* workspace-file cleanup is best-effort */
          }
        }
      }
      if (errors.length > 0) {
        setError(
          `Some worktrees could not be removed from disk:\n${errors.join("\n")}\n\nThe workspace will still be removed from the app.`,
        );
      }
    }

    onConfirm(deleteFromDisk);
    setRemoving(false);
  };

  return (
    <Modal open={open} onClose={onClose} title={`Remove "${workspace.name}"?`}>
      <div className="p-6 space-y-4">
        {tasks.length > 0 ? (
          <>
            <p className="text-sm text-text-secondary">
              This workspace has {tasks.length} task{tasks.length !== 1 ? "s" : ""}:
            </p>
            <div className="rounded-lg bg-bg-tertiary border border-border overflow-hidden max-h-56 overflow-y-auto">
              {tasks.map((task) => (
                <div key={task.id} className="px-3 py-2 border-b border-border/50 last:border-0">
                  {task.linearIssueIdentifier && (
                    <p className="text-xs font-mono text-accent">{task.linearIssueIdentifier}</p>
                  )}
                  <p className="text-sm text-text-primary truncate">
                    {task.linearIssueTitle ?? task.branchName}
                  </p>
                  <p className="text-xs text-text-muted truncate mt-0.5">
                    {task.members.map((m) => m.repoName).join(", ")}
                  </p>
                </div>
              ))}
            </div>
            <p className="text-sm text-text-secondary">
              What would you like to do with the worktrees created for these tasks?
            </p>
          </>
        ) : (
          <p className="text-sm text-text-secondary">
            This workspace has no tasks. It will be removed from the app.
          </p>
        )}

        {error && (
          <div className="rounded-lg bg-danger/10 border border-danger/20 px-3 py-2">
            <p className="text-sm text-danger select-text whitespace-pre-wrap">{error}</p>
          </div>
        )}

        <div className="flex flex-col gap-2 pt-2">
          {tasks.length > 0 && (
            <Button
              variant="ghost"
              onClick={() => handleRemove(true)}
              loading={removing}
              className="w-full justify-center text-danger hover:bg-danger/10"
            >
              Remove workspace and delete worktrees from disk
            </Button>
          )}
          <Button
            variant="ghost"
            onClick={() => handleRemove(false)}
            disabled={removing}
            className="w-full justify-center"
          >
            {tasks.length > 0
              ? "Remove workspace only (keep worktrees on disk)"
              : "Remove workspace"}
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
