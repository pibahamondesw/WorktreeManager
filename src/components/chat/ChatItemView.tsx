import { memo, useMemo, useState } from "react";
import Markdown, { Options } from "react-markdown";
import rehypeHighlight from "rehype-highlight";
import remarkGfm from "remark-gfm";
import { ChevronDownIcon, CloseIcon, SteerIcon } from "../ui/Icons";
import { AskedQuestion, ChatItem, HistoryAction } from "../../services/chat";
import { MarkdownLink } from "./MarkdownLink";
import { MarkdownCodeBlock } from "./MarkdownCodeBlock";
import { HistoryMenu } from "./HistoryMenu";
import { PlanCard, ReferencedDocuments } from "./MarkdownPreviewCard";
import { useMarkdownPreview } from "./markdownPreviewContext";
import { isPlanItem } from "./markdownDocument";
import { SubagentSteps } from "./chatEntries";
import { splitMarkdownBlocks } from "./markdownBlocks";

const STATUS_STYLE: Record<string, string> = {
  inProgress: "text-text-muted animate-pulse",
  completed: "text-success",
  failed: "text-danger",
  declined: "text-warning",
  stopped: "text-warning",
};

const STATUS_LABEL: Record<string, string> = {
  inProgress: "running",
  completed: "done",
  failed: "failed",
  declined: "rejected",
  stopped: "stopped",
};

const REMARK_PLUGINS: Options["remarkPlugins"] = [remarkGfm];
const REHYPE_PLUGINS: Options["rehypePlugins"] = [[rehypeHighlight, { plainText: ["mermaid"] }]];
const MARKDOWN_COMPONENTS: Options["components"] = { a: MarkdownLink, pre: MarkdownCodeBlock };

const MarkdownBlock = memo(function MarkdownBlock({ text }: { text: string }) {
  return (
    <Markdown
      remarkPlugins={REMARK_PLUGINS}
      rehypePlugins={REHYPE_PLUGINS}
      components={MARKDOWN_COMPONENTS}
    >
      {text}
    </Markdown>
  );
});

export function MarkdownText({
  text,
  className = "text-sm",
}: {
  text: string;
  className?: string;
}) {
  const blocks = useMemo(() => splitMarkdownBlocks(text) ?? [text], [text]);
  return (
    <div className={`chat-markdown ${className} text-text-primary select-text break-words`}>
      {blocks.map((block, index) => (
        <MarkdownBlock key={index} text={block} />
      ))}
    </div>
  );
}

export function DiffText({ text }: { text: string }) {
  return (
    <pre className="text-xs font-mono select-text overflow-x-auto whitespace-pre p-2 rounded-md bg-bg-secondary">
      {text.split("\n").map((line, i) => (
        <div
          key={i}
          className={
            line.startsWith("+") && !line.startsWith("+++")
              ? "text-success"
              : line.startsWith("-") && !line.startsWith("---")
                ? "text-danger"
                : line.startsWith("@@")
                  ? "text-accent"
                  : "text-text-secondary"
          }
        >
          {line || " "}
        </div>
      ))}
    </pre>
  );
}

function StatusLabel({ status }: { status?: string }) {
  if (!status) return null;
  return (
    <span className={STATUS_STYLE[status] ?? "text-text-muted"}>
      {STATUS_LABEL[status] ?? status}
    </span>
  );
}

const NO_STEPS: ChatItem[] = [];

function SubagentCard({ item, steps }: { item: ChatItem; steps?: SubagentSteps }) {
  const [open, setOpen] = useState(false);
  const runSteps = (steps?.get(item.id) ?? NO_STEPS).filter(isVisibleStep);
  return (
    <section
      aria-label={`Subagent: ${item.title ?? "Subagent"}`}
      className="rounded-lg border border-border bg-bg-secondary/40 text-xs"
    >
      <button
        type="button"
        onClick={() => setOpen(!open)}
        aria-expanded={open}
        className="w-full flex items-center gap-2 px-3 py-1.5 text-left cursor-pointer"
      >
        <span className="text-text-muted">↳</span>
        <span className="flex-1 min-w-0 truncate text-text-secondary select-text">
          <span className="font-medium">Subagent</span> · {item.title ?? "Subagent"}
        </span>
        {runSteps.length > 0 && (
          <span className="text-text-muted">{plural(runSteps.length, "step")}</span>
        )}
        <StatusLabel status={item.status} />
        <ChevronDownIcon
          className={`opacity-50 transition-transform ${open ? "rotate-180" : ""}`}
        />
      </button>
      {open && (
        <div className="px-3 pb-2 flex flex-col gap-2">
          {item.detail && (
            <pre className="font-mono text-text-secondary whitespace-pre-wrap select-text max-h-40 overflow-y-auto p-2 rounded-md bg-bg-secondary">
              {item.detail}
            </pre>
          )}
          {runSteps.length > 0 && (
            <div className="flex flex-col gap-2 border-l border-border pl-3 max-h-96 overflow-y-auto">
              {runSteps.map((step) => (
                <ChatItemView key={step.id} item={step} steps={steps} />
              ))}
            </div>
          )}
          {item.text && <MarkdownText text={item.text} />}
        </div>
      )}
    </section>
  );
}

