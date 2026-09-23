import { useState } from "react";
import { Button } from "../ui/Button";
import { DiffText } from "./ChatItemView";
import { ChatQuestion, ChatResponse, PendingRequest } from "../../services/chat";

interface PendingRequestCardProps {
  request: PendingRequest;
  onRespond: (response: ChatResponse) => Promise<void>;
}

const looksLikeDiff = (text: string) => /^[+-]/m.test(text);

export function PendingRequestCard({ request, onRespond }: PendingRequestCardProps) {
  const [sending, setSending] = useState(false);
  const respond = async (response: ChatResponse) => {
    setSending(true);
    try {
      await onRespond(response);
    } finally {
      setSending(false);
    }
  };

  return (
    <section
      aria-label={request.title}
      className="rounded-lg border border-warning/40 bg-warning/5 px-3 py-2 flex flex-col gap-2"
    >
      <h4 className="text-sm font-medium text-text-primary select-text">{request.title}</h4>
      {request.detail &&
        (looksLikeDiff(request.detail) ? (
          <DiffText text={request.detail} />
        ) : (
          <pre className="text-xs font-mono text-text-secondary whitespace-pre-wrap select-text max-h-60 overflow-y-auto">
            {request.detail}
          </pre>
        ))}
      {request.kind === "approval" && (
        <div className="flex flex-wrap gap-2">
          {request.decisions.map((decision, i) => (
            <Button
              key={decision.id}
              variant={i === 0 ? "primary" : "secondary"}
              className="h-7 px-3 text-xs"
              disabled={sending}
              onClick={() => void respond({ decision: decision.id })}
            >
              {decision.label}
            </Button>
          ))}
        </div>
      )}
      {request.kind === "question" && (
        <QuestionForm questions={request.questions} sending={sending} onSubmit={respond} />
      )}
      {request.kind === "unsupported" && (
        <div>
          <Button
            variant="secondary"
            className="h-7 px-3 text-xs"
            disabled={sending}
            onClick={() => void respond({})}
          >
            Reject
          </Button>
        </div>
      )}
    </section>
  );
}

function QuestionForm({
  questions,
  sending,
  onSubmit,
}: {
  questions: ChatQuestion[];
  sending: boolean;
  onSubmit: (response: ChatResponse) => Promise<void>;
}) {
  const [answers, setAnswers] = useState<Record<string, string[]>>({});
  const [other, setOther] = useState<Record<string, string>>({});

  const toggle = (question: ChatQuestion, label: string) =>
    setAnswers((previous) => {
      const current = previous[question.id] ?? [];
      const next = question.multiSelect
        ? current.includes(label)
          ? current.filter((l) => l !== label)
          : [...current, label]
        : [label];
      return { ...previous, [question.id]: next };
    });

  const collected = Object.fromEntries(
    questions.map((question) => {
      const typed = other[question.id]?.trim();
      const chosen = answers[question.id] ?? [];
      return [question.id, typed ? [...chosen, typed] : chosen];
    })
  );
  const complete = questions.every((question) => collected[question.id].length > 0);

  return (
    <form
      className="flex flex-col gap-3"
      onSubmit={(e) => {
        e.preventDefault();
        if (complete) void onSubmit({ answers: collected });
      }}
    >
      {questions.map((question) => (
        <fieldset key={question.id} className="flex flex-col gap-1.5">
          <legend className="text-xs text-text-secondary mb-1 select-text">
            {question.header && <span className="font-medium">{question.header} · </span>}
            {question.question}
          </legend>
          <div className="flex flex-wrap gap-1.5">
            {question.options.map((option) => {
              const selected = (answers[question.id] ?? []).includes(option.label);
              return (
                <button
                  key={option.label}
                  type="button"
                  title={option.description}
                  aria-pressed={selected}
                  onClick={() => toggle(question, option.label)}
                  className={`h-7 px-3 rounded-md text-xs border cursor-pointer transition-colors ${
                    selected
                      ? "border-accent bg-accent/15 text-text-primary"
                      : "border-border text-text-secondary hover:bg-bg-hover"
                  }`}
                >
                  {option.label}
                </button>
              );
            })}
          </div>
          {(question.allowOther || question.options.length === 0) && (
            <input
              type={question.secret ? "password" : "text"}
              aria-label={`Answer: ${question.question}`}
              value={other[question.id] ?? ""}
              onChange={(e) =>
                setOther((previous) => ({ ...previous, [question.id]: e.target.value }))
              }
              placeholder={question.options.length ? "Other…" : "Answer"}
              className="h-7 rounded-md bg-bg-secondary border border-border px-2 text-xs text-text-primary focus:outline-none focus:border-accent"
            />
          )}
        </fieldset>
      ))}
      <div>
        <Button type="submit" className="h-7 px-3 text-xs" disabled={!complete || sending}>
          Answer
        </Button>
      </div>
    </form>
  );
}
