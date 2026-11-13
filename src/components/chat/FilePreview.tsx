import { useEffect, useMemo, useRef } from "react";
import { DiffLine, SourceAnalysis } from "../../services/semanticDiff";
import { SpinnerIcon } from "../ui/Icons";
import { Code, LineNumber, MediaSide } from "./FileDiffView";
import { LoadedFile } from "./previewFile";

function sourceLines(text: string): DiffLine[] {
  let start = 0;
  return text.split("\n").map((lineText, index) => {
    const line: DiffLine = {
      kind: "context",
      side: "new",
      start,
      text: lineText,
      newNumber: index + 1,
      changes: [],
      formatting: false,
      moved: false,
    };
    start += lineText.length + 1;
    return line;
  });
}

function SourceCode({
  text,
  analysis,
  line,
}: {
  text: string;
  analysis: SourceAnalysis;
  line?: number;
}) {
  const lines = useMemo(() => sourceLines(text), [text]);
  const targetRef = useRef<HTMLDivElement>(null);
  const analyses = useMemo(() => ({ old: analysis, new: analysis }), [analysis]);

  useEffect(() => {
    targetRef.current?.scrollIntoView({ block: "center" });
  }, [lines, line]);

  return (
    <div className="diff-code inline-block min-w-full py-2 font-mono text-xs leading-5 select-text">
      {lines.map((sourceLine) => {
        const isTarget = sourceLine.newNumber === line;
        return (
          <div
            key={sourceLine.start}
            ref={isTarget ? targetRef : undefined}
            className={`diff-row flex ${isTarget ? "diff-selected" : ""}`}
          >
            <LineNumber value={sourceLine.newNumber} />
            <span className="pl-2 pr-4">
              <Code line={sourceLine} analysis={analyses} />
            </span>
          </div>
        );
      })}
    </div>
  );
}

export function FilePreview({ file, line }: { file: LoadedFile; line?: number }) {
  if (file.kind === "loading") {
    return (
      <div className="flex items-center justify-center gap-2 p-8 text-xs text-text-muted">
        <SpinnerIcon size={12} /> Loading…
      </div>
    );
  }
  if (file.kind === "error") {
    return (
      <p role="alert" className="p-8 text-center text-sm text-danger select-text">
        {file.message}
      </p>
    );
  }
  if (file.content.kind !== "text" || !file.analysis) {
    return (
      <div className="flex p-4">
        <MediaSide label="File" content={file.content} />
      </div>
    );
  }
  return <SourceCode text={file.content.text} analysis={file.analysis} line={line} />;
}
