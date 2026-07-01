import { useMemo, useState } from "react";
import { Task, Workspace } from "../../types";
import { AddWorkspaceModal } from "./AddWorkspaceModal";
import { EditWorkspaceModal } from "./EditWorkspaceModal";
import { RemoveWorkspaceModal } from "./RemoveWorkspaceModal";
import { ThemePicker } from "../ui/ThemePicker";
import { PlusIcon, CloseIcon, GearIcon, SunIcon } from "../ui/Icons";

interface WorkspaceListProps {
  workspaces: Workspace[];
  tasks: Task[];
  selectedWorkspaceId: string | null;
  onSelect: (workspaceId: string) => void;
  onAdd: (workspace: Workspace) => void;
  onUpdate: (
    workspaceId: string,
    updates: Partial<Pick<Workspace, "name" | "linearApiKey" | "repos">>,
  ) => void;
  onRemove: (workspaceId: string) => void;
  showAddExternal?: boolean;
  onCloseAddExternal?: () => void;
  themeId: string;
  onThemeChange: (themeId: string) => void;
  defaultLinearApiKey?: string | null;
}

export function WorkspaceList({
  workspaces,
  tasks,
  selectedWorkspaceId,
  onSelect,
  onAdd,
  onUpdate,
  onRemove,
  showAddExternal,
  onCloseAddExternal,
  themeId,
  onThemeChange,
  defaultLinearApiKey,
}: WorkspaceListProps) {
  const [showAdd, setShowAdd] = useState(false);
  const [hoveredId, setHoveredId] = useState<string | null>(null);
  const [editWorkspace, setEditWorkspace] = useState<Workspace | null>(null);
  const [removeWorkspace, setRemoveWorkspace] = useState<Workspace | null>(null);
  const [showThemes, setShowThemes] = useState(false);

  const taskCountByWorkspaceId = useMemo(() => {
    const m = new Map<string, number>();
    for (const t of tasks) {
      m.set(t.workspaceId, (m.get(t.workspaceId) ?? 0) + 1);
    }
    return m;
  }, [tasks]);

  const addOpen = showAdd || !!showAddExternal;
  const closeAdd = () => {
    setShowAdd(false);
    onCloseAddExternal?.();
  };

  return (
    <aside className="w-60 h-full bg-bg-secondary border-r border-border flex flex-col">
      {/* Titlebar drag area */}
      <div className="h-[32px] flex-shrink-0" data-tauri-drag-region />
      {/* Header */}
      <div
        className="flex items-center justify-between px-4 h-12 border-b border-border flex-shrink-0"
        data-tauri-drag-region
      >
        <span
          className="text-xs font-semibold uppercase tracking-wider text-text-muted"
          data-tauri-drag-region
        >
          Workspaces
        </span>
        <button
          onClick={() => setShowAdd(true)}
          className="w-6 h-6 flex items-center justify-center rounded-md text-text-muted hover:text-text-primary hover:bg-bg-hover transition-colors cursor-pointer"
          title="Add workspace (P)"
        >
          <PlusIcon size={14} />
        </button>
      </div>

      {/* Workspace list */}
      <div className="flex-1 overflow-y-auto py-2 space-y-0.5">
        {workspaces.length === 0 && (
          <div className="px-4 py-8 text-center">
            <p className="text-sm text-text-muted">No workspaces yet</p>
            <button
              onClick={() => setShowAdd(true)}
              className="mt-2 text-sm text-accent hover:text-accent-hover transition-colors cursor-pointer"
            >
              Add your first workspace
            </button>
          </div>
        )}

        {workspaces.map((workspace) => {
          const activeTaskCount = taskCountByWorkspaceId.get(workspace.id) ?? 0;
          const repoCount = workspace.repos.length;
          return (
            <div
              key={workspace.id}
              onClick={() => onSelect(workspace.id)}
              onMouseEnter={() => setHoveredId(workspace.id)}
              onMouseLeave={() => setHoveredId(null)}
              className={`group flex items-center justify-between gap-2 px-4 py-2.5 mx-2 rounded-lg cursor-pointer transition-colors ${
                selectedWorkspaceId === workspace.id
                  ? "bg-bg-active text-text-primary"
                  : "text-text-secondary hover:bg-bg-hover hover:text-text-primary"
              }`}
            >
              <div className="flex items-center gap-3 min-w-0 flex-1">
                <div
                  className={`w-2 h-2 rounded-full flex-shrink-0 ${
                    selectedWorkspaceId === workspace.id ? "bg-accent" : "bg-border-light"
                  }`}
                />
                <div className="flex flex-col min-w-0 flex-1">
                  <div className="flex items-center gap-2 min-w-0">
                    <p className="text-sm font-medium truncate min-w-0">{workspace.name}</p>
                    <span
                      className={`inline-flex flex-shrink-0 items-center justify-center min-w-[1.125rem] rounded px-1 py-0.5 text-[10px] font-semibold tabular-nums ${
                        selectedWorkspaceId === workspace.id
                          ? "bg-bg-secondary text-text-muted"
                          : "bg-bg-hover text-text-muted"
                      }`}
                      title={`${activeTaskCount} active task${activeTaskCount !== 1 ? "s" : ""}`}
                    >
                      {activeTaskCount}
                    </span>
                  </div>
                  {repoCount > 1 && (
                    <p className="text-[10px] text-text-muted truncate">{repoCount} repos</p>
                  )}
                </div>
              </div>

              <div
                className={`flex items-center gap-0.5 flex-shrink-0 ${
                  hoveredId === workspace.id ? "opacity-100" : "opacity-0 pointer-events-none"
                }`}
              >
                <button
                  onClick={(e) => {
                    e.stopPropagation();
                    setEditWorkspace(workspace);
                  }}
                  className="w-6 h-6 flex items-center justify-center rounded text-text-muted hover:text-text-primary transition-colors cursor-pointer"
                  title="Workspace settings"
                >
                  <GearIcon size={12} />
                </button>
                <button
                  onClick={(e) => {
                    e.stopPropagation();
                    setRemoveWorkspace(workspace);
                  }}
                  className="w-6 h-6 flex items-center justify-center rounded text-text-muted hover:text-danger transition-colors cursor-pointer"
                >
                  <CloseIcon size={12} />
                </button>
              </div>
            </div>
          );
        })}
      </div>

      {/* Footer: theme settings */}
      <div className="flex-shrink-0 border-t border-border px-4 py-2">
        <button
          onClick={() => setShowThemes(true)}
          className="w-full flex items-center gap-2 px-2 py-1.5 rounded-lg text-text-muted hover:text-text-primary hover:bg-bg-hover transition-colors cursor-pointer text-xs"
          title="Change theme"
        >
          <SunIcon />
          Theme
        </button>
      </div>

      <AddWorkspaceModal
        open={addOpen}
        onClose={closeAdd}
        onAdd={(workspace) => {
          onAdd(workspace);
          closeAdd();
        }}
        defaultLinearApiKey={defaultLinearApiKey}
      />

      {editWorkspace && (
        <EditWorkspaceModal
          open={!!editWorkspace}
          onClose={() => setEditWorkspace(null)}
          workspace={editWorkspace}
          onSave={onUpdate}
        />
      )}

      {removeWorkspace && (
        <RemoveWorkspaceModal
          open={!!removeWorkspace}
          onClose={() => setRemoveWorkspace(null)}
          workspace={removeWorkspace}
          tasks={tasks.filter((t) => t.workspaceId === removeWorkspace.id)}
          onConfirm={() => {
            onRemove(removeWorkspace.id);
            setRemoveWorkspace(null);
          }}
        />
      )}

      <ThemePicker
        open={showThemes}
        onClose={() => setShowThemes(false)}
        currentThemeId={themeId}
        onThemeChange={onThemeChange}
      />
    </aside>
  );
}
