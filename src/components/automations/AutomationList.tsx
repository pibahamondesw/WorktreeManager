import { KeyboardEvent, ReactNode } from "react";
import {
  AUTOMATION_STATUS_LABELS,
  AutomationSummary,
  automationStatus,
} from "../../services/automations";
import { AutomationStatusDot } from "./AutomationStatusDot";

/** The Inbox tab, listed before the automations. */
interface InboxTab {
  unread: number;
  selected: boolean;
  onSelect: () => void;
}

interface AutomationListProps {
  automations: AutomationSummary[];
  selectedName: string | null;
  onSelect: (name: string) => void;
  inbox?: InboxTab;
}

interface ListTab {
  id: string;
  selected: boolean;
  select: () => void;
  label: string;
  detail: string;
  title?: string;
  dot: ReactNode;
}

const automationItemId = (name: string) => `automation-item-${name}`;
const INBOX_ITEM_ID = "automation-inbox-tab";

function statusDetail(summary: AutomationSummary): string {
  const status = automationStatus(summary);
  const errors = summary.entry.errors.length;
  if (status === "invalid") return `Invalid · ${errors} error${errors === 1 ? "" : "s"}`;
  return AUTOMATION_STATUS_LABELS[status];
}

function inboxListTab({ unread, selected, onSelect }: InboxTab): ListTab {
  return {
    id: INBOX_ITEM_ID,
    selected,
    select: onSelect,
    label: "Inbox",
    detail: unread > 0 ? `${unread} unread` : "All caught up",
    dot: (
      <span
        aria-hidden="true"
        className={`w-1.5 h-1.5 rounded-full flex-shrink-0 ${unread > 0 ? "bg-accent" : "bg-text-muted/40"}`}
      />
    ),
  };
}

export function AutomationList({
  automations,
  selectedName,
  onSelect,
  inbox,
}: AutomationListProps) {
  const tabs: ListTab[] = [
    ...(inbox ? [inboxListTab(inbox)] : []),
    ...automations.map((summary) => ({
      id: automationItemId(summary.entry.name),
      selected: !inbox?.selected && summary.entry.name === selectedName,
      select: () => onSelect(summary.entry.name),
      label: summary.entry.name,
      detail: statusDetail(summary),
      title: summary.entry.definition?.description ?? undefined,
      dot: <AutomationStatusDot status={automationStatus(summary)} />,
    })),
  ];

  const selectWithKeys = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    let next: number;
    if (event.key === "ArrowDown") next = (index + 1) % tabs.length;
    else if (event.key === "ArrowUp") next = (index + tabs.length - 1) % tabs.length;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = tabs.length - 1;
    else return;
    event.preventDefault();
    tabs[next].select();
    document.getElementById(tabs[next].id)?.focus();
  };

  return (
    <div role="tablist" aria-label="Automations" aria-orientation="vertical" className="space-y-1">
      {tabs.map((tab, index) => (
        <button
          key={tab.id}
          id={tab.id}
          type="button"
          role="tab"
          aria-selected={tab.selected}
          aria-controls="automation-panel"
          tabIndex={tab.selected ? 0 : -1}
          onClick={tab.select}
          onKeyDown={(event) => selectWithKeys(event, index)}
          title={tab.title}
          className={`w-full flex items-center gap-2 px-3 py-2 rounded-lg text-left transition-colors cursor-pointer focus-visible:outline-accent ${
            tab.selected
              ? "bg-bg-active text-text-primary"
              : "text-text-secondary hover:bg-bg-hover hover:text-text-primary"
          }`}
        >
          {tab.dot}
          <span className="flex flex-col min-w-0">
            <span className="text-xs truncate">{tab.label}</span>
            <span className="text-[0.625rem] text-text-muted truncate">{tab.detail}</span>
          </span>
        </button>
      ))}
    </div>
  );
}
