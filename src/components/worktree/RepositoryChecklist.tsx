import { Workspace } from "../../types";

interface RepositoryChecklistProps {
  repos: Workspace["repos"];
  selection: Record<string, { included: boolean }>;
  disabled: boolean;
  onChange: (repoId: string, included: boolean) => void;
  onChangeAll: (included: boolean) => void;
}

export function RepositoryChecklist({
  repos,
  selection,
  disabled,
  onChange,
  onChangeAll,
}: RepositoryChecklistProps) {
  if (repos.length < 2) return null;
  const includedCount = repos.filter((repo) => selection[repo.id]?.included).length;
  const allIncluded = includedCount === repos.length;
  const someIncluded = includedCount > 0 && !allIncluded;
  return (
    <div className="flex flex-col gap-2">
      <label className="flex items-center gap-2 px-3 border-x border-transparent cursor-pointer">
        <input
          type="checkbox"
          disabled={disabled}
          checked={allIncluded}
          ref={(input) => {
            if (input) input.indeterminate = someIncluded;
          }}
          onChange={() => onChangeAll(!allIncluded)}
          className="accent-accent"
        />
        <span className="text-xs text-text-muted">
          Repositories in this task ({includedCount}/{repos.length})
        </span>
      </label>
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
