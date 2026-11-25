import { TOOL_VISIBILITY_OPTIONS, ToolVisibility } from "./toolVisibility";
import { saveToolVisibility } from "./toolVisibilityPreference";

export function ToolVisibilitySelect({ value }: { value: ToolVisibility }) {
  return (
    <select
      aria-label="Tool calls in chat"
      title="Tool calls in chat. Approvals, questions and errors always show."
      value={value}
      onChange={(event) => saveToolVisibility(event.target.value as ToolVisibility)}
      className="h-7 max-w-48 rounded-md border border-border bg-bg-tertiary px-2 text-xs text-text-primary outline-none focus:border-accent cursor-pointer"
    >
      {TOOL_VISIBILITY_OPTIONS.map((option) => (
        <option key={option.id} value={option.id}>
          {option.label}
        </option>
      ))}
    </select>
  );
}
