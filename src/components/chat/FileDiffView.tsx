import { KeyboardEvent, MouseEvent, useEffect, useMemo, useState } from "react";
import { Button } from "../ui/Button";
import { PlusIcon, SpinnerIcon } from "../ui/Icons";
import { ChangeSet, chatFileDiff, FileChange, FileContent, FileDiff } from "../../services/chat";
import {
  DiffBlock,
  diffBlocks,
  DiffLine,
  lineSegments,
  SourceAnalysis,
  semanticDiff,
} from "../../services/semanticDiff";
import { analyzeSource } from "../../services/syntaxAnalysis";
import { AgentId } from "../../types";
import {
  addDiffComment,
  diffAnchor,
  diffKeyOf,
  DiffRange,
  DiffThread,
  editDiffComment,
  removeDiffComment,
  replyToDiffThread,
} from "./diffComments";

export type DiffLayout = "unified" | "split";

export type DiffThreadsUpdate = (update: (threads: DiffThread[]) => DiffThread[]) => void;

interface TextDiff {
  lines: DiffLine[] | null;
  analysis: Record<"old" | "new", SourceAnalysis>;
}

type Loaded =
  | { kind: "text"; diff: TextDiff }
  | { kind: "media"; before: FileContent | null; after: FileContent | null };

const lineId = (line: DiffLine) => `${line.side}:${line.start}`;

const textOf = (content: FileContent | null) => (content?.kind === "text" ? content.text : "");

async function loadDiff(file: FileChange, diff: FileDiff): Promise<Loaded> {
  const isText = (content: FileContent | null) => !content || content.kind === "text";
  if (!isText(diff.before) || !isText(diff.after)) {
    return { kind: "media", before: diff.before, after: diff.after };
  }
  const oldText = textOf(diff.before);
  const newText = textOf(diff.after);
  const [oldAnalysis, newAnalysis] = await Promise.all([
    analyzeSource(file.previousPath ?? file.path, oldText),
    analyzeSource(file.path, newText),
  ]);
  return {
    kind: "text",
    diff: {
      lines: semanticDiff(oldText, newText, oldAnalysis, newAnalysis),
      analysis: { old: oldAnalysis, new: newAnalysis },
    },
  };
}

