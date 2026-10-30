import { KeyboardEvent, useState } from "react";
import { useShortcutLabels } from "../../shortcuts/runtime";
import { useEditorOcclusion } from "../../hooks/useEditorOcclusion";
import {
  AUTOMATION_STATUS_LABELS,
  AutomationSummary,
  automationStatus,
  pauseReasonLabel,
  triggerSummary,
} from "../../services/automations";
import { EditorApp } from "../../types";
import { ChevronLeftIcon, SidebarIcon } from "../ui/Icons";
import { AutomationStatusDot } from "./AutomationStatusDot";
import { AutomationRuns } from "./AutomationRuns";
import { AutomationDefinition } from "./AutomationDefinition";

type Tab = "runs" | "definition";
const TABS: { id: Tab; label: string }[] = [
  { id: "runs", label: "Runs" },
  { id: "definition", label: "Definition" },
];

interface AutomationViewProps {
  summary: AutomationSummary;
  editorApp: EditorApp;
  sidebarCollapsed: boolean;
  onExpandSidebar: () => void;
  onClose: () => void;
}

export function AutomationView({
  summary,
  editorApp,
  sidebarCollapsed,
  onExpandSidebar,
  onClose,
}: AutomationViewProps) {
  useEditorOcclusion(true);
  const label = useShortcutLabels();
  const [tab, setTab] = useState<Tab>(summary.entry.errors.length > 0 ? "definition" : "runs");
  const { entry } = summary;

  const closeOnEscape = (event: KeyboardEvent) => {
    if (event.key !== "Escape" || event.defaultPrevented) return;
    event.preventDefault();
    onClose();
  };
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
    <div className="flex-1 flex flex-col min-h-0 min-w-0" onKeyDown={closeOnEscape}>
      <div
        className="flex items-center gap-3 px-4 min-h-12 py-2 border-b border-border flex-shrink-0"
        data-drag-region
      >
        {sidebarCollapsed && (
          <button
            type="button"
            onClick={onExpandSidebar}
            className="w-6 h-6 flex items-center justify-center flex-shrink-0 rounded-md text-text-muted hover:text-text-primary hover:bg-bg-hover transition-colors cursor-pointer"
            title={`Expand sidebar (${label("app.sidebar")})`}
          >
            <SidebarIcon size={14} />
          </button>
        )}
        <button
          type="button"
          onClick={onClose}
          aria-label="Back to tasks (Esc)"
          title="Back to tasks (Esc)"
          className="w-6 h-6 flex items-center justify-center flex-shrink-0 rounded-md text-text-muted hover:text-text-primary hover:bg-bg-hover transition-colors cursor-pointer"
        >
          <ChevronLeftIcon size={14} />
        </button>
        <AutomationStatusDot status={automationStatus(summary)} />
        <h2 className="text-sm font-semibold text-text-primary truncate">{entry.name}</h2>
        <span className="text-xs text-text-muted">
          {AUTOMATION_STATUS_LABELS[automationStatus(summary)]}
        </span>
      </div>

      <AutomationFacts summary={summary} />

      <div
        role="tablist"
        aria-label="Automation"
        className="flex gap-1 px-4 border-b border-border flex-shrink-0"
      >
        {TABS.map(({ id, label: tabLabel }) => (
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
            {tabLabel}
          </button>
        ))}
      </div>

      <div
        id="automation-tab-panel"
        role="tabpanel"
        aria-labelledby={`automation-tab-${tab}`}
        className="flex-1 overflow-y-auto p-6"
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
    <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 px-6 py-3 text-xs border-b border-border flex-shrink-0 select-text">
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
