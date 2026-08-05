import { useEffect, useMemo, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Badge } from "../ui/Badge";
import { SearchIcon, BranchIcon } from "../ui/Icons";
import { useEphemeralToast } from "../../hooks/useEphemeralToast";
import { WorktreeListToast } from "../worktree/WorktreeListToast";
import { openEditorForWorktree } from "../../services/openEditor";
import { parseQuery, scopeValues, withScope, withoutScope } from "../../search/query";
import { searchTasks, TaskSearchResult } from "../../search/searchTasks";
import { EditorApp, Task, Workspace } from "../../types";

interface QuickSearchModalProps {
  open: boolean;
  onClose: () => void;
  initialQuery: string;
  tasks: Task[];
  workspaces: Workspace[];
  selectedWorkspaceId: string | null;
  editorApp: EditorApp;
  /** Switch to the task's workspace and select it in the list. */
  onReveal: (task: Task) => void;
}

export function QuickSearchModal({
  open,
  onClose,
  initialQuery,
  tasks,
  workspaces,
  selectedWorkspaceId,
  editorApp,
  onReveal,
}: QuickSearchModalProps) {
  const [query, setQuery] = useState(initialQuery);
  const [activeIndex, setActiveIndex] = useState(0);
  const { toast, showToast } = useEphemeralToast();
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const currentWorkspace = useMemo(
    () => workspaces.find((w) => w.id === selectedWorkspaceId),
    [workspaces, selectedWorkspaceId]
  );

  useEffect(() => {
    if (!open) return;
    setQuery(initialQuery);
    setActiveIndex(0);
    // Caret at the end so a pre-filled `in:<workspace>` reads as a starting point.
    requestAnimationFrame(() => {
      const input = inputRef.current;
      if (!input) return;
      input.focus();
      input.setSelectionRange(input.value.length, input.value.length);
    });
  }, [open, initialQuery]);

  const results = useMemo(
    () =>
      open
        ? searchTasks({ tasks, workspaces, selectedWorkspaceId, query })
        : ([] as TaskSearchResult[]),
    [open, tasks, workspaces, selectedWorkspaceId, query]
  );

  useEffect(() => {
    setActiveIndex((i) => Math.min(i, Math.max(results.length - 1, 0)));
  }, [results.length]);

  useEffect(() => {
    listRef.current?.querySelector('[data-active="true"]')?.scrollIntoView({ block: "nearest" });
  }, [activeIndex, results]);

  if (!open) return null;

  const scoped = scopeValues(parseQuery(query)).length > 0;
  const active = results[activeIndex];

  const setScoped = (next: boolean) => {
    if (!next) setQuery(withoutScope);
    else if (currentWorkspace) setQuery((q) => withScope(q, currentWorkspace.name));
    inputRef.current?.focus();
  };

  // Stay open on failure so the error toast is readable.
  const openInEditor = async (result: TaskSearchResult) => {
    const opened = await openEditorForWorktree(
      editorApp,
      result.task.members.map((m) => m.path),
      result.task.branchName,
      result.workspace?.name,
      { onMessage: showToast, onError: showToast }
    );
    if (opened) onClose();
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Escape") {
      e.preventDefault();
      // Don't let it reach Modal's document listener and close what's underneath too.
      e.stopPropagation();
      onClose();
      return;
    }
    if (e.key === "ArrowDown" || (e.key === "n" && e.ctrlKey)) {
      e.preventDefault();
      setActiveIndex((i) => Math.min(i + 1, results.length - 1));
      return;
    }
    if (e.key === "ArrowUp" || (e.key === "p" && e.ctrlKey)) {
      e.preventDefault();
      setActiveIndex((i) => Math.max(i - 1, 0));
      return;
    }
    if (e.key === "Enter" && active) {
      e.preventDefault();
      if (e.metaKey) {
        onReveal(active.task);
        onClose();
      } else {
        void openInEditor(active);
      }
      return;
    }
    if (e.key === "l" && e.metaKey && active?.task.linearIssueIdentifier) {
      e.preventDefault();
      openUrl(`https://linear.app/issue/${active.task.linearIssueIdentifier}`);
      onClose();
    }
  };

  let lastGroup: boolean | null = null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/60 backdrop-blur-sm pt-[12vh]"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="w-[640px] max-w-[92vw] bg-bg-secondary border border-border rounded-xl shadow-2xl flex flex-col max-h-[68vh] overflow-hidden">
        <div className="px-4 py-3 border-b border-border">
          <div className="relative">
            <SearchIcon className="absolute left-3 top-1/2 -translate-y-1/2 text-text-muted" />
            <input
              ref={inputRef}
              className="w-full pl-9 pr-3 py-2 rounded-lg border border-border bg-bg-tertiary text-sm text-text-primary placeholder:text-text-muted outline-none focus:border-accent transition-colors"
              placeholder="Search tasks by Linear ID, title, or branch…"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={handleKeyDown}
            />
          </div>
          <div className="flex items-center gap-1 mt-2">
            <ScopeTab active={!scoped} onClick={() => setScoped(false)}>
              All workspaces
            </ScopeTab>
            <ScopeTab active={scoped} onClick={() => setScoped(true)} disabled={!currentWorkspace}>
              This workspace
            </ScopeTab>
            <span className="ml-auto text-[10px] text-text-muted font-mono">
              in:web|api · repo: · branch: · -exclude
            </span>
          </div>
        </div>

        <div ref={listRef} className="flex-1 overflow-y-auto">
          {results.length === 0 ? (
            <p className="text-sm text-text-muted text-center py-10">
              {tasks.length === 0 ? "No tasks yet" : "No tasks match"}
            </p>
          ) : (
            results.map((result, i) => {
              const header =
                result.inCurrentWorkspace !== lastGroup ? (
                  <p
                    key={`h-${i}`}
                    className="px-4 pt-3 pb-1 text-[10px] uppercase tracking-wide text-text-muted"
                  >
                    {result.inCurrentWorkspace ? "This workspace" : "Other workspaces"}
                  </p>
                ) : null;
              lastGroup = result.inCurrentWorkspace;
              return (
                <div key={result.task.id}>
                  {header}
                  <button
                    data-active={i === activeIndex}
                    onMouseMove={() => setActiveIndex(i)}
                    onClick={() => void openInEditor(result)}
                    className={`w-full text-left px-4 py-2.5 flex items-center gap-3 cursor-pointer transition-colors ${
                      i === activeIndex ? "bg-bg-hover" : "hover:bg-bg-hover/50"
                    }`}
                  >
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2">
                        {result.task.linearIssueIdentifier && (
                          <span className="text-xs font-mono text-text-muted flex-shrink-0">
                            {result.task.linearIssueIdentifier}
                          </span>
                        )}
                        <span className="text-sm text-text-primary truncate">
                          {result.task.linearIssueTitle || result.task.branchName}
                        </span>
                      </div>
                      <div className="flex items-center gap-1.5 mt-0.5 text-xs text-text-muted min-w-0">
                        <BranchIcon />
                        <span className="font-mono truncate">{result.task.branchName}</span>
                      </div>
                    </div>
                    {!result.inCurrentWorkspace && result.workspace && (
                      <Badge>{result.workspace.name}</Badge>
                    )}
                  </button>
                </div>
              );
            })
          )}
        </div>

        <div className="flex-shrink-0 px-4 py-2 border-t border-border flex items-center gap-3 text-[10px] text-text-muted font-mono flex-wrap">
          <Hint keys="↑↓">navigate</Hint>
          <Hint keys="↵">open</Hint>
          <Hint keys="⌘↵">reveal</Hint>
          <Hint keys="⌘L">linear</Hint>
          <Hint keys="esc">close</Hint>
          <span className="ml-auto">
            {results.length} result{results.length !== 1 ? "s" : ""}
          </span>
        </div>
      </div>

      {toast && <WorktreeListToast message={toast} />}
    </div>
  );
}

function ScopeTab({
  active,
  onClick,
  disabled,
  children,
}: {
  active: boolean;
  onClick: () => void;
  disabled?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className={`px-2 py-1 rounded-md text-xs transition-colors cursor-pointer disabled:opacity-40 disabled:cursor-default ${
        active ? "bg-bg-tertiary text-text-primary" : "text-text-muted hover:text-text-primary"
      }`}
    >
      {children}
    </button>
  );
}

function Hint({ keys, children }: { keys: string; children: React.ReactNode }) {
  return (
    <span>
      <kbd className="px-1 py-0.5 bg-bg-tertiary rounded">{keys}</kbd> {children}
    </span>
  );
}