function formatSize(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

function MediaSide({ label, content }: { label: string; content: FileContent | null }) {
  const [size, setSize] = useState<string | null>(null);
  return (
    <figure className="flex-1 min-w-0 flex flex-col gap-1.5">
      <figcaption className="text-xs text-text-muted">
        {label}
        {size && ` · ${size}`}
      </figcaption>
      {content?.kind === "image" ? (
        <div className="diff-image-frame min-h-40 rounded-md border border-border p-2 flex items-center justify-center">
          <img
            src={`data:${content.mime};base64,${content.data}`}
            alt={label}
            className="max-w-full max-h-[60vh] object-contain"
            onLoad={(e) =>
              setSize(`${e.currentTarget.naturalWidth}×${e.currentTarget.naturalHeight}`)
            }
          />
        </div>
      ) : (
        <div className="rounded-md border border-dashed border-border p-6 text-xs text-text-muted text-center">
          {content?.kind === "binary"
            ? `Binary file · ${formatSize(content.size)}`
            : content
              ? "Text file"
              : "No file"}
        </div>
      )}
    </figure>
  );
}

const SIGN: Record<DiffLine["kind"], string> = { added: "+", removed: "−", context: " " };

function lineClass(line: DiffLine | undefined) {
  if (!line) return "diff-empty";
  if (line.kind === "context") return "";
  if (line.moved) return "diff-moved";
  if (line.formatting) return "diff-formatting";
  return line.kind === "added" ? "diff-added" : "diff-removed";
}

function lineTitle(line: DiffLine) {
  if (line.kind === "context") return undefined;
  if (line.moved) return "Moved line";
  if (line.formatting) return "Formatting-only change";
  return undefined;
}

function Code({ line, analysis }: { line: DiffLine; analysis: TextDiff["analysis"] }) {
  return (
    <>
      {lineSegments(line, analysis[line.side].highlights).map((segment, index) => (
        <span
          key={index}
          className={`${segment.className ?? ""} ${segment.changed ? "diff-changed" : ""}`}
        >
          {segment.text}
        </span>
      ))}
    </>
  );
}

function LineNumber({ value }: { value?: number }) {
  return (
    <span className="diff-number w-10 shrink-0 pr-2 text-right text-text-muted select-none">
      {value ?? ""}
    </span>
  );
}

interface CommentTarget {
  index: number;
  selected: boolean;
  onComment: (index: number, extend: boolean) => void;
}

function CommentButton({ index, onComment, line }: CommentTarget & { line: DiffLine }) {
  const label = line.kind === "removed" ? `old line ${line.oldNumber}` : `line ${line.newNumber}`;
  return (
    <button
      type="button"
      aria-label={`Comment on ${label}`}
      title="Comment · Shift-click to select a range"
      onMouseDown={(e: MouseEvent) => e.shiftKey && e.preventDefault()}
      onClick={(e) => onComment(index, e.shiftKey)}
      className="diff-comment-button w-4 shrink-0 flex items-center justify-center text-accent cursor-pointer"
    >
      <PlusIcon size={10} />
    </button>
  );
}

function UnifiedRow({
  line,
  analysis,
  target,
}: {
  line: DiffLine;
  analysis: TextDiff["analysis"];
  target: CommentTarget;
}) {
  return (
    <div
      className={`diff-row flex ${lineClass(line)} ${target.selected ? "diff-selected" : ""}`}
      title={lineTitle(line)}
    >
      <CommentButton {...target} line={line} />
      <LineNumber value={line.oldNumber} />
      <LineNumber value={line.newNumber} />
      <span className="w-4 shrink-0 text-center text-text-muted select-none">
        {SIGN[line.kind]}
      </span>
      <span className="pr-4">
        <Code line={line} analysis={analysis} />
      </span>
    </div>
  );
}

function splitRows(lines: DiffLine[]): [DiffLine | undefined, DiffLine | undefined][] {
  const rows: [DiffLine | undefined, DiffLine | undefined][] = [];
  let removed: DiffLine[] = [];
  let added: DiffLine[] = [];
  const flush = () => {
    let i = 0;
    let j = 0;
    while (i < removed.length || j < added.length) {
      if (removed[i]?.moved || (i < removed.length && j >= added.length)) {
        rows.push([removed[i++], undefined]);
      } else if (added[j]?.moved || i >= removed.length) {
        rows.push([undefined, added[j++]]);
      } else {
        rows.push([removed[i++], added[j++]]);
      }
    }
    removed = [];
    added = [];
  };
  for (const line of lines) {
    if (line.kind === "removed") {
      if (added.length) flush();
      removed.push(line);
    } else if (line.kind === "added") added.push(line);
    else {
      flush();
      rows.push([line, line]);
    }
  }
  flush();
  return rows;
}

function SplitCell({
  line,
  side,
  analysis,
  target,
}: {
  line: DiffLine | undefined;
  side: "old" | "new";
  analysis: TextDiff["analysis"];
  target?: CommentTarget;
}) {
  const number = side === "old" ? line?.oldNumber : line?.newNumber;
  return (
    <div
      className={`diff-row flex flex-1 min-w-0 ${lineClass(line)} ${target?.selected ? "diff-selected" : ""}`}
      title={line ? lineTitle(line) : undefined}
    >
      {line && target ? (
        <CommentButton {...target} line={line} />
      ) : (
        <span className="w-4 shrink-0" />
      )}
      <LineNumber value={number} />
      <span className="diff-wrap flex-1 min-w-0 pr-2">
        {line && <Code line={line} analysis={analysis} />}
      </span>
    </div>
  );
}

function Gap({ count, onExpand }: { count: number; onExpand: () => void }) {
  return (
    <button
      type="button"
      onClick={onExpand}
      className="diff-gap block w-full px-3 py-0.5 text-left text-text-muted hover:text-text-primary cursor-pointer"
    >
      ⋯ Show {count} unchanged {count === 1 ? "line" : "lines"}
    </button>
  );
}

function CommentForm({
  initial = "",
  submitLabel,
  onSubmit,
  onCancel,
}: {
  initial?: string;
  submitLabel: string;
  onSubmit: (text: string) => void;
  onCancel: () => void;
}) {
  const [text, setText] = useState(initial);
  const submit = () => text.trim() && onSubmit(text);
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      submit();
    } else if (e.key === "Escape") {
      e.preventDefault();
      onCancel();
    }
  };
  return (
    <div className="flex flex-col gap-1.5">
      <textarea
        autoFocus
        aria-label="Comment"
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={onKeyDown}
        placeholder="Leave a comment for the agent"
        rows={3}
        className="w-full resize-y rounded-md border border-border bg-bg-primary px-2 py-1.5 text-xs text-text-primary outline-none focus:border-accent"
      />
      <div className="flex justify-end gap-1.5">
        <Button variant="ghost" onClick={onCancel} className="h-7 px-2 text-xs">
          Cancel
        </Button>
        <Button onClick={submit} disabled={!text.trim()} className="h-7 px-2 text-xs">
          {submitLabel}
        </Button>
      </div>
    </div>
  );
}

