import { KeyboardEvent } from "react";
import {
  AUTOMATION_STATUS_LABELS,
  AutomationSummary,
  automationStatus,
} from "../../services/automations";
import { AutomationStatusDot } from "./AutomationStatusDot";

interface AutomationListProps {
  automations: AutomationSummary[];
  error: string | null;
  selectedName: string | null;
  onSelect: (name: string) => void;
}

function moveFocus(event: KeyboardEvent<HTMLUListElement>) {
  const step = event.key === "ArrowDown" ? 1 : event.key === "ArrowUp" ? -1 : 0;
  if (!step) return;
  const items = [...event.currentTarget.querySelectorAll<HTMLButtonElement>("button")];
  const index = items.indexOf(document.activeElement as HTMLButtonElement);
  if (index === -1) return;
  event.preventDefault();
  items[Math.min(Math.max(index + step, 0), items.length - 1)].focus();
}

function statusDetail(summary: AutomationSummary): string {
  const status = automationStatus(summary);
  const errors = summary.entry.errors.length;
  if (status === "invalid") return `Invalid · ${errors} error${errors === 1 ? "" : "s"}`;
  return AUTOMATION_STATUS_LABELS[status];
}

export function AutomationList({
  automations,
  error,
  selectedName,
  onSelect,
}: AutomationListProps) {
  return (
    <section
      aria-label="Automations"
      className="flex-shrink-0 max-h-[40%] flex flex-col border-t border-border"
    >
      <h2 className="px-4 pt-3 pb-1 text-xs font-semibold uppercase tracking-wider text-text-muted">
        Automations
      </h2>
      {error && <p className="px-4 py-1 text-xs text-danger break-words select-text">{error}</p>}
      {!error && automations.length === 0 && (
        <p className="px-4 py-1 text-xs text-text-muted">No automations yet</p>
      )}
      <ul className="overflow-y-auto pb-2 space-y-0.5" onKeyDown={moveFocus}>
        {automations.map((summary) => {
          const { name } = summary.entry;
          const selected = name === selectedName;
          return (
            <li key={name}>
              <button
                type="button"
                onClick={() => onSelect(name)}
                aria-current={selected ? "page" : undefined}
                title={summary.entry.definition?.description ?? undefined}
                className={`w-[calc(100%-1rem)] mx-2 flex items-center gap-2 px-4 py-1.5 rounded-lg text-left cursor-pointer transition-colors outline-none focus-visible:ring-1 focus-visible:ring-accent ${
                  selected
                    ? "bg-bg-active text-text-primary"
                    : "text-text-secondary hover:bg-bg-hover hover:text-text-primary"
                }`}
              >
                <AutomationStatusDot status={automationStatus(summary)} />
                <span className="flex flex-col min-w-0">
                  <span className="text-sm truncate">{name}</span>
                  <span className="text-[0.625rem] text-text-muted truncate">
                    {statusDetail(summary)}
                  </span>
                </span>
              </button>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
