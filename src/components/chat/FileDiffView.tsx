import { useEffect, useMemo, useState } from "react";
import { SpinnerIcon } from "../ui/Icons";
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

export type DiffLayout = "unified" | "split";

interface TextDiff {
  lines: DiffLine[] | null;
  analysis: Record<"old" | "new", SourceAnalysis>;
}

type Loaded =
  | { kind: "text"; diff: TextDiff }
  | { kind: "media"; before: FileContent | null; after: FileContent | null };

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

function UnifiedRow({ line, analysis }: { line: DiffLine; analysis: TextDiff["analysis"] }) {
  return (
    <div className={`flex ${lineClass(line)}`} title={lineTitle(line)}>
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
}: {
  line: DiffLine | undefined;
  side: "old" | "new";
  analysis: TextDiff["analysis"];
}) {
  return (
    <div
      className={`flex flex-1 min-w-0 ${lineClass(line)}`}
      title={line ? lineTitle(line) : undefined}
    >
      <LineNumber value={side === "old" ? line?.oldNumber : line?.newNumber} />
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

function TextDiffBody({
  diff,
  layout,
  hideFormatting,
}: {
  diff: TextDiff;
  layout: DiffLayout;
  hideFormatting: boolean;
}) {
  const [expanded, setExpanded] = useState<Set<number>>(new Set());
  const blocks = useMemo(
    () => (diff.lines ? diffBlocks(diff.lines, hideFormatting) : []),
    [diff.lines, hideFormatting]
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
  const renderLines = (lines: DiffLine[]) =>
    layout === "unified"
      ? lines.map((line, index) => <UnifiedRow key={index} line={line} analysis={diff.analysis} />)
      : splitRows(lines).map(([left, right], index) => (
          <div key={index} className="flex">
            <SplitCell line={left} side="old" analysis={diff.analysis} />
            <SplitCell line={right} side="new" analysis={diff.analysis} />
          </div>
        ));
  const blockKey = (block: DiffBlock) => {
    const first = block.lines[0];
    return first.newNumber ?? -(first.oldNumber ?? 0);
  };
  const moved = diff.lines.some((line) => line.moved);
  const formatting = !hideFormatting && diff.lines.some((line) => line.formatting);
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
        if (block.kind === "lines" || expanded.has(key)) {
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
}: {
  taskId: string;
  agent: AgentId;
  changes: ChangeSet;
  file: FileChange;
  layout: DiffLayout;
  hideFormatting: boolean;
}) {
  const [state, setState] = useState<{ key: string; loaded?: Loaded; error?: string } | null>(null);
  const key = `${changes.before}:${changes.after}:${file.folder}:${file.path}`;

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
    />
  );
}