const annotationClass =
  "sticky left-0 my-1 ml-6 max-w-xl whitespace-normal rounded-md border border-border bg-bg-secondary p-2 font-sans text-xs";

function ThreadCard({
  thread,
  onThreadsChange,
}: {
  thread: DiffThread;
  onThreadsChange: DiffThreadsUpdate;
}) {
  const [editing, setEditing] = useState<string | null>(null);
  const [replying, setReplying] = useState(false);
  return (
    <div role="group" aria-label={`Comments on ${thread.location}`} className={annotationClass}>
      <ul className="flex flex-col gap-2">
        {thread.comments.map((comment) => (
          <li key={comment.id}>
            {editing === comment.id ? (
              <CommentForm
                initial={comment.text}
                submitLabel="Save"
                onSubmit={(text) => {
                  onThreadsChange((threads) => editDiffComment(threads, comment.id, text));
                  setEditing(null);
                }}
                onCancel={() => setEditing(null)}
              />
            ) : (
              <div className="flex items-start gap-2">
                <p className="flex-1 min-w-0 whitespace-pre-wrap select-text text-text-primary">
                  {comment.text}
                </p>
                <button
                  type="button"
                  onClick={() => setEditing(comment.id)}
                  className="text-text-muted hover:text-text-primary cursor-pointer"
                >
                  Edit
                </button>
                <button
                  type="button"
                  onClick={() =>
                    onThreadsChange((threads) => removeDiffComment(threads, comment.id))
                  }
                  className="text-text-muted hover:text-danger cursor-pointer"
                >
                  Discard
                </button>
              </div>
            )}
          </li>
        ))}
      </ul>
      <div className="mt-2">
        {replying ? (
          <CommentForm
            submitLabel="Add"
            onSubmit={(text) => {
              onThreadsChange((threads) => replyToDiffThread(threads, thread.id, text));
              setReplying(false);
            }}
            onCancel={() => setReplying(false)}
          />
        ) : (
          <button
            type="button"
            onClick={() => setReplying(true)}
            className="text-text-muted hover:text-text-primary cursor-pointer"
          >
            Add to thread
          </button>
        )}
      </div>
    </div>
  );
}

