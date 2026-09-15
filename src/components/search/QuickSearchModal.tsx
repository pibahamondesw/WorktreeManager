import { useEffect, useMemo, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { SearchIcon } from "../ui/Icons";
import { useEphemeralToast } from "../../hooks/useEphemeralToast";
import { WorktreeListToast } from "../worktree/WorktreeListToast";
import { OpenTaskOptions } from "../../hooks/useOpenTask";
import {
  parseQuery,
  parsePaletteInput,
  scopeValues,
  withScope,
  withoutScope,
} from "../../search/query";
import { buildCommands, CommandShortcut, WorkspaceAction } from "../../search/commands";
import { PaletteItem, searchPalette } from "../../search/searchPalette";
import { activatePaletteItem, PaletteActionDeps } from "../../search/runCommand";
import { EditorApp, Task, Workspace } from "../../types";
import { NavigationEntry, lastTaskVisits } from "../../navigation/history";
import { linearIssueUrl } from "../../utils";
import { useEditorOcclusion } from "../../hooks/useEditorOcclusion";
import { PaletteResults } from "./PaletteResults";

const EMPTY_ENTRIES: NavigationEntry[] = [];

interface QuickSearchModalProps {
  open: boolean;
  onClose: () => void;
  initialQuery: string;
  tasks: Task[];
  workspaces: Workspace[];
  selectedWorkspaceId: string | null;
  historyEntries?: NavigationEntry[];
  themeId: string;
  editorApp: EditorApp;
  /** Switch to the task's workspace and select it in the list. */
  onReveal: (task: Task) => void;
  /** Reveal and open the task (embedded view or external editor, per the editor setting). */
  onOpenTask: (task: Task, options?: OpenTaskOptions) => Promise<boolean>;
  onSelectWorkspace: (workspaceId: string) => void;
  onWorkspaceAction: (action: WorkspaceAction) => void;
  onThemeChange: (themeId: string) => void;
  onEditorChange: (editor: EditorApp) => void;
  onNewTask: () => void;
}

export function QuickSearchModal({
  open,
  onClose,
  initialQuery,
  tasks,
  workspaces,
  selectedWorkspaceId,
  historyEntries = EMPTY_ENTRIES,
  themeId,
  editorApp,
  onReveal,
  onOpenTask,
  onSelectWorkspace,
  onWorkspaceAction,
  onThemeChange,
  onEditorChange,
  onNewTask,
}: QuickSearchModalProps) {
  useEditorOcclusion(open);
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

  const lastVisitAt = useMemo(() => lastTaskVisits(historyEntries), [historyEntries]);

  const commands = useMemo(
    () =>
      open ? buildCommands({ workspaces, selectedWorkspaceId, tasks, themeId, editorApp }) : [],
    [open, workspaces, selectedWorkspaceId, tasks, themeId, editorApp]
  );

  const results = useMemo(
    () =>
      open
        ? searchPalette({
            tasks,
            workspaces,
            selectedWorkspaceId,
            query,
            lastVisitAt,
            commands,
          })
        : ([] as PaletteItem[]),
    [open, tasks, workspaces, selectedWorkspaceId, query, lastVisitAt, commands]
  );

  useEffect(() => {
    setActiveIndex((i) => Math.min(i, Math.max(results.length - 1, 0)));
  }, [results.length]);

  useEffect(() => {
    listRef.current?.querySelector('[data-active="true"]')?.scrollIntoView({ block: "nearest" });
  }, [activeIndex, results]);

  if (!open) return null;

  const paletteInput = parsePaletteInput(query);
  const parsedQuery = parseQuery(paletteInput.query);
  const { commandsOnly } = paletteInput;
  const scoped = scopeValues(parsedQuery).length > 0;
  const bareShortcutsEnabled =
    !commandsOnly && parsedQuery.terms.length === 0 && parsedQuery.negTerms.length === 0;
  const active = results[activeIndex];
  const taskResultIndexes = results
    .map((result, index) => (result.kind === "task" ? index : -1))
    .filter((index) => index >= 0);
  const taskCount = taskResultIndexes.length;
  const commandCount = results.length - taskCount;

  const setScoped = (next: boolean) => {
    if (!next) setQuery(withoutScope);
    else if (currentWorkspace) setQuery((q) => withScope(q, currentWorkspace.name));
    inputRef.current?.focus();
  };

  const deps: PaletteActionDeps = {
    tasks,
    workspaces,
    onClose,
    onSelectWorkspace,
    onWorkspaceAction,
    onThemeChange,
    onEditorChange,
    onNewTask,
    onOpenTask,
    showToast,
  };

  const activate = (item: PaletteItem) => void activatePaletteItem(item, deps);

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
    if (/^[0-9]$/.test(e.key) && !e.metaKey && !e.ctrlKey && !e.altKey && bareShortcutsEnabled) {
      const resultIndex = taskResultIndexes[Number(e.key)];
      if (resultIndex !== undefined) {
        e.preventDefault();
        setActiveIndex(resultIndex);
      }
      return;
    }
    const shortcutCommand = results.find(
      (item) =>
        item.kind === "command" &&
        item.command.shortcut &&
        commandShortcutMatches(e, item.command.shortcut) &&
        (item.command.shortcut.meta || bareShortcutsEnabled)
    );
    if (shortcutCommand) {
      e.preventDefault();
      activate(shortcutCommand);
      return;
    }
    if (e.key === "Enter" && active) {
      e.preventDefault();
      if (e.metaKey && active.kind === "task") {
        onReveal(active.result.task);
        onClose();
      } else {
        activate(active);
      }
      return;
    }
    if (
      e.key === "l" &&
      e.metaKey &&
      active?.kind === "task" &&
      active.result.task.linearIssueIdentifier
    ) {
      e.preventDefault();
      openUrl(
        linearIssueUrl(
          active.result.task.linearIssueIdentifier,
          active.result.workspace?.linearOrgUrlKey
        )
      );
      onClose();
    }
  };

  const emptyMessage = commandsOnly
    ? "No commands match"
    : tasks.length === 0
      ? "No tasks yet"
      : "No tasks match";

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/60 backdrop-blur-sm pt-[12vh]"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="w-[40rem] max-w-[92vw] bg-bg-secondary border border-border rounded-xl shadow-2xl flex flex-col max-h-[68vh] overflow-hidden">
        <div className="px-4 py-3 border-b border-border">
          <div className="relative">
            <SearchIcon className="absolute left-3 top-1/2 -translate-y-1/2 text-text-muted" />
            <input
              ref={inputRef}
              className="w-full pl-9 pr-3 py-2 rounded-lg border border-border bg-bg-tertiary text-sm text-text-primary placeholder:text-text-muted outline-none focus:border-accent transition-colors"
              placeholder="Search tasks and commands…"
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
            <span className="ml-auto text-[0.625rem] text-text-muted font-mono">
              in:web|api · repo: · branch: · -exclude · &gt;commands
            </span>
          </div>
        </div>

        <div ref={listRef} className="flex-1 overflow-y-auto">
          <PaletteResults
            results={results}
            activeIndex={activeIndex}
            emptyMessage={emptyMessage}
            onHover={setActiveIndex}
            onActivate={activate}
          />
        </div>

        <div className="flex-shrink-0 px-4 py-2 border-t border-border flex items-center gap-3 text-[0.625rem] text-text-muted font-mono flex-wrap">
          <Hint keys="↑↓">navigate</Hint>
          {taskCount > 0 && <Hint keys="0-9">jump</Hint>}
          <Hint keys="↵">{active?.kind === "command" ? "run" : "open"}</Hint>
          {active?.kind === "task" && (
            <>
              <Hint keys="⌘↵">reveal</Hint>
              <Hint keys="⌘L">linear</Hint>
            </>
          )}
          <Hint keys="esc">close</Hint>
          <span className="ml-auto">
            {taskCount > 0 && (
              <>
                {taskCount} task{taskCount !== 1 ? "s" : ""}
              </>
            )}
            {taskCount > 0 && commandCount > 0 && " · "}
            {commandCount > 0 && (
              <>
                {commandCount} command{commandCount !== 1 ? "s" : ""}
              </>
            )}
            {results.length === 0 && "0 results"}
          </span>
        </div>
      </div>

      {toast && <WorktreeListToast message={toast} />}
    </div>
  );
}

function commandShortcutMatches(
  event: React.KeyboardEvent<HTMLInputElement>,
  shortcut: CommandShortcut
): boolean {
  const metaPressed = event.metaKey || event.ctrlKey;
  return (
    event.key.toLowerCase() === shortcut.key.toLowerCase() &&
    metaPressed === Boolean(shortcut.meta) &&
    event.shiftKey === Boolean(shortcut.shift) &&
    !event.altKey
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
