import {
  useShortcutActions,
  useShortcutLabels,
  useShortcutOverrides,
  dispatchShortcut,
} from "../../shortcuts/runtime";
import { prKey, useGithubPrStatuses } from "../../services/github";
import { useSearchPrNumbers } from "../../hooks/useSearchPrNumbers";
import { ProjectFilter } from "../ui/ProjectFilter";
import { matchesProject } from "../../search/projects";
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
  sessionValues,
  withScope,
  withActiveSession,
  withoutScope,
  withoutActiveSession,
} from "../../search/query";
import { buildCommands, WorkspaceAction } from "../../search/commands";
import { PaletteItem, searchPalette } from "../../search/searchPalette";
import { activatePaletteItem, PaletteActionDeps } from "../../search/runCommand";
import { EditorApp, PullRequestInfo, Task, Workspace } from "../../types";
import { NavigationEntry, lastTaskVisits } from "../../navigation/history";
import { linearIssueUrl } from "../../utils";
import { useEditorOcclusion } from "../../hooks/useEditorOcclusion";
import { PaletteResults } from "./PaletteResults";
import { TerminalStatus } from "../../services/terminal";
import { AgentActivities } from "../../services/agentActivity";

const EMPTY_ENTRIES: NavigationEntry[] = [];
const EMPTY_ACTIVITIES: AgentActivities = {};