function TextDiffBody({
  diff,
  layout,
  hideFormatting,
  diffKey,
  path,
  threads,
  onThreadsChange,
}: {
  diff: TextDiff;
  layout: DiffLayout;
  hideFormatting: boolean;
  diffKey: string;
  path: string;
  threads: DiffThread[];
  onThreadsChange: DiffThreadsUpdate;
}) {
  const [expanded, setExpanded] = useState<Set<number>>(new Set());
  const [draft, setDraft] = useState<(DiffRange & { anchor: number }) | null>(null);
  const blocks = useMemo(
    () => (diff.lines ? diffBlocks(diff.lines, hideFormatting) : []),
    [diff.lines, hideFormatting]
  );
  const indexOf = useMemo(
    () => new Map((diff.lines ?? []).map((line, index) => [lineId(line), index])),
    [diff.lines]
  );
  if (!diff.lines) {
    return <p className="p-4 text-xs text-text-muted">This diff is too large to display.</p>;
  }
  if (!blocks.some((block) => block.kind === "lines")) {
    return (
      <p className="p-4 text-xs text-text-muted">
        {hideFormatting ? "Only formatting changed." : "No line changes."}
      </p>
    );
  }
  const lines = diff.lines;
  const onComment = (index: number, extend: boolean) =>
    setDraft((current) =>
      extend && current
        ? {
            anchor: current.anchor,
            start: Math.min(current.anchor, index),
            end: Math.max(current.anchor, index),
          }
        : { anchor: index, start: index, end: index }
    );
  const target = (line: DiffLine | undefined): CommentTarget | undefined => {
    if (!line) return undefined;
    const index = indexOf.get(lineId(line))!;
    const selected = draft !== null && index >= draft.start && index <= draft.end;
    return { index, selected, onComment };
  };
  const annotations = (indexes: number[]) =>
    [...new Set(indexes)].map((index) => (
      <div key={`annotations-${index}`}>
        {threads
          .filter((thread) => thread.end === index)
          .map((thread) => (
            <ThreadCard key={thread.id} thread={thread} onThreadsChange={onThreadsChange} />
          ))}
        {draft?.end === index && (
          <div className={annotationClass}>
            <CommentForm
              submitLabel="Comment"
              onSubmit={(text) => {
                const anchor = diffAnchor(diffKey, path, lines, draft);
                onThreadsChange((current) => addDiffComment(current, anchor, text));
                setDraft(null);
              }}
              onCancel={() => setDraft(null)}
            />
          </div>
        )}
      </div>
    ));
  const renderLines = (blockLines: DiffLine[]) =>
    layout === "unified"
      ? blockLines.map((line) => {
          const lineTarget = target(line)!;
          return (
            <div key={lineTarget.index}>
              <UnifiedRow line={line} analysis={diff.analysis} target={lineTarget} />
              {annotations([lineTarget.index])}
            </div>
          );
        })
      : splitRows(blockLines).map(([left, right], index) => {
          const leftTarget = target(left);
          const rightTarget = target(right);
          return (
            <div key={index}>
              <div className="flex">
                <SplitCell line={left} side="old" analysis={diff.analysis} target={leftTarget} />
                <SplitCell line={right} side="new" analysis={diff.analysis} target={rightTarget} />
              </div>
              {annotations(
                [leftTarget?.index, rightTarget?.index].filter((value) => value !== undefined)
              )}
            </div>
          );
        });
  const threadEnds = new Set(threads.map((thread) => thread.end));
  const hasThread = (block: DiffBlock) =>
    block.lines.some((line) => threadEnds.has(indexOf.get(lineId(line))!));
  const blockKey = (block: DiffBlock) => {
    const first = block.lines[0];
    return first.newNumber ?? -(first.oldNumber ?? 0);
  };
  const moved = lines.some((line) => line.moved);
  const formatting = !hideFormatting && lines.some((line) => line.formatting);
  return (
    <div
      className={`diff-code font-mono text-xs leading-5 select-text ${
        layout === "unified" ? "inline-block min-w-full" : "block"
      }`}
    >
      {(moved || formatting) && (
        <div className="flex gap-3 px-3 py-1 font-sans text-[11px] text-text-muted">
          {moved && <span className="diff-moved px-1.5 rounded">Moved</span>}
          {formatting && <span className="diff-formatting px-1.5 rounded">Formatting only</span>}
        </div>
      )}
      {blocks.map((block) => {
        const key = blockKey(block);
        if (block.kind === "lines" || expanded.has(key) || hasThread(block)) {
          return <div key={key}>{renderLines(block.lines)}</div>;
        }
        return (
          <Gap
            key={key}
            count={block.lines.length}
            onExpand={() => setExpanded((current) => new Set(current).add(key))}
          />
        );
      })}
    </div>
  );
}

export function FileDiffView({
  taskId,
  agent,
  changes,
  file,
  layout,
  hideFormatting,
  threads,
  onThreadsChange,
}: {
  taskId: string;
  agent: AgentId;
  changes: ChangeSet;
  file: FileChange;
  layout: DiffLayout;
  hideFormatting: boolean;
  threads: DiffThread[];
  onThreadsChange: DiffThreadsUpdate;
}) {
  const [state, setState] = useState<{ key: string; loaded?: Loaded; error?: string } | null>(null);
  const key = diffKeyOf(changes, file);

  useEffect(() => {
    let cancelled = false;
    chatFileDiff(taskId, agent, changes, file)
      .then((diff) => loadDiff(file, diff))
      .then((loaded) => !cancelled && setState({ key, loaded }))
      .catch((e) => !cancelled && setState({ key, error: String(e) }));
    return () => {
      cancelled = true;
    };
  }, [taskId, agent, changes, file, key]);

  if (state?.key !== key) {
    return (
      <div className="flex items-center gap-2 p-4 text-xs text-text-muted">
        <SpinnerIcon size={12} /> Loading diff…
      </div>
    );
  }
  if (state.error) {
    return (
      <p role="alert" className="p-4 text-xs text-danger select-text">
        {state.error}
      </p>
    );
  }
  const loaded = state.loaded!;
  if (loaded.kind === "media") {
    return (
      <div className="flex flex-col @lg:flex-row gap-4 p-4">
        <MediaSide label="Before" content={loaded.before} />
        <MediaSide label="After" content={loaded.after} />
      </div>
    );
  }
  return (
    <TextDiffBody
      key={`${key}:${hideFormatting}`}
      diff={loaded.diff}
      layout={layout}
      hideFormatting={hideFormatting}
      diffKey={key}
      path={`${file.folder}/${file.path}`}
      threads={threads.filter((thread) => thread.diffKey === key)}
      onThreadsChange={onThreadsChange}
    />
  );
}
