import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { PlanUsage } from "../services/usage";

export interface CachedUsage {
  usage: PlanUsage | null;
  updatedAt: number | null;
}

export function useCachedUsage(command: "claude_cached_usage" | "codex_cached_usage") {
  const [snapshot, setSnapshot] = useState<CachedUsage | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const refreshRef = useRef<() => Promise<void>>(async () => undefined);

  useEffect(() => {
    let disposed = false;
    let timer: ReturnType<typeof setTimeout>;
    let pending: Promise<void> | null = null;
    const refresh = () => {
      if (pending) return pending;
      pending = invoke<CachedUsage>(command)
        .then((next) => {
          if (disposed) return;
          setSnapshot(next);
          setError(null);
        })
        .catch((e: unknown) => {
          if (disposed) return;
          setSnapshot(null);
          setError(String(e));
        })
        .finally(() => {
          pending = null;
          if (!disposed) setLoading(false);
        });
      return pending;
    };
    refreshRef.current = refresh;
    const poll = async () => {
      await refresh();
      if (!disposed) timer = setTimeout(() => void poll(), 2000);
    };
    void poll();
    return () => {
      disposed = true;
      clearTimeout(timer);
      refreshRef.current = async () => undefined;
    };
  }, [command]);

  const refresh = useCallback(() => refreshRef.current(), []);
  return { snapshot, error, loading, refresh };
}