function Activity({ item }: { item: ChatItem }) {
  const [open, setOpen] = useState(false);
  const label = item.title ?? item.text;
  const hasDetail = !!item.detail || (!!item.title && !!item.text);
  return (
    <div className="rounded-lg border border-border bg-bg-secondary/40 text-xs">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        disabled={!hasDetail}
        aria-expanded={open}
        className="w-full flex items-center gap-2 px-3 py-1.5 text-left cursor-pointer disabled:cursor-default"
      >
        <span className="text-text-muted">
          {item.kind === "command" ? "$" : item.kind === "fileChange" ? "±" : "⚙"}
        </span>
        <span className="flex-1 min-w-0 truncate font-mono text-text-secondary select-text">
          {label}
        </span>
        <StatusLabel status={item.status} />
        {hasDetail && (
          <ChevronDownIcon
            className={`opacity-50 transition-transform ${open ? "rotate-180" : ""}`}
          />
        )}
      </button>
      {open && (
        <div className="px-3 pb-2 flex flex-col gap-2">
          {item.title && item.text && (
            <pre className="font-mono text-text-secondary whitespace-pre-wrap select-text">
              {item.text}
            </pre>
          )}
          {item.detail &&
            (item.kind === "fileChange" ? (
              <DiffText text={item.detail} />
            ) : (
              <pre className="font-mono text-text-secondary whitespace-pre-wrap select-text max-h-80 overflow-y-auto p-2 rounded-md bg-bg-secondary">
                {item.detail}
              </pre>
            ))}
        </div>
      )}
    </div>
  );
}

function QuestionAnswer({ question, status }: { question: AskedQuestion; status?: string }) {
  if (question.answer !== undefined)
    return (
      <p className="border-l-2 border-accent pl-2 text-sm text-text-primary whitespace-pre-wrap select-text">
        {question.answer}
      </p>
    );
  const waiting = status === "inProgress";
  return (
    <p className={`text-xs italic text-text-muted${waiting ? " animate-pulse" : ""}`}>
      {waiting ? "Waiting for an answer…" : "Not answered"}
    </p>
  );
}

function QuestionCard({ item }: { item: ChatItem }) {
  const questions = item.questions ?? [];
  const title = questions.length === 1 ? "Question" : "Questions";
  return (
    <section
      aria-label={title}
      className="rounded-lg border border-border bg-bg-secondary/40 px-3 py-2 flex flex-col gap-3"
    >
      <span className="text-xs font-medium text-text-muted">{title}</span>
      {questions.map((question, index) => (
        <div key={index} className="flex flex-col gap-1">
          <p className="text-xs text-text-secondary select-text [overflow-wrap:anywhere]">
            {question.header && <span className="font-medium">{question.header} · </span>}
            {question.question}
          </p>
          <QuestionAnswer question={question} status={item.status} />
        </div>
      ))}
      {item.text && (
        <p className="text-xs text-text-muted whitespace-pre-wrap select-text">{item.text}</p>
      )}
    </section>
  );
}

function Reasoning({ item }: { item: ChatItem }) {
  const [open, setOpen] = useState(false);
  const thinking = item.status === "inProgress";
  const label = thinking
    ? "Thinking…"
    : item.durationMs === undefined
      ? "Thought"
      : `Thought for ${item.durationMs < 1000 ? "<1" : Math.round(item.durationMs / 1000)}s`;
  if (!isVisibleStep(item)) return null;
  if (!item.text.trim()) {
    return (
      <span className={`self-start text-xs text-text-muted${thinking ? " animate-pulse" : ""}`}>
        {label}
      </span>
    );
  }
  return (
    <div className="flex flex-col gap-2">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        aria-expanded={open}
        className="self-start flex items-center gap-1 text-xs text-text-muted hover:text-text-secondary cursor-pointer"
      >
        <span className={thinking ? "animate-pulse" : undefined}>{label}</span>
        <ChevronDownIcon
          className={`opacity-50 transition-transform ${open ? "rotate-180" : ""}`}
        />
      </button>
      {open && (
        <p className="border-l border-border pl-3 text-xs italic text-text-muted whitespace-pre-wrap select-text">
          {item.text}
        </p>
      )}
    </div>
  );
}

const isVisibleStep = (item: ChatItem) =>
  item.kind !== "reasoning" || !!item.text || !!item.status || item.durationMs !== undefined;
const isFailedStep = (item: ChatItem) => item.status === "failed" || item.status === "declined";
const plural = (count: number, word: string) => `${count} ${word}${count === 1 ? "" : "s"}`;

const sameItems = (a: ChatItem[], b: ChatItem[]) =>
  a.length === b.length && a.every((item, index) => item === b[index]);

export const WorkGroup = memo(
  WorkGroupView,
  (prev, next) => prev.running === next.running && sameItems(prev.items, next.items)
);

