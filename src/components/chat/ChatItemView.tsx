import { useState } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { ChevronDownIcon } from "../ui/Icons";
import { ChatItem } from "../../services/chat";

const STATUS_STYLE: Record<string, string> = {
  inProgress: "text-text-muted animate-pulse",
  completed: "text-success",
  failed: "text-danger",
  declined: "text-warning",
};

const STATUS_LABEL: Record<string, string> = {
  inProgress: "running",
  completed: "done",
  failed: "failed",
  declined: "rejected",
};

export function MarkdownText({ text }: { text: string }) {
  return (
    <div className="chat-markdown text-sm text-text-primary select-text break-words">
      <Markdown remarkPlugins={[remarkGfm]}>{text}</Markdown>
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

function Activity({ item }: { item: ChatItem }) {
  const [open, setOpen] = useState(false);
  const label = item.title ?? item.text;
  const hasDetail = !!item.detail || (!!item.title && !!item.text);
  const status = item.status ?? "";
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
        {status && (
          <span className={STATUS_STYLE[status] ?? "text-text-muted"}>
            {STATUS_LABEL[status] ?? status}
          </span>
        )}
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

export function ChatItemView({ item }: { item: ChatItem }) {
  switch (item.kind) {
    case "user":
      return (
        <div className="self-end max-w-[85%] rounded-lg bg-bg-tertiary px-3 py-2 text-sm text-text-primary whitespace-pre-wrap select-text">
          {item.text}
        </div>
      );
    case "assistant":
      return <MarkdownText text={item.text} />;
    case "reasoning":
      return item.text ? (
        <p className="text-xs italic text-text-muted whitespace-pre-wrap select-text">
          {item.text}
        </p>
      ) : null;
    case "error":
      return (
        <p role="alert" className="text-sm text-danger whitespace-pre-wrap select-text">
          {item.text}
        </p>
      );
    case "notice":
      return (
        <div className="text-xs text-text-muted select-text">
          {item.title && <span className="font-medium">{item.title}: </span>}
          <span className="whitespace-pre-wrap">{item.text}</span>
        </div>
      );
    default:
      return <Activity item={item} />;
  }
}
