import { useCallback, useEffect, useRef, useState } from "react";
import { loadShowAutomationTasks, persist } from "../services/store";

/**
 * Whether the task list shows tasks created by automations. `choose` remembers the user's choice
 * across sessions; `reveal` shows them for this session only, to bring one task into view.
 */
export function useShowAutomationTasks() {
  const [show, setShow] = useState(true);
  const changedLocally = useRef(false);

  useEffect(() => {
    void loadShowAutomationTasks().then(
      (saved) => {
        if (!changedLocally.current) setShow(saved);
      },
      () => undefined
    );
  }, []);

  const reveal = useCallback(() => {
    changedLocally.current = true;
    setShow(true);
  }, []);

  const choose = useCallback((next: boolean) => {
    changedLocally.current = true;
    setShow(next);
    void persist([["showAutomationTasks", next]]).catch(() => undefined);
  }, []);

  return { show, reveal, choose };
}
