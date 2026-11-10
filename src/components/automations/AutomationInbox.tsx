import { KeyboardEvent, useEffect, useRef, useState } from "react";
import { InboxItem, InboxKind, unreadCount } from "../../services/automationInbox";
import { ApprovalAnswer } from "../../services/automations";
import { AutomationApproval } from "./AutomationApproval";
import { Badge } from "../ui/Badge";
import { Button } from "../ui/Button";

type View = "inbox" | "archived";

const KIND_BADGES: Record<
  InboxKind,
  { label: string; variant: "default" | "success" | "warning" | "danger" | "accent" }
> = {
  findings: { label: "Findings", variant: "accent" },
  failed: { label: "Failed", variant: "danger" },
  needs_attention: { label: "Needs attention", variant: "warning" },
  question: { label: "Question", variant: "warning" },
  permission: { label: "Permission", variant: "warning" },
  approval: { label: "Approval", variant: "accent" },
  taken_over: { label: "Taken over", variant: "warning" },
  noop: { label: "No-op", variant: "default" },
  completed: { label: "Completed", variant: "success" },
};

const MISSING_TASK = "The task of this entry no longer exists";

interface AutomationInboxProps {
  items: InboxItem[];
  /** Opens the entry's conversation; false when its task is gone. */
  onOpen: (item: InboxItem) => boolean;
  onSetRead: (keys: string[], read: boolean) => void;
  onSetArchived: (keys: string[], archived: boolean) => void;
  /** Answers a pending approval entry. */
  onAnswer?: (item: InboxItem, answer: ApprovalAnswer) => Promise<unknown>;
  /** The app's tasks; an entry whose task is not among them has nothing to open. */
  taskIds?: ReadonlySet<string>;
  /** The entry to focus on mount, after the modal places its own focus. */
  focusKey?: string | null;
}

