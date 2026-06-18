import { useState, useEffect, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { v4 as uuid } from "uuid";
import { Modal } from "../ui/Modal";
import { Button } from "../ui/Button";
import { Badge } from "../ui/Badge";
import { SpinnerIcon, SearchIcon, ChevronLeftIcon } from "../ui/Icons";
import { useDebounce } from "../../hooks/useDebounce";
import { useLinear } from "../../contexts/LinearContext";
import {
  ALWAYS_COPIED_CONFIG_PATHS,
  EDITOR_CONFIG_PATHS,
  EditorApp,
  LinearIssue,
  Repo,
  Worktree,
} from "../../types";

interface NewWorktreeModalProps {
  open: boolean;
  onClose: () => void;
  repo: Repo;
  onCreated: (worktree: Worktree) => void;
  editorApp: EditorApp;
}

const priorityLabels: Record<number, string> = {
  0: "No priority",
  1: "Urgent",
  2: "High",
  3: "Medium",
  4: "Low",
};

const priorityVariants: Record<number, "danger" | "warning" | "accent" | "default"> = {
  1: "danger",
  2: "warning",
  3: "accent",
  4: "default",
};

export function NewWorktreeModal({ open, onClose, repo, onCreated, editorApp }: NewWorktreeModalProps) {
  const linear = useLinear();
  const [query, setQuery] = useState("");
  const [cachedIssues, setCachedIssues] = useState<LinearIssue[]>([]);
  const [remoteResults, setRemoteResults] = useState<LinearIssue[]>([]);
  const [loading, setLoading] = useState(false);
  const [searching, setSearching] = useState(false);
  const [selected, setSelected] = useState<LinearIssue | null>(null);
  const [creating, setCreating] = useState(false);
  const [creatingStatus, setCreatingStatus] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [manualMode, setManualMode] = useState(false);
  const [manualBranch, setManualBranch] = useState("");

  const debouncedQuery = useDebounce(query, 300);

  // Load assigned issues once on open (cached for instant local filtering)
  useEffect(() => {
    if (open && cachedIssues.length === 0 && linear) {
      setLoading(true);
      setError(null);
      linear
        .fetchAssignedIssues()
        .then(setCachedIssues)
        .catch((e) => setError(e instanceof Error ? e.message : "Failed to load issues"))
        .finally(() => setLoading(false));
    }
  }, [open, cachedIssues.length, linear]);

  // Local filter: instant matching on cached issues
  const localFiltered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return cachedIssues;
    return cachedIssues.filter((issue) => {
      return (
        issue.identifier.toLowerCase().includes(q) ||
        issue.title.toLowerCase().includes(q) ||
        (issue.projectName?.toLowerCase().includes(q) ?? false) ||
        (issue.description?.toLowerCase().includes(q) ?? false) ||
        issue.branchName.toLowerCase().includes(q)
      );
    });
  }, [query, cachedIssues]);

  // Remote search: debounced API call for broader results
  useEffect(() => {
    if (!debouncedQuery || debouncedQuery.trim().length === 0) {
      setRemoteResults([]);
      return;
    }
    if (!linear) return;
    setSearching(true);
    linear
      .fetchAssignedIssues(debouncedQuery)
      .then(setRemoteResults)
      .finally(() => setSearching(false));
  }, [debouncedQuery, linear]);

  // Merge: local results first, then remote results not already in local set
  const issues = useMemo(() => {
    const q = query.trim();
    if (!q) return cachedIssues;
    const localIds = new Set(localFiltered.map((i) => i.id));
    const extra = remoteResults.filter((r) => !localIds.has(r.id));
    return [...localFiltered, ...extra];
  }, [query, cachedIssues, localFiltered, remoteResults]);

  useEffect(() => {
    if (!open) {
      setQuery("");
      setSelected(null);
      setError(null);
      setCachedIssues([]);
      setRemoteResults([]);
      setManualMode(false);
      setManualBranch("");
    }
  }, [open]);

  const handleCreate = async () => {
    const branchName = selected?.branchName ?? manualBranch.trim();
    if (!branchName) return;
    setCreating(true);
    setError(null);

    try {
      if (!repo.worktreeBasePath) {
        setError("This project has no worktree directory configured. Please remove and re-add it.");
        setCreating(false);
        return;
      }
      const worktreePath = `${repo.worktreeBasePath}/${branchName}`;

      setCreatingStatus("Fetching latest from origin...");
      await invoke<string>("git_worktree_add", {
        repoPath: repo.localPath,
        worktreePath,
        branchName,
      });

      // Copy local (gitignored) config so the worktree doesn't start from
      // scratch — editor config for the editor in use + env files.
      // Best-effort: a copy failure must not abort worktree creation.
      setCreatingStatus("Copying local config...");
      try {
        await invoke<string[]>("copy_local_configs", {
          sourceRepo: repo.localPath,
          worktreePath,
          paths: [...EDITOR_CONFIG_PATHS[editorApp], ...ALWAYS_COPIED_CONFIG_PATHS],
        });
      } catch (cfgErr) {
        console.warn("Could not copy local config:", cfgErr);
      }

      if (selected && linear) {
        setCreatingStatus("Updating Linear issue...");
        await linear.startIssue(selected.id);
      }

      const worktree: Worktree = {
        id: uuid(),
        repoId: repo.id,
        branchName,
        ...(selected
          ? {
              linearIssueId: selected.id,
              linearIssueTitle: selected.title,
              linearIssueIdentifier: selected.identifier,
            }
          : {}),
        path: worktreePath,
        createdAt: new Date().toISOString(),
      };
      onCreated(worktree);

      setCreatingStatus("Opening editor...");
      try {
        await invoke<string>("open_editor", {
          editor: editorApp,
          path: worktreePath,
          branchName,
        });
      } catch (editorErr) {
        console.warn("Could not open editor:", editorErr);
      }

      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setCreating(false);
    }
  };

  const showManualForm = !linear || manualMode;

  return (
    <Modal open={open} onClose={onClose} title="New Worktree" wide={!showManualForm || !!selected}>
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

          {manualBranch.trim() && (
            <div className="flex flex-col gap-1.5">
              <p className="text-xs text-text-muted">Worktree path</p>
              <code className="block text-sm text-text-secondary bg-bg-tertiary rounded px-3 py-2 font-mono text-xs select-text cursor-text">
                {repo.worktreeBasePath}/{manualBranch.trim()}
              </code>
            </div>
          )}

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
                Create Worktree
              </Button>
            </div>
          </div>
        </div>
      ) : (
        /* Linear issue mode */
        <div className="flex flex-col h-[60vh]">
          {/* Search */}
          <div className="px-6 py-3 border-b border-border">
            <div className="relative">
              <SearchIcon className="absolute left-3 top-1/2 -translate-y-1/2 text-text-muted" />
              <input
                className="w-full pl-9 pr-3 py-2 rounded-lg border border-border bg-bg-tertiary text-sm text-text-primary placeholder:text-text-muted outline-none focus:border-accent transition-colors"
                placeholder="Search by issue ID, title, project, or description..."
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                autoFocus
              />
            </div>
          </div>

          {/* Issue list or selected issue */}
          <div className="flex-1 overflow-y-auto">
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
                      <h3 className="text-sm font-medium text-text-primary mt-1">{selected.title}</h3>
                    </div>
                    {selected.stateName && <Badge>{selected.stateName}</Badge>}
                  </div>

                  {selected.projectName && (
                    <p className="text-xs text-text-secondary">Project: {selected.projectName}</p>
                  )}

                  <div className="pt-2 border-t border-border">
                    <p className="text-xs text-text-muted mb-1">Branch name</p>
                    <code className="block text-sm text-accent bg-bg-primary rounded px-3 py-2 font-mono select-text">
                      {selected.branchName}
                    </code>
                  </div>

                  <div className="pt-1">
                    <p className="text-xs text-text-muted mb-1">Worktree path</p>
                    <code className="block text-sm text-text-secondary bg-bg-primary rounded px-3 py-2 font-mono text-xs select-text cursor-text">
                      {repo.worktreeBasePath}/{selected.branchName}
                    </code>
                  </div>
                </div>

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
                      Create Worktree
                    </Button>
                  </div>
                </div>
              </div>
            ) : (
              <>
                {loading && (
                  <div className="flex items-center justify-center py-12">
                    <SpinnerIcon size={20} className="text-text-muted" />
                  </div>
                )}

                {!loading && issues.length === 0 && !searching && (
                  <div className="flex items-center justify-center py-12">
                    <p className="text-sm text-text-muted">
                      {query ? "No issues found" : "No assigned issues in progress"}
                    </p>
                  </div>
                )}

                {!loading && issues.length === 0 && searching && (
                  <div className="flex items-center justify-center py-12">
                    <p className="text-sm text-text-muted">Searching...</p>
                  </div>
                )}

                {!loading &&
                  issues.map((issue) => (
                    <button
                      key={issue.id}
                      onClick={() => setSelected(issue)}
                      className="w-full flex items-center gap-3 px-6 py-3 hover:bg-bg-hover transition-colors text-left cursor-pointer border-b border-border/50 last:border-0"
                    >
                      <div className="flex-1 min-w-0">
                        <div className="flex items-center gap-2">
                          <span className="text-xs font-mono text-text-muted flex-shrink-0">
                            {issue.identifier}
                          </span>
                          <span className="text-sm text-text-primary truncate">{issue.title}</span>
                        </div>
                        <div className="flex items-center gap-2 mt-1">
                          {issue.projectName && (
                            <span className="text-xs text-text-muted">{issue.projectName}</span>
                          )}
                        </div>
                      </div>
                      <div className="flex items-center gap-2 flex-shrink-0">
                        {issue.priority > 0 && (
                          <Badge variant={priorityVariants[issue.priority] ?? "default"}>
                            {priorityLabels[issue.priority] ?? ""}
                          </Badge>
                        )}
                        {issue.stateName && <Badge>{issue.stateName}</Badge>}
                      </div>
                    </button>
                  ))}

                {error && (
                  <div className="px-6 py-3">
                    <div className="rounded-lg bg-danger/10 border border-danger/20 px-3 py-2">
                      <p className="text-sm text-danger select-text cursor-text">{error}</p>
                    </div>
                  </div>
                )}
              </>
            )}
          </div>

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
