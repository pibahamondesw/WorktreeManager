import { v4 as uuid } from "uuid";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Button } from "../ui/Button";
import { CloseIcon } from "../ui/Icons";
import { WorkspaceRepo } from "../../types";

interface WorkspaceRepoEditorProps {
  repos: WorkspaceRepo[];
  onChange: (repos: WorkspaceRepo[]) => void;
  home: string;
}

/** Default worktree base for a repo: ~/Documents/.worktreemanager/worktrees/<slug>. */
function computeDefaultWorktreeBase(home: string, name: string): string {
  if (!home || !name.trim()) return "";
  const slug = name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/(^-|-$)/g, "");
  const base = home.endsWith("/") ? home : `${home}/`;
  return `${base}Documents/.worktreemanager/worktrees/${slug}`;
}

/** Editor for a workspace's peer member repos (add via Browse, name, worktree base, default mode). */
export function WorkspaceRepoEditor({ repos, onChange, home }: WorkspaceRepoEditorProps) {
  const addRepo = async () => {
    const selected = await openDialog({
      directory: true,
      multiple: false,
      title: "Select repo directory",
    });
    if (!selected) return;
    const path = selected as string;
    const parts = path.split("/");
    const folderName = parts[parts.length - 1] || parts[parts.length - 2] || "repo";
    onChange([
      ...repos,
      {
        id: uuid(),
        name: folderName,
        localPath: path,
        worktreeBasePath: computeDefaultWorktreeBase(home, folderName),
      },
    ]);
  };

  const update = (id: string, patch: Partial<WorkspaceRepo>) =>
    onChange(repos.map((r) => (r.id === id ? { ...r, ...patch } : r)));

  const remove = (id: string) => onChange(repos.filter((r) => r.id !== id));

  return (
    <div className="flex flex-col gap-2">
      <label className="text-sm font-medium text-text-secondary">Repositories</label>
      {repos.length === 0 && (
        <p className="text-xs text-text-muted">
          Add one or more repos — they open together as one workspace.
        </p>
      )}
      <div className="flex flex-col gap-3">
        {repos.map((r) => (
          <div key={r.id} className="rounded-lg border border-border bg-bg-tertiary p-3 space-y-2">
            <div className="flex items-center gap-2">
              <input
                className="flex-1 rounded-md border border-border bg-bg-secondary px-2 py-1 text-sm text-text-primary outline-none focus:border-accent"
                value={r.name}
                placeholder="Repo name"
                onChange={(e) => update(r.id, { name: e.target.value })}
              />
              <button
                onClick={() => remove(r.id)}
                className="w-6 h-6 flex items-center justify-center rounded text-text-muted hover:text-danger transition-colors cursor-pointer flex-shrink-0"
                title="Remove repo"
              >
                <CloseIcon size={12} />
              </button>
            </div>
            <p className="text-xs text-text-muted font-mono truncate" title={r.localPath}>
              {r.localPath}
            </p>
            <input
              className="w-full rounded-md border border-border bg-bg-secondary px-2 py-1 text-xs text-text-primary outline-none focus:border-accent font-mono"
              value={r.worktreeBasePath}
              placeholder="Worktree base path"
              onChange={(e) => update(r.id, { worktreeBasePath: e.target.value })}
            />
          </div>
        ))}
      </div>
      <Button variant="secondary" onClick={addRepo} className="self-start">
        + Add repo
      </Button>
    </div>
  );
}
