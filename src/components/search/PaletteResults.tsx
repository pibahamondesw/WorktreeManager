import { Badge } from "../ui/Badge";
import { BranchIcon } from "../ui/Icons";
import { PaletteItem } from "../../search/searchPalette";
import { TaskIndicatorDot } from "../ui/TaskIndicatorDot";

const ROW_CLASS =
  "w-full text-left px-4 py-2.5 flex items-center gap-3 cursor-pointer transition-colors";

export function PaletteResults({
  results,
  activeIndex,
  emptyMessage,
  onHover,
  onActivate,
}: {
  results: PaletteItem[];
  activeIndex: number;
  emptyMessage: string;
  onHover: (index: number) => void;
  onActivate: (item: PaletteItem) => void;
}) {
  if (results.length === 0) {
    return <p className="text-sm text-text-muted text-center py-10">{emptyMessage}</p>;
  }

  let taskIndex = 0;

  return (
    <>
      {results.map((item, i) => {
        const headerLabel = itemHeader(item);
        const showHeader = i === 0 || itemHeader(results[i - 1]) !== headerLabel;
        return (
          <div key={item.id}>
            {showHeader && (
              <p className="px-4 pt-3 pb-1 text-[0.625rem] uppercase tracking-wide text-text-muted">
                {headerLabel}
              </p>
            )}
            {item.kind === "task" ? (
              <TaskRow
                item={item}
                index={taskIndex++}
                active={i === activeIndex}
                onHover={() => onHover(i)}
                onClick={() => onActivate(item)}
              />
            ) : (
              <CommandRow
                item={item}
                active={i === activeIndex}
                onHover={() => onHover(i)}
                onClick={() => onActivate(item)}
              />
            )}
          </div>
        );
      })}
    </>
  );
}

function itemHeader(item: PaletteItem): string {
  if (item.kind === "task") {
    const indicator = item.result.indicator;
    if (indicator === "input") return "Needs your input";
    if (indicator === "idle" || indicator === "working") return "Active sessions";
    return item.result.inCurrentWorkspace ? "This workspace" : "Other workspaces";
  }
  switch (item.command.group) {
    case "action":
      return "Actions";
    case "workspace":
      return "Workspaces";
    case "settings":
      return "Settings";
  }
}

function TaskRow({
  item,
  index,
  active,
  onHover,
  onClick,
}: {
  item: Extract<PaletteItem, { kind: "task" }>;
  index: number;
  active: boolean;
  onHover: () => void;
  onClick: () => void;
}) {
  const { result } = item;
  return (
    <button
      data-active={active ? "true" : undefined}
      onMouseMove={onHover}
      onClick={onClick}
      className={`${ROW_CLASS} ${active ? "bg-bg-hover" : "hover:bg-bg-hover/50"}`}
    >
      {index <= 9 && (
        <span className="text-xs font-mono text-text-muted/40 flex-shrink-0 w-4 text-right">
          {index}
        </span>
      )}
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2">
          {result.indicator && result.indicator !== "ended" && (
            <TaskIndicatorDot indicator={result.indicator} />
          )}
          {result.task.linearIssueIdentifier && (
            <span className="text-xs font-mono text-text-muted flex-shrink-0">
              {result.task.linearIssueIdentifier}
            </span>
          )}
          <span className="text-sm text-text-primary truncate">
            {result.task.linearIssueTitle || result.task.branchName}
          </span>
        </div>
        <div className="flex items-center gap-1.5 mt-0.5 text-xs text-text-muted min-w-0">
          <BranchIcon />
          <span className="font-mono truncate">{result.task.branchName}</span>
        </div>
      </div>
      {!result.inCurrentWorkspace && result.workspace && <Badge>{result.workspace.name}</Badge>}
    </button>
  );
}

function CommandRow({
  item,
  active,
  onHover,
  onClick,
}: {
  item: Extract<PaletteItem, { kind: "command" }>;
  active: boolean;
  onHover: () => void;
  onClick: () => void;
}) {
  return (
    <button
      data-active={active ? "true" : undefined}
      onMouseMove={onHover}
      onClick={onClick}
      className={`${ROW_CLASS} ${active ? "bg-bg-hover" : "hover:bg-bg-hover/50"}`}
    >
      <span className="flex-1 min-w-0 text-sm text-text-primary truncate">
        {item.command.label}
      </span>
      {item.command.shortcut && <Badge>{item.command.shortcut.label}</Badge>}
      {item.command.hint && <Badge>{item.command.hint}</Badge>}
    </button>
  );
}
