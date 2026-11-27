import { useState } from "react";
import { DEFAULT_AUTO_SHELVE_INACTIVE_DAYS } from "../../services/preferences";

function parseDays(value: string) {
  const days = Number(value);
  return Number.isSafeInteger(days) && days > 0 ? days : null;
}

export function AutoShelveSetting({
  inactiveDays,
  onChange,
}: {
  inactiveDays: number | null;
  onChange: (inactiveDays: number | null) => void;
}) {
  const [draft, setDraft] = useState(String(inactiveDays ?? DEFAULT_AUTO_SHELVE_INACTIVE_DAYS));
  const enabled = inactiveDays !== null;

  const commitDraft = () => {
    const days = parseDays(draft);
    if (days === null) setDraft(String(inactiveDays ?? DEFAULT_AUTO_SHELVE_INACTIVE_DAYS));
    else if (days !== inactiveDays) onChange(days);
  };

  return (
    <div className="px-6 pt-4 space-y-1">
      <div className="flex items-center gap-2 text-sm text-text-primary">
        <label className="flex items-center gap-2 cursor-pointer">
          <input
            type="checkbox"
            checked={enabled}
            onChange={(event) =>
              onChange(
                event.target.checked
                  ? (parseDays(draft) ?? DEFAULT_AUTO_SHELVE_INACTIVE_DAYS)
                  : null
              )
            }
          />
          Move tasks inactive for
        </label>
        <input
          type="number"
          min={1}
          aria-label="Days without activity"
          value={draft}
          disabled={!enabled}
          onChange={(event) => setDraft(event.target.value)}
          onBlur={commitDraft}
          className="w-16 rounded-md border border-border bg-bg-tertiary px-2 py-0.5 text-sm text-text-primary outline-none focus:border-accent disabled:opacity-50"
        />
        days to Finished
      </div>
      <p className="text-xs text-text-muted">
        Tasks whose Linear issue is done or canceled, or where each repository has a pull request
        and all are merged or closed, move to Finished on their own. Activity is the last commit,
        visit or agent event; pinned tasks and tasks set to never move stay where they are.
      </p>
    </div>
  );
}
