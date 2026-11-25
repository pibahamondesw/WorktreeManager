import { useEffect } from "react";
import { LruCache } from "./lruCache";

const MAX_TRACKED_RUNS = 200;
const TICK_MS = 1000;

const runStarts = new LruCache<number>(MAX_TRACKED_RUNS, MAX_TRACKED_RUNS);
const tickListeners = new Set<() => void>();
let tickTimer: ReturnType<typeof setInterval> | undefined;

export function onSecondTick(listener: () => void) {
  tickListeners.add(listener);
  tickTimer ??= setInterval(() => tickListeners.forEach((tick) => tick()), TICK_MS);
  return () => {
    tickListeners.delete(listener);
    if (tickListeners.size > 0) return;
    clearInterval(tickTimer);
    tickTimer = undefined;
  };
}

/** When this window first saw `runKey` running. */
export function runStart(runKey: string) {
  const known = runStarts.get(runKey);
  if (known !== undefined) return known;
  const now = Date.now();
  runStarts.set(runKey, now, 1);
  return now;
}

/** Restarts the count the next time `runKey` runs; a run that is seen again keeps its start. */
export function useForgetRunWhenStopped(runKey: string, running: boolean) {
  useEffect(() => {
    if (!running) runStarts.delete(runKey);
  }, [runKey, running]);
}

const twoDigits = (value: number) => String(value).padStart(2, "0");

export function formatElapsed(ms: number) {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ${twoDigits(seconds % 60)}s`;
  return `${Math.floor(minutes / 60)}h ${twoDigits(minutes % 60)}m`;
}
