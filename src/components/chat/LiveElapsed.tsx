import { useCallback } from "react";
import { formatElapsed, onSecondTick, runStart } from "./runClock";

/**
 * Time since this window first saw `runKey` running. It rewrites its own text every second,
 * so a ticking count never re-renders the conversation around it.
 */
export function LiveElapsed({ runKey }: { runKey: string }) {
  const writeElapsed = useCallback(
    (node: HTMLSpanElement | null) => {
      if (!node) return;
      const startedAt = runStart(runKey);
      const write = () => {
        node.textContent = formatElapsed(Date.now() - startedAt);
      };
      write();
      return onSecondTick(write);
    },
    [runKey]
  );
  return <span ref={writeElapsed} aria-hidden="true" className="tabular-nums" />;
}
