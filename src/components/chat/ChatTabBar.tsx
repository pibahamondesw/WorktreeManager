import { KeyboardEvent, useState } from "react";
import { AgentIcon, CloseIcon, PlusIcon } from "../ui/Icons";
import { Button } from "../ui/Button";
import { TaskIndicatorDot } from "../ui/TaskIndicatorDot";
import { agentPresentation } from "../../services/agents";
import { ChatTab } from "../../services/chat";
import { TaskIndicator } from "../../services/agentActivity";

export interface ChatTabView extends ChatTab {
  title: string;
  agentLabel: string;
  canRename: boolean;
  indicator: TaskIndicator | null;
}

interface ChatTabBarProps {
  tabs: ChatTabView[];
  active: string | null;
  newLabel: string;
  onActivate: (tab: string) => void;
  onClose: (tab: ChatTabView) => void;
  onRename: (tab: ChatTabView, title: string) => void;
  onNew: () => void;
  onConversations: () => void;
  /** Receives the slot the active chat puts its own controls in. */
  toolbarRef: (element: HTMLElement | null) => void;
}

/** One tab per open conversation, the agent's logo first; closed ones live in Conversations. */
export function ChatTabBar({
  tabs,
  active,
  newLabel,
  onActivate,
  onClose,
  onRename,
  onNew,
  onConversations,
  toolbarRef,
}: ChatTabBarProps) {
  const [renaming, setRenaming] = useState<{ id: string; title: string } | null>(null);

  const moveFocus = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (event.key !== "ArrowRight" && event.key !== "ArrowLeft") return;
    const index = tabs.findIndex((tab) => tab.id === active);
    if (index < 0) return;
    event.preventDefault();
    const step = event.key === "ArrowRight" ? 1 : -1;
    const next = tabs[(index + step + tabs.length) % tabs.length];
    onActivate(next.id);
    event.currentTarget
      .closest('[role="tablist"]')
      ?.querySelector<HTMLElement>(`[data-tab="${next.id}"]`)
      ?.focus();
  };

  const saveRename = (tab: ChatTabView) => {
    const title = renaming?.title.trim();
    setRenaming(null);
    if (title && title !== tab.title) onRename(tab, title);
  };

  return (
    <div className="flex items-center gap-1 shrink-0 h-9 px-2 border-b border-border">
      <div
        role="tablist"
        aria-label="Open conversations"
        className="flex items-center gap-0.5 min-w-0 overflow-x-auto"
      >
        {tabs.map((tab) => {
          const selected = tab.id === active;
          return (
            <div
              key={tab.id}
              role="presentation"
              className={`group flex items-center gap-1.5 h-7 min-w-0 max-w-[13rem] shrink-0 rounded-md pl-2 pr-1 text-xs transition-colors ${
                selected
                  ? "bg-bg-tertiary text-text-primary"
                  : "text-text-muted hover:bg-bg-hover hover:text-text-primary"
              }`}
            >
              <AgentIcon
                agent={tab.agent}
                size={12}
                className={`flex-shrink-0 ${agentPresentation(tab.agent).tone.text}`}
              />
              {renaming?.id === tab.id ? (
                <form
                  className="min-w-0"
                  onSubmit={(e) => {
                    e.preventDefault();
                    saveRename(tab);
                  }}
                >
                  <input
                    autoFocus
                    aria-label="Conversation name"
                    value={renaming.title}
                    onChange={(e) => setRenaming({ id: tab.id, title: e.target.value })}
                    onBlur={() => saveRename(tab)}
                    onKeyDown={(e) => {
                      if (e.key === "Escape") setRenaming(null);
                    }}
                    className="w-32 rounded bg-bg-primary px-1 text-xs text-text-primary outline-none border border-accent"
                  />
                </form>
              ) : (
                <button
                  type="button"
                  role="tab"
                  data-tab={tab.id}
                  aria-selected={selected}
                  tabIndex={selected ? 0 : -1}
                  title={`${tab.title} · ${tab.agentLabel}${tab.canRename ? " · Double-click to rename" : ""}`}
                  onClick={() => onActivate(tab.id)}
                  onKeyDown={moveFocus}
                  onDoubleClick={() =>
                    tab.canRename && setRenaming({ id: tab.id, title: tab.title })
                  }
                  className="min-w-0 truncate cursor-pointer outline-none focus-visible:underline"
                >
                  {tab.title}
                </button>
              )}
              {tab.indicator && <TaskIndicatorDot indicator={tab.indicator} />}
              <button
                type="button"
                aria-label={`Close conversation ${tab.title}`}
                title="Close conversation"
                onClick={() => onClose(tab)}
                className={`flex size-5 flex-shrink-0 items-center justify-center rounded text-text-muted hover:bg-bg-hover hover:text-text-primary cursor-pointer focus-visible:opacity-100 ${
                  selected ? "opacity-100" : "opacity-0 group-hover:opacity-100"
                }`}
              >
                <CloseIcon size={12} />
              </button>
            </div>
          );
        })}
      </div>
      <button
        type="button"
        aria-label={newLabel}
        title={newLabel}
        onClick={onNew}
        className="flex size-7 flex-shrink-0 items-center justify-center rounded-md text-text-muted hover:bg-bg-hover hover:text-text-primary cursor-pointer"
      >
        <PlusIcon />
      </button>
      <div ref={toolbarRef} className="ml-auto flex items-center gap-2 shrink-0" />
      <Button variant="ghost" onClick={onConversations} className="h-7 px-2 text-xs shrink-0">
        Conversations
      </Button>
    </div>
  );
}
