import { ChatResponse, PendingRequest } from "../../services/chat";
import { ApprovalDraft, approvalResponse } from "./approval";
import { Button } from "../ui/Button";

export function ApprovalActions({
  request,
  draft,
  onDraftChange,
  sending,
  onRespond,
  onNewConversation,
  shortcuts = true,
}: {
  request: PendingRequest;
  draft: ApprovalDraft;
  onDraftChange: (draft: ApprovalDraft) => void;
  sending: boolean;
  onRespond: (response: ChatResponse) => Promise<void>;
  onNewConversation?: () => Promise<void>;
  shortcuts?: boolean;
}) {
  if (request.kind !== "approval") return null;
  const emptyPlan = request.editablePlan && !(draft.plan ?? request.detail ?? "").trim();
  return (
    <div className="flex w-full flex-col gap-2">
      {request.feedback && (
        <textarea
          aria-label="Rejection feedback (optional)"
          placeholder="Feedback if you reject (optional)"
          value={draft.message ?? ""}
          onChange={(e) => onDraftChange({ ...draft, message: e.target.value })}
          disabled={sending}
          rows={2}
          className="w-full resize-y rounded-md border border-border bg-bg-secondary px-2 py-1.5 text-xs text-text-primary focus:outline-none focus:border-accent"
        />
      )}
      <div className="flex flex-wrap gap-2">
        {request.decisions.map((decision, index) => (
          <Button
            key={decision.id}
            variant={index === 0 ? "primary" : "secondary"}
            className="min-h-7 max-w-full px-3 py-1 text-xs text-left"
            disabled={sending || (decision.id === "allow" && emptyPlan)}
            onClick={() => void onRespond(approvalResponse(request, draft, decision.id))}
          >
            {shortcuts && index < 9 && (
              <kbd aria-hidden="true" className="mr-1 shrink-0 font-mono opacity-50">
                {index + 1}
              </kbd>
            )}
            <span className="min-w-0 whitespace-pre-wrap [overflow-wrap:anywhere]">
              {decision.label}
            </span>
          </Button>
        ))}
        {request.editablePlan && onNewConversation && (
          <Button
            variant="secondary"
            className="min-h-7 px-3 py-1 text-xs"
            disabled={sending || emptyPlan}
            onClick={() => void onNewConversation()}
          >
            Approve in new conversation
          </Button>
        )}
      </div>
    </div>
  );
}
