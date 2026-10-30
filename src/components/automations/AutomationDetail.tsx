import { KeyboardEvent, useState } from "react";
import {
  AUTOMATION_STATUS_LABELS,
  AutomationSummary,
  automationStatus,
  pauseReasonLabel,
  triggerSummary,
} from "../../services/automations";
import { EditorApp } from "../../types";
import { AutomationStatusDot } from "./AutomationStatusDot";
import { AutomationRuns } from "./AutomationRuns";
import { AutomationDefinition } from "./AutomationDefinition";

type Tab = "runs" | "definition";
const TABS: { id: Tab; label: string }[] = [
  { id: "runs", label: "Runs" },
  { id: "definition", label: "Definition" },
];

interface AutomationDetailProps {
  summary: AutomationSummary;
  editorApp: EditorApp;
}

export function AutomationDetail({ summary, editorApp }: AutomationDetailProps) {
  const [tab, setTab] = useState<Tab>(summary.entry.errors.length > 0 ? "definition" : "runs");
  const { entry } = summary;
  const status = automationStatus(summary);

  const switchTabWithArrows = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault();
    const step = event.key === "ArrowRight" ? 1 : -1;
    const index = TABS.findIndex((t) => t.id === tab);
    const next = TABS[(index + step + TABS.length) % TABS.length].id;
    setTab(next);
    event.currentTarget.parentElement
      ?.querySelector<HTMLElement>(`#automation-tab-${next}`)
      ?.focus();
  };

  return (
    <div className="flex flex-col h-full min-h-0">
      <div className="flex items-center gap-3 px-6 pt-5 pb-3 flex-shrink-0">
        <AutomationStatusDot status={status} />
        <h3 className="text-sm font-semibold text-text-primary truncate select-text">
          {entry.name}
        </h3>
        <span className="text-xs text-text-muted">{AUTOMATION_STATUS_LABELS[status]}</span>
      </div>

      <AutomationFacts summary={summary} />

      <div
        role="tablist"
        aria-label="Automation"
        className="flex gap-1 px-4 border-b border-border flex-shrink-0"
      >
        {TABS.map(({ id, label }) => (
          <button
            key={id}
            id={`automation-tab-${id}`}
            type="button"
            role="tab"
            aria-selected={tab === id}
            aria-controls="automation-tab-panel"
            tabIndex={tab === id ? 0 : -1}
            onClick={() => setTab(id)}
            onKeyDown={switchTabWithArrows}
            className={`px-3 py-2 -mb-px border-b-2 text-xs font-medium cursor-pointer transition-colors outline-none focus-visible:text-text-primary ${
              tab === id
                ? "border-accent text-text-primary"
                : "border-transparent text-text-muted hover:text-text-primary"
            }`}
          >
            {label}
          </button>
        ))}
      </div>

      <div
        id="automation-tab-panel"
        role="tabpanel"
        aria-labelledby={`automation-tab-${tab}`}
        className="flex-1 min-h-0 overflow-y-auto p-6"
      >
        {tab === "runs" ? (
          <AutomationRuns key={entry.name} name={entry.name} />
        ) : (
          <AutomationDefinition entry={entry} editorApp={editorApp} />
        )}
      </div>
    </div>
  );
}

function AutomationFacts({ summary }: { summary: AutomationSummary }) {
  const { entry, state, stateError } = summary;
  const pauseReason = pauseReasonLabel(state);
  const nextRun = state?.enabled && state.next_run_at ? new Date(state.next_run_at) : null;
  return (
    <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 px-6 pb-3 text-xs border-b border-border flex-shrink-0 select-text">
      {entry.definition?.description && (
        <>
          <dt className="text-text-muted">Description</dt>
          <dd className="text-text-secondary">{entry.definition.description}</dd>
        </>
      )}
      <dt className="text-text-muted">Trigger</dt>
      <dd className="text-text-secondary font-mono">
        {entry.definition
          ? triggerSummary(entry.definition)
          : "Unknown until the definition is valid"}
      </dd>
      <dt className="text-text-muted">Next run</dt>
      <dd className="text-text-secondary">
        {nextRun ? nextRun.toLocaleString() : "Not scheduled"}
      </dd>
      {pauseReason && (
        <>
          <dt className="text-text-muted">Paused</dt>
          <dd className="text-text-secondary">{pauseReason}</dd>
        </>
      )}
      {stateError && (
        <>
          <dt className="text-text-muted">State</dt>
          <dd className="text-danger break-words">{stateError}</dd>
        </>
      )}
    </dl>
  );
}
