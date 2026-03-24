import { useState } from "react";
import { Repo, Worktree } from "../../types";
import { AddRepoModal } from "./AddRepoModal";
import { RemoveRepoModal } from "./RemoveRepoModal";
import { ThemePicker } from "../ui/ThemePicker";
import { PlusIcon, CloseIcon, SunIcon } from "../ui/Icons";

interface RepoListProps {
  repos: Repo[];
  worktrees: Worktree[];
  selectedRepoId: string | null;
  onSelect: (repoId: string) => void;
  onAdd: (repo: Repo) => void;
  onRemove: (repoId: string) => void;
  showAddExternal?: boolean;
  onCloseAddExternal?: () => void;
  themeId: string;
  onThemeChange: (themeId: string) => void;
  defaultLinearApiKey?: string | null;
}

export function RepoList({
  repos,
  worktrees,
  selectedRepoId,
  onSelect,
  onAdd,
  onRemove,
  showAddExternal,
  onCloseAddExternal,
  themeId,
  onThemeChange,
  defaultLinearApiKey,
}: RepoListProps) {
  const [showAdd, setShowAdd] = useState(false);
  const [hoveredId, setHoveredId] = useState<string | null>(null);
  const [removeRepo, setRemoveRepo] = useState<Repo | null>(null);
  const [showThemes, setShowThemes] = useState(false);

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
          Projects
        </span>
        <button
          onClick={() => setShowAdd(true)}
          className="w-6 h-6 flex items-center justify-center rounded-md text-text-muted hover:text-text-primary hover:bg-bg-hover transition-colors cursor-pointer"
          title="Add project (P)"
        >
          <PlusIcon size={14} />
        </button>
      </div>

      {/* Repo list */}
      <div className="flex-1 overflow-y-auto py-2">
        {repos.length === 0 && (
          <div className="px-4 py-8 text-center">
            <p className="text-sm text-text-muted">No projects yet</p>
            <button
              onClick={() => setShowAdd(true)}
              className="mt-2 text-sm text-accent hover:text-accent-hover transition-colors cursor-pointer"
            >
              Add your first project
            </button>
          </div>
        )}

        {repos.map((repo) => (
          <div
            key={repo.id}
            onClick={() => onSelect(repo.id)}
            onMouseEnter={() => setHoveredId(repo.id)}
            onMouseLeave={() => setHoveredId(null)}
            className={`group flex items-center justify-between px-4 py-2.5 mx-2 rounded-lg cursor-pointer transition-colors ${
              selectedRepoId === repo.id
                ? "bg-bg-active text-text-primary"
                : "text-text-secondary hover:bg-bg-hover hover:text-text-primary"
            }`}
          >
            <div className="flex items-center gap-3 min-w-0">
              <div
                className={`w-2 h-2 rounded-full flex-shrink-0 ${
                  selectedRepoId === repo.id ? "bg-accent" : "bg-border-light"
                }`}
              />
              <div className="min-w-0">
                <p className="text-sm font-medium truncate">{repo.name}</p>
              </div>
            </div>

            <button
              onClick={(e) => {
                e.stopPropagation();
                setRemoveRepo(repo);
              }}
              className={`flex-shrink-0 w-6 h-6 flex items-center justify-center rounded text-text-muted hover:text-danger transition-colors cursor-pointer ${
                hoveredId === repo.id ? "opacity-100" : "opacity-0 pointer-events-none"
              }`}
            >
              <CloseIcon size={12} />
            </button>
          </div>
        ))}
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

      <AddRepoModal
        open={addOpen}
        onClose={closeAdd}
        onAdd={(repo) => {
          onAdd(repo);
          closeAdd();
        }}
        defaultLinearApiKey={defaultLinearApiKey}
      />

      {removeRepo && (
        <RemoveRepoModal
          open={!!removeRepo}
          onClose={() => setRemoveRepo(null)}
          repo={removeRepo}
          worktrees={worktrees.filter((w) => w.repoId === removeRepo.id)}
          onConfirm={() => {
            onRemove(removeRepo.id);
            setRemoveRepo(null);
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