/** Enter opens an entry, arrows move between entries, E archives and U toggles read. */
export function AutomationInbox({
  items,
  onOpen,
  onSetRead,
  onSetArchived,
  onAnswer,
  taskIds,
  focusKey,
}: AutomationInboxProps) {
  const [view, setView] = useState<View>("inbox");
  const [focusIndex, setFocusIndex] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const listRef = useRef<HTMLUListElement>(null);
  const visible = items.filter((item) => item.archived === (view === "archived"));
  const unread = unreadCount(items);
  const focusable = Math.min(focusIndex, Math.max(visible.length - 1, 0));

  const focusEntry = (index: number) => {
    setFocusIndex(index);
    listRef.current?.querySelectorAll<HTMLElement>("[data-inbox-entry]")[index]?.focus();
  };

  useEffect(() => {
    const index = items.filter((item) => !item.archived).findIndex((item) => item.key === focusKey);
    if (index === -1) return;
    setFocusIndex(index);
    const timer = window.setTimeout(() =>
      listRef.current?.querySelectorAll<HTMLElement>("[data-inbox-entry]")[index]?.focus()
    );
    return () => window.clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- focuses once per requested entry
  }, [focusKey]);

  const taskRemoved = (item: InboxItem) =>
    !!item.target && !!taskIds && !taskIds.has(item.target.taskId);

  const open = (item: InboxItem) => {
    onSetRead([item.key], true);
    setError(item.target && !taskRemoved(item) && !onOpen(item) ? MISSING_TASK : null);
  };

  const handleKey = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    const item = visible[index];
    const key = event.key.toLowerCase();
    if (key === "e" && !item.autoArchived) onSetArchived([item.key], !item.archived);
    else if (key === "u") onSetRead([item.key], !item.read);
    else if (event.key === "ArrowDown") focusEntry(Math.min(index + 1, visible.length - 1));
    else if (event.key === "ArrowUp") focusEntry(Math.max(index - 1, 0));
    else if (event.key === "Home") focusEntry(0);
    else if (event.key === "End") focusEntry(visible.length - 1);
    else return;
    event.preventDefault();
  };

  return (
    <div className="flex flex-col h-full min-h-0">
      <div className="flex items-center gap-3 px-6 pt-5 pb-3 border-b border-border flex-shrink-0">
        <h3 className="text-sm font-semibold text-text-primary">Inbox</h3>
        <span className="text-xs text-text-muted">
          {unread > 0 ? `${unread} unread` : "All read"}
        </span>
        <div className="ml-auto flex items-center gap-1">
          {(["inbox", "archived"] as View[]).map((option) => (
            <Button
              key={option}
              variant={view === option ? "secondary" : "ghost"}
              aria-pressed={view === option}
              className="h-7 px-2 text-xs"
              onClick={() => {
                setView(option);
                setFocusIndex(0);
              }}
            >
              {option === "inbox" ? "Inbox" : "Archived"}
            </Button>
          ))}
          <Button
            variant="ghost"
            className="h-7 px-2 text-xs"
            disabled={unread === 0}
            onClick={() =>
              onSetRead(
                items.filter((item) => !item.read).map((item) => item.key),
                true
              )
            }
          >
            Mark all read
          </Button>
        </div>
      </div>
      {error && (
        <p role="alert" className="px-6 pt-3 text-xs text-danger select-text">
          {error}
        </p>
      )}
      <div className="flex-1 min-h-0 overflow-y-auto p-4">
        {visible.length === 0 ? (
          <p className="px-2 text-sm text-text-muted">
            {view === "inbox" ? "Nothing needs you" : "No archived entries"}
          </p>
        ) : (
          <ul ref={listRef} aria-label="Inbox entries" className="space-y-1">
            {visible.map((item, index) => (
              <InboxRow
                key={item.key}
                item={item}
                taskRemoved={taskRemoved(item)}
                focusable={index === focusable}
                onOpen={() => {
                  setFocusIndex(index);
                  open(item);
                }}
                onKeyDown={(event) => handleKey(event, index)}
                onSetRead={(read) => onSetRead([item.key], read)}
                onSetArchived={(archived) => onSetArchived([item.key], archived)}
                onAnswer={onAnswer && ((answer) => onAnswer(item, answer))}
              />
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

interface InboxRowProps {
  item: InboxItem;
  taskRemoved: boolean;
  focusable: boolean;
  onOpen: () => void;
  onKeyDown: (event: KeyboardEvent<HTMLButtonElement>) => void;
  onSetRead: (read: boolean) => void;
  onSetArchived: (archived: boolean) => void;
  onAnswer?: (answer: ApprovalAnswer) => Promise<unknown>;
}

function InboxRow({
  item,
  taskRemoved,
  focusable,
  onOpen,
  onKeyDown,
  onSetRead,
  onSetArchived,
  onAnswer,
}: InboxRowProps) {
  const badge = KIND_BADGES[item.kind];
  const meta = [
    item.automation,
    new Date(item.at).toLocaleString(),
    item.test ? "test run" : null,
    item.target ? null : "no task",
  ]
    .filter(Boolean)
    .join(" · ");
  const pending = item.approval && item.approval.answer === null && !item.archived;
  return (
    <li className="group rounded-lg hover:bg-bg-hover">
      <div className="flex items-start gap-2">
        <button
          type="button"
          data-inbox-entry
          tabIndex={focusable ? 0 : -1}
          onClick={onOpen}
          onKeyDown={onKeyDown}
          aria-label={`${item.read ? "" : "Unread: "}${item.title}`}
          className="flex-1 min-w-0 flex items-start gap-3 px-3 py-2 rounded-lg text-left cursor-pointer outline-none focus-visible:ring-2 focus-visible:ring-accent"
        >
          <span
            aria-hidden="true"
            className={`mt-1.5 w-1.5 h-1.5 rounded-full flex-shrink-0 ${item.read ? "" : "bg-accent"}`}
          />
          <span className="min-w-0 flex-1 space-y-0.5">
            <span className="flex items-center gap-2">
              <span
                className={`text-xs truncate ${item.read ? "text-text-secondary" : "font-semibold text-text-primary"}`}
              >
                {item.title}
              </span>
              <Badge variant={badge.variant}>{badge.label}</Badge>
            </span>
            {item.detail && (
              <span className="block text-xs text-text-secondary truncate">{item.detail}</span>
            )}
            <span className="block text-[0.625rem] text-text-muted truncate">{meta}</span>
            {taskRemoved && (
              <span
                className="block text-[0.625rem] text-text-muted"
                title="The entry's task and its worktrees were deleted, so there is nothing left to open."
              >
                Task removed
              </span>
            )}
          </span>
        </button>
        <span className="flex items-center gap-1 px-2 py-2 opacity-0 group-hover:opacity-100 group-focus-within:opacity-100">
          <Button
            variant="ghost"
            tabIndex={-1}
            className="h-6 px-2 text-[0.625rem]"
            onClick={() => onSetRead(!item.read)}
          >
            {item.read ? "Mark unread" : "Mark read"}
          </Button>
          {!item.autoArchived && (
            <Button
              variant="ghost"
              tabIndex={-1}
              className="h-6 px-2 text-[0.625rem]"
              onClick={() => onSetArchived(!item.archived)}
            >
              {item.archived ? "Unarchive" : "Archive"}
            </Button>
          )}
        </span>
      </div>
      {pending && item.approval && onAnswer && (
        <AutomationApproval title={item.title} request={item.approval} onAnswer={onAnswer} />
      )}
    </li>
  );
}
