import { useDeferredValue } from "react";

const INITIAL_ROWS = 24;

/** Paints the first rows right away on mount and the rest in a deferred render. */
export function useProgressiveRows<T>(rows: T[], initialRows = INITIAL_ROWS): T[] {
  const showAll = useDeferredValue(true, rows.length > initialRows ? false : undefined);
  return showAll ? rows : rows.slice(0, initialRows);
}
