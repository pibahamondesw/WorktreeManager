import { KeyboardEvent } from "react";
import {
  AUTOMATION_STATUS_LABELS,
  AutomationSummary,
  automationStatus,
} from "../../services/automations";
import { AutomationStatusDot } from "./AutomationStatusDot";

interface AutomationListProps {
  automations: AutomationSummary[];
  selectedName: string | null;
  onSelect: (name: string) => void;
}

const automationItemId = (name: string) => `automation-item-${name}`;

function statusDetail(summary: AutomationSummary): string {
  const status = automationStatus(summary);
  const errors = summary.entry.errors.length;
  if (status === "invalid") return `Invalid · ${errors} error${errors === 1 ? "" : "s"}`;
  return AUTOMATION_STATUS_LABELS[status];
}

export function AutomationList({ automations, selectedName, onSelect }: AutomationListProps) {
  const selectWithKeys = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    let next: number;
    if (event.key === "ArrowDown") next = (index + 1) % automations.length;
    else if (event.key === "ArrowUp") next = (index + automations.length - 1) % automations.length;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = automations.length - 1;
    else return;
    event.preventDefault();
    const name = automations[next].entry.name;
    onSelect(name);
    document.getElementById(automationItemId(name))?.focus();
  };

  return (
    <div role="tablist" aria-label="Automations" aria-orientation="vertical" className="space-y-1">
      {automations.map((summary, index) => {
        const { name } = summary.entry;
        const selected = name === selectedName;
        return (
          <button
            key={name}
            id={automationItemId(name)}
            type="button"
            role="tab"
            aria-selected={selected}
            aria-controls="automation-panel"
            tabIndex={selected ? 0 : -1}
            onClick={() => onSelect(name)}
            onKeyDown={(event) => selectWithKeys(event, index)}
            title={summary.entry.definition?.description ?? undefined}
            className={`w-full flex items-center gap-2 px-3 py-2 rounded-lg text-left transition-colors cursor-pointer focus-visible:outline-accent ${
              selected
                ? "bg-bg-active text-text-primary"
                : "text-text-secondary hover:bg-bg-hover hover:text-text-primary"
            }`}
          >
            <AutomationStatusDot status={automationStatus(summary)} />
            <span className="flex flex-col min-w-0">
              <span className="text-xs truncate">{name}</span>
              <span className="text-[0.625rem] text-text-muted truncate">
                {statusDetail(summary)}
              </span>
            </span>
          </button>
        );
      })}
    </div>
  );
}
