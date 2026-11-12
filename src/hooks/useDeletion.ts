import { useSyncExternalStore } from "react";
import { DeleteOptions, OperationResult } from "../services/operations";
import { DeleteOutcome, DeleteRequest, runDelete } from "../services/deletion";

interface DeletionState {
  outcome: DeleteOutcome | null;
  deleting: boolean;
}

const IDLE: DeletionState = { outcome: null, deleting: false };
const deletions = new Map<string, DeletionState>();
const listeners = new Set<() => void>();

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function setDeletion(key: string, state: DeletionState) {
  if (state.deleting || state.outcome) deletions.set(key, state);
  else deletions.delete(key);
  listeners.forEach((listener) => listener());
}

export function useDeletion(
  key: string,
  remove: (options: DeleteOptions) => Promise<OperationResult<unknown>>
) {
  const { outcome, deleting } = useSyncExternalStore(subscribe, () => deletions.get(key) ?? IDLE);

  const run = async (request: DeleteRequest) => {
    setDeletion(key, { outcome: null, deleting: true });
    const result = await runDelete(remove, request);
    setDeletion(key, { outcome: result, deleting: false });
    return result;
  };

  return { outcome, deleting, run, dismiss: () => setDeletion(key, IDLE) };
}
