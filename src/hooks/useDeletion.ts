import { useState } from "react";
import { DeleteOptions, OperationResult } from "../services/operations";
import { DeleteOutcome, DeleteRequest, runDelete } from "../services/deletion";

export function useDeletion(remove: (options: DeleteOptions) => Promise<OperationResult<unknown>>) {
  const [outcome, setOutcome] = useState<DeleteOutcome | null>(null);
  const [deleting, setDeleting] = useState(false);

  const run = async (request: DeleteRequest) => {
    setOutcome(null);
    setDeleting(true);
    const result = await runDelete(remove, request);
    setDeleting(false);
    setOutcome(result);
    return result;
  };

  return { outcome, deleting, run, dismiss: () => setOutcome(null) };
}
