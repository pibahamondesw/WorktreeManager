import { useState, useEffect, useMemo, useRef } from "react";
import { Modal } from "../ui/Modal";
import { Badge } from "../ui/Badge";
import { ChevronLeftIcon } from "../ui/Icons";
import { LinearIssuePicker } from "./LinearIssuePicker";
import { NewLinearIssueForm, NewLinearIssueDraft } from "./NewLinearIssueForm";
import { RepositoryChecklist } from "./RepositoryChecklist";
import { NewTaskFooter } from "./NewTaskFooter";
import { useLinear } from "../../contexts/useLinear";
import { EditorApp, LinearIssue, Task, TaskMember, Workspace } from "../../types";
import {
  CreateTaskInput,
  OperationResult,
  OperationError,
  TaskReady,
} from "../../services/operations";
import { OpenTaskOptions } from "../../hooks/useOpenTask";

interface NewWorktreeModalProps {
  open: boolean;
  onClose: () => void;
  workspace: Workspace;
  onCreated: (
    input: CreateTaskInput,
    progress?: (message: string) => void,
    onReady?: TaskReady
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
  onOpenHint,
}: NewWorktreeModalProps) {
  const linear = useLinear();
  const [selected, setSelected] = useState<LinearIssue | null>(null);
  const [creating, setCreating] = useState(false);
  const creatingRef = useRef(false);
  const [creatingStatus, setCreatingStatus] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [mode, setMode] = useState<"existingIssue" | "manualBranch" | "newIssue">("existingIssue");
  const [manualBranch, setManualBranch] = useState("");
  const [issueDraft, setIssueDraft] = useState<NewLinearIssueDraft>({
    title: "",
    description: "",
    teamId: "",
  });
  const [createdIssue, setCreatedIssue] = useState<LinearIssue | null>(null);
  const [repoSel, setRepoSel] = useState<Record<string, RepoSelection>>({});

  useEffect(() => {
    if (!open) {
      setSelected(null);
      setError(null);
      setMode("existingIssue");
      setManualBranch("");
      setIssueDraft({ title: "", description: "", teamId: "" });
      setCreatedIssue(null);
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
    if (creatingRef.current) return;
    if (!canCreate) return;
    if (includedRepos.length === 0) {
      setError("Select at least one repository");
      return;
    }
    creatingRef.current = true;
    setCreating(true);
    setCreatingStatus("Preparing worktrees...");
    setError(null);

    let ready = false;
    try {
      let linearIssue =
        mode === "existingIssue" && linear
          ? selected
          : mode === "newIssue" && linear
            ? createdIssue
            : null;
      if (!linearIssue && mode === "newIssue" && linear) {
        setCreatingStatus("Creating Linear issue...");
        linearIssue = await linear.createIssue({
          teamId: issueDraft.teamId,
          title: issueDraft.title.trim(),
          description: issueDraft.description.trim(),
        });
        setCreatedIssue(linearIssue);
      }
      const branchInput = (linearIssue?.branchName ?? manualBranch).trim();
      setCreatingStatus("Preparing worktrees...");
      await onCreated(
        {
          workspaceId: workspace.id,
          branchName: branchInput,
          repoIds: includedRepos.map((repo) => repo.id),
          ...(linearIssue
            ? {
                linearIssue: {
                  id: linearIssue.id,
                  identifier: linearIssue.identifier,
                  title: linearIssue.title,
                  projectId: linearIssue.projectId,
                  projectName: linearIssue.projectName,
                },
              }
            : {}),
        },
        (message) => {
          if (!ready) setCreatingStatus(message);
        },
        async (task) => {
          ready = true;
          if (linearIssue && linear) {
            void linear
              .startIssue(linearIssue.id)
              .catch(() => onOpenHint?.("Could not update Linear issue status"));
          }
          try {
            return (
              (await onOpenTask?.(task, { onMessage: onOpenHint, onError: onOpenHint })) ?? false
            );
          } finally {
            creatingRef.current = false;
            setCreating(false);
            onClose();
          }
        }
      );
    } catch (e) {
      if (!ready) {
        let message = e instanceof Error ? e.message : String(e);
        if (e instanceof OperationError && e.code === "create_failed") {
          const details = e.details as {
            completedMembers: TaskMember[];
            failedMembers: TaskMember[];
          };
          message +=
            "\n" +
            [
              ...details.completedMembers.map(
                (member) => `Created: ${member.repoName} · ${member.path}`
              ),
              ...details.failedMembers.map(
                (member) => `Failed: ${member.repoName} · ${member.path}`
              ),
            ].join("\n");
        }
        setError(message);
      }
    } finally {
      if (!ready) {
        creatingRef.current = false;
        setCreating(false);
      }
    }
  };

  const showManualForm = !linear || mode !== "existingIssue";
  const canCreate =
    !linear || mode === "manualBranch"
      ? !!manualBranch.trim()
      : mode === "newIssue"
        ? !!createdIssue || (!!issueDraft.title.trim() && !!issueDraft.teamId)
        : !!selected?.branchName.trim();

  const repoChecklist = (
    <RepositoryChecklist
      repos={workspace.repos}
      selection={repoSel}
      disabled={creating}
      onChange={setIncluded}
    />
  );
  const footer = (
    <NewTaskFooter
      error={error}
      creating={creating}
      status={creatingStatus}
      canCreate={canCreate}
      onClose={onClose}
      onCreate={() => void handleCreate()}
    />
  );

  return (
    <Modal
      open={open}
      onClose={() => {
        if (!creatingRef.current) onClose();
      }}
      title="New Task"
      wide={!showManualForm}
    >
      {showManualForm ? (
        /* Manual branch mode */
        <div className="p-6 space-y-4">
          {linear && (
            <button
              disabled={creating}
              onClick={() => setMode("existingIssue")}
              className="flex items-center gap-1 text-sm text-text-muted hover:text-text-primary transition-colors cursor-pointer"
            >
              <ChevronLeftIcon />
              Back to Linear issues
            </button>
          )}

          {linear && (
            <label className="flex items-center gap-2 text-sm text-text-secondary cursor-pointer">
              <input
                type="checkbox"
                disabled={creating}
                checked={mode === "newIssue"}
                onChange={(e) => setMode(e.target.checked ? "newIssue" : "manualBranch")}
                className="accent-accent"
              />
              Also create Linear issue
            </label>
          )}

          {linear && mode === "newIssue" ? (
            <NewLinearIssueForm
              linear={linear}
              value={issueDraft}
              onChange={setIssueDraft}
              disabled={creating}
              createdIssue={createdIssue}
              onSubmit={() => void handleCreate()}
            />
          ) : (
            <div className="flex flex-col gap-1.5">
              <label className="text-sm font-medium text-text-secondary">Branch name</label>
              <input
                className="w-full rounded-lg border border-border bg-bg-tertiary px-3 py-2 text-sm text-text-primary placeholder:text-text-muted outline-none focus:border-accent transition-colors font-mono"
                placeholder="feature/my-branch"
                disabled={creating}
                value={manualBranch}
                onChange={(e) => setManualBranch(e.target.value)}
                autoFocus
                onKeyDown={(e) => {
                  if (e.key === "Enter" && !creating && manualBranch.trim()) void handleCreate();
                }}
              />
            </div>
          )}

          {repoChecklist}

          {footer}
        </div>
      ) : (
        /* Linear issue mode */
        <div className="flex flex-col h-[60vh]">
          {open && (
            <LinearIssuePicker onSelect={setSelected}>
              {selected ? (
                <div className="p-6 space-y-4">
                  <button
                    disabled={creating}
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

                  {footer}
                </div>
              ) : undefined}
            </LinearIssuePicker>
          )}

          {/* Create without issue link */}
          {!selected && (
            <div className="px-6 py-3 border-t border-border flex-shrink-0">
              <button
                disabled={creating}
                onClick={() => setMode("manualBranch")}
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
