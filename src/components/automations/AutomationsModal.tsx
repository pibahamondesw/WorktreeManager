import { useEffect, useMemo, useRef } from "react";
import {
  ApprovalAnswer,
  AutomationSummary,
  signalAutomationStep,
} from "../../services/automations";
import { EditorApp, Task } from "../../types";
import { Modal } from "../ui/Modal";
import { AutomationList } from "./AutomationList";
import { AutomationDetail } from "./AutomationDetail";
import { AutomationInbox } from "./AutomationInbox";
import { OpenConversation, OpenConversationContext } from "./openConversation";
import { AutomationTasksContext } from "./automationTasks";
import { InboxItem } from "../../services/automationInbox";

const NO_TASKS: Task[] = [];

interface InboxProps {
  items: InboxItem[];
  unread: number;
  setRead: (keys: string[], read: boolean) => void;
  setArchived: (keys: string[], archived: boolean) => void;
  /** The entry to focus when the Inbox shows, such as a clicked notification's. */
  focusKey?: string | null;
}

interface AutomationsModalProps {
  open: boolean;
  onClose: () => void;
  automations: AutomationSummary[];
  error: string | null;
  /** Reloads the automations after an action changed one. */
  onChanged: () => void;
  /** Falls back to the first automation when unset or no longer listed. */
  selectedName: string | null;
  onSelect: (name: string) => void;
  editorApp: EditorApp;
  inbox: InboxProps;
  /** The Inbox replaces the selected automation's detail. */
  inboxSelected: boolean;
  onSelectInbox: () => void;
  onOpenConversation: OpenConversation;
  /** Campaign items show their task's branch and issue. */
  tasks?: Task[];
}

export function AutomationsModal({
  open,
  onClose,
  automations,
  error,
  onChanged,
  selectedName,
  onSelect,
  editorApp,
  inbox,
  inboxSelected,
  onSelectInbox,
  onOpenConversation,
  tasks = NO_TASKS,
}: AutomationsModalProps) {
  const selected =
    automations.find((summary) => summary.entry.name === selectedName) ?? automations[0];
  const contentRef = useRef<HTMLDivElement>(null);
  const taskIds = useMemo(() => new Set(tasks.map((task) => task.id)), [tasks]);

  useEffect(() => {
    if (!open) return;
    const previous = document.activeElement;
    contentRef.current?.querySelector<HTMLElement>('[aria-selected="true"]')?.focus();
    return () => {
      if (previous instanceof HTMLElement && previous.isConnected) previous.focus();
    };
  }, [open]);

  return (
    <Modal open={open} onClose={onClose} title="Automations" size="settings" trapFocus>
      {open && (
        <OpenConversationContext.Provider value={onOpenConversation}>
          <div ref={contentRef} className="flex h-full min-h-0">
            <div className="w-52 shrink-0 border-r border-border p-3 overflow-y-auto bg-bg-tertiary/30">
              {error && (
                <p className="px-1 pb-2 text-xs text-danger break-words select-text">{error}</p>
              )}
              <AutomationList
                automations={automations}
                selectedName={selected?.entry.name ?? null}
                onSelect={onSelect}
                inbox={{ unread: inbox.unread, selected: inboxSelected, onSelect: onSelectInbox }}
              />
              {automations.length === 0 && !error && (
                <p className="px-3 pt-2 text-xs text-text-muted">No automations yet</p>
              )}
            </div>
            <div
              id="automation-panel"
              role="tabpanel"
              aria-label={inboxSelected ? "Inbox" : selected?.entry.name}
              className="flex-1 min-w-0 min-h-0"
            >
              {inboxSelected ? (
                <AutomationInbox
                  items={inbox.items}
                  onOpen={(item) => (item.target ? onOpenConversation(item.target) : true)}
                  onSetRead={inbox.setRead}
                  onSetArchived={inbox.setArchived}
                  onAnswer={(item, answer) => answerApproval(item, answer).then(onChanged)}
                  taskIds={taskIds}
                  focusKey={inbox.focusKey}
                />
              ) : (
                selected && (
                  <AutomationTasksContext.Provider value={tasks}>
                    <AutomationDetail
                      key={selected.entry.name}
                      summary={selected}
                      editorApp={editorApp}
                      onChanged={onChanged}
                    />
                  </AutomationTasksContext.Provider>
                )
              )}
            </div>
          </div>
        </OpenConversationContext.Provider>
      )}
    </Modal>
  );
}

function answerApproval(item: InboxItem, answer: ApprovalAnswer) {
  const { runId, step, item: itemId } = item.approval ?? { runId: "", step: "", item: "" };
  return signalAutomationStep({
    name: item.automation,
    runId,
    step,
    item: itemId,
    payload: { ...answer },
  });
}
