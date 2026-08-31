import { useEffect, useRef } from "react";
import { LinearService } from "../services/linear";
import { Workspace } from "../types";

export function useLinearOrgKeyBackfill(
  workspaces: Workspace[],
  onResolved: (workspaceId: string, updates: { linearOrgUrlKey: string }) => void
): void {
  const attemptedRef = useRef(new Set<string>());

  useEffect(() => {
    const pending = workspaces.filter(
      (w) => w.linearApiKey && !w.linearOrgUrlKey && !attemptedRef.current.has(w.id)
    );
    if (pending.length === 0) return;

    let cancelled = false;
    void (async () => {
      for (const w of pending) {
        attemptedRef.current.add(w.id);
        const urlKey = await new LinearService(w.linearApiKey!).fetchOrgUrlKey();
        if (cancelled) return;
        if (urlKey) onResolved(w.id, { linearOrgUrlKey: urlKey });
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [workspaces, onResolved]);
}
