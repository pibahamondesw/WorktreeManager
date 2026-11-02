import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { AutomationRun, AutomationSummary, listAutomationRuns } from "../services/automations";
import {
  EMPTY_INBOX_MARKS,
  InboxItem,
  InboxMarks,
  badgeCount,
  freshEntries,
  inboxEntries,
  mark,
  unreadCount,
  withMarks,
} from "../services/automationInbox";
import { NotificationTarget } from "../services/agentActivity";
import { loadAutomationInbox, persist } from "../services/store";

/** Runs per automation the Inbox reads entries from. */
const INBOX_RUNS = 50;
const NOTIFICATION_OPEN_EVENT = "agent-notification-open";

interface LoadedRuns {
  /** Automations whose runs loaded, in the order listed. */
  loaded: string[];
  byName: Record<string, AutomationRun[]>;
}

function notify(item: InboxItem) {
  const target: NotificationTarget = {
    taskId: item.target?.taskId ?? "",
    surface: "chat",
    agent: item.target?.agent ?? "",
    inboxEntry: item.key,
  };
  void invoke("agent_attention", {
    sound: null,
    notification: { title: item.automation, body: item.title, target, taskOpen: false },
  }).catch(() => undefined);
}

/**
 * The automation Inbox: entries derived from the newest runs of every automation, with their read
 * and archived marks. New unread entries notify; clicking a notification opens its entry.
 */
export function useAutomationInbox(
  automations: AutomationSummary[],
  onOpenEntry: (item: InboxItem) => void
) {
  const [runs, setRuns] = useState<LoadedRuns>({ loaded: [], byName: {} });
  const [marks, setMarks] = useState<InboxMarks | null>(null);
  const marksRef = useRef(marks);

  useEffect(() => {
    void loadAutomationInbox()
      .catch(() => EMPTY_INBOX_MARKS)
      .then((loaded) => {
        marksRef.current = loaded;
        setMarks(loaded);
      });
  }, []);

  useEffect(() => {
    let current = true;
    const names = automations.map(({ entry }) => entry.name);
    void Promise.allSettled(names.map((name) => listAutomationRuns(name, null, INBOX_RUNS))).then(
      (results) => {
        if (!current) return;
        const next: LoadedRuns = { loaded: [], byName: {} };
        results.forEach((result, index) => {
          if (result.status !== "fulfilled") return;
          next.loaded.push(names[index]);
          next.byName[names[index]] = result.value.runs;
        });
        setRuns(next);
      }
    );
    return () => {
      current = false;
    };
  }, [automations]);

  const items = useMemo(() => {
    const loaded = automations
      .filter(({ entry }) => entry.name in runs.byName)
      .map(({ entry }) => ({ entry, runs: runs.byName[entry.name] }));
    return withMarks(inboxEntries(loaded), marks ?? EMPTY_INBOX_MARKS);
  }, [automations, runs, marks]);

  const known = useRef(new Map<string, Set<string>>());
  useEffect(() => {
    if (!marks) return;
    freshEntries(items, runs.loaded, known.current).forEach(notify);
  }, [items, runs.loaded, marks]);

  const updateMarks = useCallback((update: (marks: InboxMarks) => InboxMarks) => {
    const next = update(marksRef.current ?? EMPTY_INBOX_MARKS);
    marksRef.current = next;
    setMarks(next);
    void persist([["automationInbox", next]]).catch(() => undefined);
  }, []);

  const setRead = useCallback(
    (keys: string[], read: boolean) => updateMarks((current) => mark(current, "read", keys, read)),
    [updateMarks]
  );
  const setArchived = useCallback(
    (keys: string[], archived: boolean) =>
      updateMarks((current) => mark(current, "archived", keys, archived)),
    [updateMarks]
  );

  const latest = useRef({ items, onOpenEntry, setRead });
  useEffect(() => {
    latest.current = { items, onOpenEntry, setRead };
  }, [items, onOpenEntry, setRead]);

  useEffect(() => {
    const unlisten = listen<NotificationTarget>(NOTIFICATION_OPEN_EVENT, ({ payload }) => {
      const item = latest.current.items.find((candidate) => candidate.key === payload.inboxEntry);
      if (!item) return;
      latest.current.setRead([item.key], true);
      latest.current.onOpenEntry(item);
    });
    return () => {
      void unlisten.then((stop) => stop());
    };
  }, []);

  return {
    items,
    unread: unreadCount(items),
    badge: badgeCount(items),
    setRead,
    setArchived,
  };
}
