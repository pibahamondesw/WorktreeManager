import { useState } from "react";
import { ApprovalAnswer, ApprovalDefault } from "../../services/automations";
import { ApprovalRequest, editedParams, paramText } from "../../services/automationInbox";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";

interface AutomationApprovalProps {
  title: string;
  request: ApprovalRequest;
  onAnswer: (answer: ApprovalAnswer) => Promise<unknown>;
}

/** Approve, with any edited params, or reject with a comment the agent receives. */
export function AutomationApproval({ title, request, onAnswer }: AutomationApprovalProps) {
  const [texts, setTexts] = useState<Record<string, string>>(() =>
    Object.fromEntries(
      Object.entries(request.params).map(([key, value]) => [key, paramText(value)])
    )
  );
  const [comment, setComment] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [sending, setSending] = useState<ApprovalDefault | null>(null);

  const answer = (decision: ApprovalDefault) => {
    const edited = editedParams(request.params, texts);
    if ("error" in edited) {
      setError(edited.error);
      return;
    }
    const note = comment.trim();
    const params = decision === "approve" && Object.keys(edited.params).length > 0;
    setError(null);
    setSending(decision);
    onAnswer({
      decision,
      ...(note ? { comment: note } : {}),
      ...(params ? { params: edited.params } : {}),
    })
      .catch((reason: unknown) => setError(String(reason)))
      .finally(() => setSending(null));
  };

  return (
    <div role="group" aria-label={`Answer ${title}`} className="px-3 pb-3 pl-7 space-y-2">
      {Object.keys(texts).map((key) => (
        <Input
          key={key}
          label={key}
          value={texts[key]}
          onChange={(event) => setTexts({ ...texts, [key]: event.target.value })}
          className="h-7 py-1 text-xs font-mono"
        />
      ))}
      <textarea
        aria-label="Comment"
        placeholder="Comment for the agent when you reject"
        value={comment}
        onChange={(event) => setComment(event.target.value)}
        rows={2}
        className="w-full rounded-lg border border-border bg-bg-tertiary px-3 py-2 text-xs text-text-primary placeholder:text-text-muted outline-none focus:border-accent"
      />
      <div className="flex items-center gap-2">
        <Button
          className="h-7 px-3 text-xs"
          loading={sending === "approve"}
          disabled={sending !== null}
          onClick={() => answer("approve")}
        >
          Approve
        </Button>
        <Button
          variant="secondary"
          className="h-7 px-3 text-xs"
          loading={sending === "reject"}
          disabled={sending !== null}
          onClick={() => answer("reject")}
        >
          Reject
        </Button>
        {request.timeoutAt !== null && request.default && (
          <span className="text-[0.625rem] text-text-muted">
            {request.default === "approve" ? "Approves" : "Rejects"} on its own at{" "}
            {new Date(request.timeoutAt).toLocaleString()}
          </span>
        )}
      </div>
      {error && (
        <p role="alert" className="text-xs text-danger select-text">
          {error}
        </p>
      )}
    </div>
  );
}
