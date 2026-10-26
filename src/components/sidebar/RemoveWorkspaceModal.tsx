import { Modal } from "../ui/Modal";
import { Button } from "../ui/Button";
import { DeleteOutcomeView } from "../ui/DeleteOutcomeView";
import { Task, VaultConfig, Workspace } from "../../types";
import { DeleteOptions, OperationResult } from "../../services/operations";
import { DeleteRequest } from "../../services/deletion";
import { useDeletion } from "../../hooks/useDeletion";

interface RemoveWorkspaceModalProps {
  open: boolean;
  onClose: () => void;
  workspace: Workspace;
  vault: VaultConfig;
  tasks: Task[];
  onConfirm: (options: DeleteOptions) => Promise<OperationResult<{ id: string }>>;
}

export function RemoveWorkspaceModal({
  open,
  onClose,
  workspace,
  vault,
  tasks,
  onConfirm,
}: RemoveWorkspaceModalProps) {
  const deletion = useDeletion(onConfirm);
  const removing = deletion.deleting;
  const outcome = deletion.outcome;

  const handleRemove = async (request: DeleteRequest) => {
    const result = await deletion.run(request);
    if (result.kind === "deleted" && !result.warnings.length) onClose();
  };

  const closeUnlessRemoving = () => {
    if (!removing) onClose();
  };

  return (
    <Modal open={open} onClose={closeUnlessRemoving} title={`Remove "${workspace.name}"?`}>
      <div className="p-6 space-y-4">
        {outcome?.kind === "deleted" ? null : tasks.length > 0 ? (
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
            {vault.enabled && (
              <p className="text-xs text-text-muted">
                Task notes are kept either way, moved to{" "}
                <span className="font-mono">_archive/</span>.
              </p>
            )}
          </>
        ) : (
          <p className="text-sm text-text-secondary">
            This workspace has no tasks. It will be removed from the app.
          </p>
        )}

        {outcome && (
          <DeleteOutcomeView
            outcome={outcome}
            deleting={removing}
            onRetry={(request) => void handleRemove(request)}
            onDismiss={outcome.kind === "deleted" ? onClose : deletion.dismiss}
          />
        )}

        {!outcome && (
          <div className="flex flex-col gap-2 pt-2">
            {tasks.length > 0 && (
              <Button
                variant="ghost"
                onClick={() => void handleRemove({ deleteWorktrees: true })}
                loading={removing}
                className="w-full justify-center text-danger hover:bg-danger/10"
              >
                Remove workspace and delete worktrees from disk
              </Button>
            )}
            <Button
              variant="ghost"
              onClick={() => void handleRemove({ deleteWorktrees: false })}
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
        )}
      </div>
    </Modal>
  );
}