function WorkGroupView({ items, running }: { items: ChatItem[]; running: boolean }) {
  const [open, setOpen] = useState(false);
  const steps = items.filter(isVisibleStep);
  const failed = steps.filter(isFailedStep).length;
  if (steps.length === 0) return null;
  return (
    <div className="flex flex-col gap-2">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        aria-expanded={open}
        className="self-start flex items-center gap-1 text-xs text-text-muted hover:text-text-secondary cursor-pointer"
      >
        <span className={running ? "animate-pulse" : undefined}>
          {running ? "Working" : "Worked"} · {plural(steps.length, "step")}
          {failed > 0 && (
            <>
              {" · "}
              <span className="text-danger">{failed} failed</span>
            </>
          )}
        </span>
        <ChevronDownIcon
          className={`opacity-50 transition-transform ${open ? "rotate-180" : ""}`}
        />
      </button>
      {open && (
        <div className="flex flex-col gap-2 border-l border-border pl-3">
          {steps.map((item) => (
            <ChatItemView key={item.id} item={item} />
          ))}
        </div>
      )}
    </div>
  );
}

/** What can be done with a message waiting for the turn in progress. */
export interface QueuedActions {
  /** Absent when the agent cannot redirect a running turn. */
  steer?: (itemId: string) => void;
  remove: (itemId: string) => void;
}

const QUEUE_BUTTON =
  "inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-text-secondary hover:bg-bg-hover hover:text-text-primary cursor-pointer";

function MessageQueueState({ item, queue }: { item: ChatItem; queue?: QueuedActions }) {
  if (item.status === "steered") {
    return (
      <p
        title="Sent to redirect the turn that was running"
        className="mt-1 text-right text-xs text-text-muted"
      >
        Steered
      </p>
    );
  }
  if (item.status !== "queued") return null;
  const steer = queue?.steer;
  return (
    <div className="mt-1 flex items-center justify-end gap-1 text-xs">
      <span className="text-text-muted">Queued</span>
      {steer && (
        <button
          type="button"
          onClick={() => steer(item.id)}
          title="Send it now, redirecting the turn in progress"
          className={QUEUE_BUTTON}
        >
          <SteerIcon />
          Steer
        </button>
      )}
      {queue && (
        <button
          type="button"
          onClick={() => queue.remove(item.id)}
          aria-label="Remove queued message"
          title="Remove queued message"
          className={QUEUE_BUTTON}
        >
          <CloseIcon size={10} />
        </button>
      )}
    </div>
  );
}

/** Callbacks must be stable so a row only re-renders when its own item changes. */
export const ChatItemView = memo(ChatItemContent);

function ChatItemContent({
  item,
  steps,
  onHistory,
  historyDisabled,
  queue,
}: {
  item: ChatItem;
  steps?: SubagentSteps;
  onHistory?: (item: ChatItem, action: HistoryAction) => void;
  historyDisabled?: boolean;
  queue?: QueuedActions;
}) {
  const preview = useMarkdownPreview();
  switch (item.kind) {
    case "user": {
      const queued = item.status === "queued";
      return (
        <div className="group relative self-end max-w-[85%]">
          <div
            className={`chat-user-message rounded-lg bg-bg-tertiary px-3 py-2${queued ? " opacity-60" : ""}`}
          >
            <MarkdownText text={item.text} />
          </div>
          <MessageQueueState item={item} queue={queue} />
          {onHistory && !queued && (
            <HistoryMenu
              onPick={(action) => onHistory(item, action)}
              disabled={historyDisabled}
              canRewind={!!item.checkpoint}
            />
          )}
        </div>
      );
    }
    case "assistant":
      return (
        <>
          <MarkdownText text={item.text} />
          <ReferencedDocuments text={item.text} />
        </>
      );
    case "reasoning":
      return <Reasoning item={item} />;
    case "error":
      return (
        <p role="alert" className="text-sm text-danger whitespace-pre-wrap select-text">
          {item.text}
        </p>
      );
    case "notice":
      if (isPlanItem(item) && preview) return <PlanCard text={item.text} />;
      return (
        <div className="text-xs text-text-muted select-text">
          {item.title && <span className="font-medium">{item.title}: </span>}
          <span className="whitespace-pre-wrap">{item.text}</span>
        </div>
      );
    case "handoff":
      return (
        <div role="separator" aria-label={item.text} className="flex flex-col gap-1 py-2">
          <div className="flex items-center gap-2 text-xs font-medium text-text-secondary">
            <span className="h-px flex-1 bg-border" />
            <span className="select-text">{item.text}</span>
            <span className="h-px flex-1 bg-border" />
          </div>
          {item.detail && (
            <p className="text-center text-[0.6875rem] text-text-muted select-text">
              {item.detail}
            </p>
          )}
        </div>
      );
    case "review":
      return (
        <section
          aria-label={item.title ?? "Review"}
          className="rounded-lg border border-border bg-bg-secondary/40 px-3 py-2 flex flex-col gap-1"
        >
          <span className="text-xs font-medium text-text-muted">{item.title ?? "Review"}</span>
          <MarkdownText text={item.text} />
        </section>
      );
    case "agent":
      return <SubagentCard item={item} steps={steps} />;
    case "question":
      return <QuestionCard item={item} />;
    default:
      return <Activity item={item} />;
  }
}
