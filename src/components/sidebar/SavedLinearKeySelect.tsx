import { useState } from "react";
import { Workspace } from "../../types";

interface SavedLinearKeySelectProps {
  workspaces: Workspace[];
  defaultLinearApiKey?: string | null;
  value: string;
  onChange: (key: string) => void;
}

export function SavedLinearKeySelect({
  workspaces,
  defaultLinearApiKey,
  value,
  onChange,
}: SavedLinearKeySelectProps) {
  const [enteringKey, setEnteringKey] = useState(false);
  const keys = new Map<string, { sources: string[]; org: string | null }>();
  const defaultKey = defaultLinearApiKey?.trim();
  if (defaultKey) keys.set(defaultKey, { sources: ["Default"], org: null });
  for (const workspace of workspaces) {
    const key = workspace.linearApiKey?.trim();
    if (!key) continue;
    const entry = keys.get(key) ?? { sources: [], org: null };
    entry.sources.push(workspace.name);
    entry.org ||= workspace.linearOrgUrlKey ?? null;
    keys.set(key, entry);
  }
  const options = [...keys.entries()];
  const selectedIndex = options.findIndex(([key]) => key === value.trim());

  return (
    <label className="flex flex-col gap-1.5 text-sm font-medium text-text-secondary">
      Linear connection
      <select
        className="rounded-lg border border-border bg-bg-tertiary px-3 py-2 text-sm text-text-primary"
        value={
          selectedIndex >= 0 ? String(selectedIndex) : value || enteringKey ? "custom" : "none"
        }
        onChange={(event) => {
          const selection = event.target.value;
          setEnteringKey(selection === "custom");
          onChange(
            selection === "none" || selection === "custom" ? "" : options[Number(selection)][0]
          );
        }}
      >
        <option value="none">No Linear connection</option>
        {options.map(([, entry], index) => (
          <option key={index} value={String(index)}>
            {[entry.org, entry.sources.join(", ")].filter(Boolean).join(" — ")}
          </option>
        ))}
        <option value="custom">Enter a new API key</option>
      </select>
    </label>
  );
}
