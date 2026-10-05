import { Workspace } from "../../types";

interface RepositoryChecklistProps {
  repos: Workspace["repos"];
  selection: Record<string, { included: boolean }>;
  disabled: boolean;
  onChange: (repoId: string, included: boolean) => void;
}

export function RepositoryChecklist({
  repos,
  selection,
  disabled,
  onChange,
}: RepositoryChecklistProps) {
  if (repos.length < 2) return null;
  return (
    <div className="flex flex-col gap-2">
      <p className="text-xs text-text-muted">Repositories in this task</p>
      <div className="rounded-lg border border-border bg-bg-tertiary divide-y divide-border/50">
        {repos.map((repo) => {
          const included = selection[repo.id]?.included ?? false;
          return (
            <label key={repo.id} className="flex items-center gap-2 px-3 py-2 cursor-pointer">
              <input
                type="checkbox"
                disabled={disabled}
                checked={included}
                onChange={(e) => onChange(repo.id, e.target.checked)}
                className="accent-accent"
              />
              <span
                className={`text-sm truncate ${included ? "text-text-primary" : "text-text-muted"}`}
              >
                {repo.name}
              </span>
            </label>
          );
        })}
      </div>
    </div>
  );
}
