import { ChatResponse, PendingRequest } from "../../services/chat";

export interface ApprovalDraft {
  plan?: string;
  message?: string;
}

export function approvalResponse(
  request: PendingRequest,
  draft: ApprovalDraft,
  decision: string
): ChatResponse {
  const response: ChatResponse = { decision };
  if (request.kind !== "approval") return response;
  if (request.feedback && ["deny", "denyAndStop"].includes(decision) && draft.message?.trim()) {
    response.message = draft.message.trim();
  }
  if (request.editablePlan && decision === "allow" && draft.plan !== undefined) {
    response.plan = draft.plan;
  }
  return response;
}
