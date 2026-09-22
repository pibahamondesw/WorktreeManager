import { useState, useEffect, useMemo } from "react";
import { Modal } from "../ui/Modal";
import { Button } from "../ui/Button";
import { Badge } from "../ui/Badge";
import { ChevronLeftIcon } from "../ui/Icons";
import { LinearIssuePicker } from "./LinearIssuePicker";
import { useLinear } from "../../contexts/useLinear";
import { EditorApp, LinearIssue, Task, Workspace } from "../../types";
import { openEditorForWorktree } from "../../services/openEditor";
import { CreateTaskInput, OperationResult } from "../../services/operations";
import { isEmbedded, taskSurfaceFor } from "../../embedded/taskSurface";
import { OpenTaskOptions } from "../../hooks/useOpenTask";

interface NewWorktreeModalProps {
  open: boolean;
  onClose: () => void;
  workspace: Workspace;
  onCreated: (
    input: CreateTaskInput,
    progress?: (message: string) => void
  ) => Promise<OperationResult<Task>>;
  onOpenTask?: (task: Task, options?: OpenTaskOptions) => Promise<boolean>;
  editorApp: EditorApp;
  onOpenHint?: (msg: string) => void;
}

interface RepoSelection {
  included: boolean;
}

export function NewWorktreeModal({
  open,
  onClose,
  workspace,
  onCreated,
  onOpenTask,
  editorApp,
  onOpenHint,
}: NewWorktreeModalProps) {
  const linear = useLinear();
  const [selected, setSelected] = useState<LinearIssue | null>(null);
  const [creating, setCreating] = useState(false);
  const [creatingStatus, setCreatingStatus] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [manualMode, setManualMode] = useState(false);
  const [manualBranch, setManualBranch] = useState("");
  const [repoSel, setRepoSel] = useState<Record<string, RepoSelection>>({});

  useEffect(() => {
    if (!open) {
      setSelected(null);
      setError(null);
      setManualMode(false);
      setManualBranch("");
      return;
    }
    // Initialize the repo checklist: all repos included, each at its default mode.
    const init: Record<string, RepoSelection> = {};
    for (const r of workspace.repos) init[r.id] = { included: true };
    setRepoSel(init);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, workspace.id]);

  const includedRepos = useMemo(
    () => workspace.repos.filter((r) => repoSel[r.id]?.included),
    [workspace.repos, repoSel]
  );

  const setIncluded = (repoId: string, included: boolean) =>
    setRepoSel((prev) => ({ ...prev, [repoId]: { ...prev[repoId], included } }));

  const handleCreate = async () => {
    const branchInput = (selected?.branchName ?? manualBranch).trim();
    if (!branchInput) return;
    if (includedRepos.length === 0) {
      setError("Select at least one repository");
      return;
    }
    setCreating(true);
    setCreatingStatus("Preparing worktrees...");
    setError(null);

    try {
      const { data: task, warnings } = await onCreated(
        {
          workspaceId: workspace.id,
          branchName: branchInput,
          repoIds: includedRepos.map((repo) => repo.id),
          ...(selected
            ? {
                linearIssue: {
                  id: selected.id,
                  identifier: selected.identifier,
                  title: selected.title,
                },
              }
            : {}),
        },
        setCreatingStatus
      );
      for (const warning of warnings) onOpenHint?.(warning.message);
      if (selected && linear) {
        void linear
          .startIssue(selected.id)
          .catch(() => onOpenHint?.("Could not update Linear issue status"));
      }
      if (isEmbedded(taskSurfaceFor(editorApp))) {
        void onOpenTask?.(task);
      } else {
        await openEditorForWorktree(
          editorApp,
          task.members.map((member) => member.path),
          task.branchName,
          workspace.name,
          {
            onMessage: onOpenHint,
            onError: onOpenHint,
          }
        );
      }
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setCreating(false);
    }
  };

  const showManualForm = !linear || manualMode;

  const repoChecklist =
    workspace.repos.length > 1 ? (
      <div className="flex flex-col gap-2">
        <p className="text-xs text-text-muted">Repositories in this task</p>
        <div className="rounded-lg border border-border bg-bg-tertiary divide-y divide-border/50">
          {workspace.repos.map((r) => {
            const sel = repoSel[r.id];
            const included = sel?.included ?? false;
            return (
              <label key={r.id} className="flex items-center gap-2 px-3 py-2 cursor-pointer">
                <input
                  type="checkbox"
                  checked={included}
                  onChange={(e) => setIncluded(r.id, e.target.checked)}
                  className="accent-accent"
                />
                <span
                  className={`text-sm truncate ${included ? "text-text-primary" : "text-text-muted"}`}
                >
                  {r.name}
                </span>
              </label>
            );
          })}
        </div>
      </div>
    ) : null;

  return (
    <Modal open={open} onClose={onClose} title="New Task" wide={!showManualForm || !!selected}>
      {showManualForm && !selected ? (
        /* Manual branch mode */
        <div className="p-6 space-y-4">
          {linear && (
            <button
              onClick={() => setManualMode(false)}
              className="flex items-center gap-1 text-sm text-text-muted hover:text-text-primary transition-colors cursor-pointer"
            >
              <ChevronLeftIcon />
              Back to Linear issues
            </button>
          )}

          <div className="flex flex-col gap-1.5">
            <label className="text-sm font-medium text-text-secondary">Branch name</label>
            <input
              className="w-full rounded-lg border border-border bg-bg-tertiary px-3 py-2 text-sm text-text-primary placeholder:text-text-muted outline-none focus:border-accent transition-colors font-mono"
              placeholder="feature/my-branch"
              value={manualBranch}
              onChange={(e) => setManualBranch(e.target.value)}
              autoFocus
              onKeyDown={(e) => {
                if (e.key === "Enter" && manualBranch.trim()) handleCreate();
              }}
            />
          </div>

          {repoChecklist}

          {error && (
            <div className="rounded-lg bg-danger/10 border border-danger/20 px-3 py-2">
              <p className="text-sm text-danger select-text cursor-text">{error}</p>
            </div>
          )}

          <div className="flex items-center justify-between">
            {creating && creatingStatus ? (
              <span className="text-xs text-text-muted">{creatingStatus}</span>
            ) : (
              <span />
            )}
            <div className="flex gap-3">
              <Button variant="ghost" onClick={onClose} disabled={creating}>
                Cancel
              </Button>
              <Button onClick={handleCreate} loading={creating} disabled={!manualBranch.trim()}>
                Create Task
              </Button>
            </div>
          </div>
        </div>
      ) : (
        /* Linear issue mode */
        <div className="flex flex-col h-[60vh]">
          {open && (
            <LinearIssuePicker onSelect={setSelected}>
              {selected ? (
                <div className="p-6 space-y-4">
                  <button
                    onClick={() => setSelected(null)}
                    className="flex items-center gap-1 text-sm text-text-muted hover:text-text-primary transition-colors cursor-pointer"
                  >
                    <ChevronLeftIcon />
                    Back to results
                  </button>

                  <div className="bg-bg-tertiary rounded-lg p-4 space-y-3">
                    <div className="flex items-start justify-between">
                      <div>
                        <p className="text-xs text-text-muted font-mono">{selected.identifier}</p>
                        <h3 className="text-sm font-medium text-text-primary mt-1">
                          {selected.title}
                        </h3>
                      </div>
                      {selected.stateName && <Badge>{selected.stateName}</Badge>}
                    </div>

                    {selected.projectName && (
                      <p className="text-xs text-text-secondary">Project: {selected.projectName}</p>
                    )}

                    <div className="pt-2 border-t border-border">
                      <p className="text-xs text-text-muted mb-1">Branch name</p>
                      <code
                        className="block truncate text-sm text-accent bg-bg-primary rounded px-3 py-2 font-mono select-text"
                        title={selected.branchName}
                      >
                        {selected.branchName}
                      </code>
                    </div>
                  </div>

                  {repoChecklist}

                  {error && (
                    <div className="rounded-lg bg-danger/10 border border-danger/20 px-3 py-2">
                      <p className="text-sm text-danger select-text cursor-text">{error}</p>
                    </div>
                  )}

                  <div className="flex items-center justify-between">
                    {creating && creatingStatus ? (
                      <span className="text-xs text-text-muted">{creatingStatus}</span>
                    ) : (
                      <span />
                    )}
                    <div className="flex gap-3">
                      <Button variant="ghost" onClick={onClose} disabled={creating}>
                        Cancel
                      </Button>
                      <Button onClick={handleCreate} loading={creating}>
                        Create Task
                      </Button>
                    </div>
                  </div>
                </div>
              ) : undefined}
            </LinearIssuePicker>
          )}

          {/* Create without issue link */}
          {!selected && (
            <div className="px-6 py-3 border-t border-border flex-shrink-0">
              <button
                onClick={() => setManualMode(true)}
                className="text-xs text-text-muted hover:text-text-primary transition-colors cursor-pointer"
              >
                Create without Linear issue
              </button>
            </div>
          )}
        </div>
      )}
    </Modal>
  );
}