interface QuickSearchModalProps {
  onPrReady?: (taskId: string, pr: PullRequestInfo) => Promise<void>;
  open: boolean;
  onClose: () => void;
  initialQuery: string;
  tasks: Task[];
  workspaces: Workspace[];
  selectedWorkspaceId: string | null;
  historyEntries?: NavigationEntry[];
  agentSessions?: Record<string, TerminalStatus>;
  agentActivities?: AgentActivities;
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
  onPrReady,
  onClose,
  initialQuery,
  tasks,
  workspaces,
  selectedWorkspaceId,
  historyEntries = EMPTY_ENTRIES,
  agentSessions = {},
  agentActivities = EMPTY_ACTIVITIES,
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
  const shortcutOverrides = useShortcutOverrides();
  const label = useShortcutLabels();
  const [openedWith, setOpenedWith] = useState({ open, activities: agentActivities });
  if (openedWith.open !== open) setOpenedWith({ open, activities: agentActivities });
  const { prNumbersByTask, prsByTask } = useSearchPrNumbers(open, tasks, workspaces);
  const githubStatuses = useGithubPrStatuses();
  const readyPrsByTask = useMemo(
    () =>
      Object.fromEntries(
        Object.entries(prsByTask).map(([taskId, prs]) => [
          taskId,
          prs.filter((pr) => {
            const entry = githubStatuses[prKey(pr)];
            return entry?.status?.state === "open" && entry.status.isDraft && !entry.pendingReady;
          }),
        ])
      ),
    [prsByTask, githubStatuses]
  );
  const [project, setProject] = useState("");
  const [query, setQuery] = useState(initialQuery);
  const [activeIndex, setActiveIndex] = useState(0);
  const [ring, setRing] = useState<{ index: number; nonce: number }>();
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
    setProject("");
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
      open
        ? buildCommands({
            workspaces,
            selectedWorkspaceId,
            tasks,
            themeId,
            editorApp,
            readyPrsByTask,
            shortcutOverrides,
          })
        : [],
    [
      open,
      workspaces,
      selectedWorkspaceId,
      tasks,
      themeId,
      editorApp,
      readyPrsByTask,
      shortcutOverrides,
    ]
  );

  const results = useMemo(
    () =>
      open
        ? searchPalette({
            tasks: tasks.filter((task) => matchesProject(task, project)),
            workspaces,
            selectedWorkspaceId,
            query,
            prNumbersByTask,
            lastVisitAt,
            agentSessions,
            agentActivities: openedWith.activities,
            commands,
          })
        : ([] as PaletteItem[]),
    [
      open,
      tasks,
      workspaces,
      selectedWorkspaceId,
      query,
      project,
      prNumbersByTask,
      lastVisitAt,
      agentSessions,
      openedWith.activities,
      commands,
    ]
  );

  useEffect(() => {
    setActiveIndex((i) => Math.min(i, Math.max(results.length - 1, 0)));
  }, [results.length]);

  useEffect(() => {
    listRef.current?.querySelector('[data-active="true"]')?.scrollIntoView({ block: "nearest" });
  }, [activeIndex, results]);

  const paletteInput = parsePaletteInput(query);
  const parsedQuery = parseQuery(paletteInput.query);
  const { commandsOnly } = paletteInput;
  const scoped = scopeValues(parsedQuery).length > 0;
  const activeSessionsOnly = sessionValues(parsedQuery).includes("active");
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

  const setActiveSessionsOnly = (next: boolean) => {
    setQuery(next ? withActiveSession : withoutActiveSession);
    inputRef.current?.focus();
  };

  const deps: PaletteActionDeps = {
    onPrReady,
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
    dispatchShortcut(e.nativeEvent);
    if (e.nativeEvent.defaultPrevented) return;
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
    if (e.key === "Enter" && !e.metaKey && !e.ctrlKey && !e.altKey && active) {
      e.preventDefault();
      activate(active);
      return;
    }
  };

  useShortcutActions(
    {
      "palette.settings": {
        handler: () => {
          onClose();
          onWorkspaceAction({ kind: "settings" });
        },
      },
      "palette.new": {
        handler: () => {
          onClose();
          onNewTask();
        },
        enabled:
          !!currentWorkspace &&
          results.some((item) => item.kind === "command" && item.command.id === "new-task"),
        inTextFields: bareShortcutsEnabled,
      },
      "palette.linear": {
        handler: () => {
          if (active?.kind !== "task" || !active.result.task.linearIssueIdentifier) return;
          void openUrl(
            linearIssueUrl(
              active.result.task.linearIssueIdentifier,
              active.result.workspace?.linearOrgUrlKey
            )
          )
            .then(onClose)
            .catch(() => showToast("Could not open Linear"));
        },
        enabled: active?.kind === "task" && !!active.result.task.linearIssueIdentifier,
      },
      "palette.reveal": {
        handler: () => {
          if (active?.kind === "task") {
            onReveal(active.result.task);
            onClose();
          }
        },
        enabled: active?.kind === "task",
      },
      ...Object.fromEntries(
        Array.from({ length: 10 }, (_, i) => [
          `palette.jump.${i}`,
          {
            handler: () => {
              const index = taskResultIndexes[i];
              setActiveIndex(index);
              setRing((current) => ({ index, nonce: (current?.nonce ?? 0) + 1 }));
              inputRef.current?.focus();
            },
            enabled: taskResultIndexes[i] !== undefined,
          },
        ])
      ),
      ...Object.fromEntries(
        Array.from({ length: 10 }, (_, i) => [
          `palette.workspace.${i}`,
          {
            handler: () => {
              onSelectWorkspace(workspaces[i].id);
              onClose();
            },
            enabled:
              !!workspaces[i] &&
              results.some(
                (item) =>
                  item.kind === "command" && item.command.id === `switch-ws:${workspaces[i].id}`
              ),
          },
        ])
      ),
    },
    open
  );
  if (!open) return null;

  const emptyMessage = commandsOnly
    ? "No commands match"
    : tasks.length === 0
      ? "No tasks yet"
      : "No tasks match";

  return (
    <div
      role="dialog"
      aria-label="Search tasks"
      data-shortcut-context="palette"
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/60 backdrop-blur-sm pt-[12vh] motion-fade"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="motion-rise w-[40rem] max-w-[92vw] bg-bg-secondary border border-border rounded-xl shadow-2xl flex flex-col max-h-[68vh] overflow-hidden">
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
          <div className="flex items-center flex-wrap gap-1 mt-2">
            <ScopeTab active={!scoped} onClick={() => setScoped(false)}>
              All workspaces
            </ScopeTab>
            <ScopeTab active={scoped} onClick={() => setScoped(true)} disabled={!currentWorkspace}>
              This workspace
            </ScopeTab>
            <ScopeTab
              active={activeSessionsOnly}
              onClick={() => setActiveSessionsOnly(!activeSessionsOnly)}
            >
              Active sessions
            </ScopeTab>
            {!commandsOnly && (
              <ProjectFilter
                tasks={tasks}
                value={project}
                onChange={(value) => {
                  setProject(value);
                  setActiveIndex(0);
                  inputRef.current?.focus();
                }}
              />
            )}
            <span className="w-full text-[0.625rem] text-text-muted font-mono">
              in:web|api · project: · session:active · repo: · branch: · -exclude · &gt;commands
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
            ring={ring}
          />
        </div>

        <div className="flex-shrink-0 px-4 py-2 border-t border-border flex items-center gap-3 text-[0.625rem] text-text-muted font-mono flex-wrap">
          <Hint keys="↑↓">navigate</Hint>
          {taskCount > 0 && <Hint keys={label("palette.jump.0")}>jump to first</Hint>}
          <Hint keys="↵">{active?.kind === "command" ? "run" : "open"}</Hint>
          {active?.kind === "task" && (
            <>
              <Hint keys={label("palette.reveal")}>reveal</Hint>
              <Hint keys={label("palette.linear")}>linear</Hint>
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
