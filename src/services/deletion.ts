import {
  DeleteOptions,
  OperationError,
  OperationErrorDetails,
  OperationResult,
  OperationWarning,
  operationErrorDetails,
} from "./operations";

type TeardownChoice = Pick<DeleteOptions, "approveTeardown" | "skipTeardown">;

export interface DeleteRequest {
  deleteWorktrees: boolean;
  teardown?: TeardownChoice;
}

export type DeleteOutcome =
  | { kind: "deleted"; warnings: OperationWarning[] }
  | {
      kind: "teardown";
      teardown: "approval" | "failed";
      message: string;
      scripts: OperationErrorDetails["teardown_needs_approval"]["scripts"];
    }
  | { kind: "failed"; message: string; canRemoveFromApp: boolean };

type BlockedDeleteOutcome = Exclude<DeleteOutcome, { kind: "deleted" }>;

function deleteOptions({ deleteWorktrees, teardown }: DeleteRequest): DeleteOptions {
  return { deleteWorktrees, force: deleteWorktrees, ...teardown };
}

export async function runDelete(
  remove: (options: DeleteOptions) => Promise<OperationResult<unknown>>,
  request: DeleteRequest
): Promise<DeleteOutcome> {
  try {
    const { warnings } = await remove(deleteOptions(request));
    return { kind: "deleted", warnings };
  } catch (error) {
    return blockedOutcome(error, request);
  }
}

function blockedOutcome(error: unknown, request: DeleteRequest): BlockedDeleteOutcome {
  const approval = operationErrorDetails(error, "teardown_needs_approval");
  if (approval)
    return {
      kind: "teardown",
      teardown: "approval",
      message: "Before deleting, these repositories want to run a teardown script:",
      scripts: approval.scripts,
    };
  const message = error instanceof Error ? error.message : String(error);
  if (error instanceof OperationError && error.code === "teardown_failed")
    return { kind: "teardown", teardown: "failed", message, scripts: [] };
  return { kind: "failed", message, canRemoveFromApp: request.deleteWorktrees };
}
