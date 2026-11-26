import { TOOL_VISIBILITY_OPTIONS, ToolVisibility } from "../chat/toolVisibility";
import { saveToolVisibility, useToolVisibility } from "../chat/toolVisibilityPreference";

export function ToolVisibilitySettings() {
  const visibility = useToolVisibility();
  return (
    <div className="px-6 pt-2 space-y-1">
      <label className="flex items-center justify-between gap-4 text-sm text-text-primary">
        Tool calls in chat
        <select
          value={visibility}
          onChange={(event) => saveToolVisibility(event.target.value as ToolVisibility)}
          className="bg-bg-tertiary border border-border rounded-md px-2 py-1 text-xs text-text-primary cursor-pointer"
        >
          {TOOL_VISIBILITY_OPTIONS.map((option) => (
            <option key={option.id} value={option.id}>
              {option.label}
            </option>
          ))}
        </select>
      </label>
      <p className="text-xs text-text-muted">
        Applies to every chat. Approvals, questions and errors always show.
      </p>
    </div>
  );
}
