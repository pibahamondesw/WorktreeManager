import { KeyboardEvent, useEffect, useMemo, useRef, useState } from "react";
import { useAutomationItems } from "../../hooks/useAutomations";
import { AutomationSummary, PipelineStep, formatDuration } from "../../services/automations";
import { BoardCard, BoardColumn, boardColumns } from "../../services/automationBoard";
import { Badge } from "../ui/Badge";
import { Button } from "../ui/Button";
import { useOpenConversation } from "./openConversation";
import { useAutomationTasks } from "./automationTasks";
import { AutomationOperator } from "./AutomationOperator";
import { AutomationItemControls } from "./AutomationItemControls";

const LIVE_REFRESH_MS = 2000;

const MOVES: Record<string, [number, number]> = {
  ArrowLeft: [-1, 0],
  ArrowRight: [1, 0],
  ArrowUp: [0, -1],
  ArrowDown: [0, 1],
};

interface AutomationItemsProps {
  summary: AutomationSummary;
  steps: PipelineStep[];
  onChanged: () => void;
}

export function AutomationItems({ summary, steps, onChanged }: AutomationItemsProps) {
  const { items, error, loading, reload } = useAutomationItems(summary.entry.name);
  const tasks = useAutomationTasks();
  const [now, setNow] = useState(() => Date.now());
  const live = items.some((item) => item.status === "active");

  useEffect(() => {
    if (!live) return;
    const interval = window.setInterval(() => {
      setNow(Date.now());
      reload();
    }, LIVE_REFRESH_MS);
    return () => window.clearInterval(interval);
  }, [live, reload]);

  const columns = useMemo(() => boardColumns(steps, items, tasks, now), [steps, items, tasks, now]);

  return (
    <div className="space-y-4">
      <AutomationOperator
        summary={summary}
        onChanged={() => {
          reload();
          onChanged();
        }}
      />
      {error && <p className="text-sm text-danger break-words select-text">{error}</p>}
      {items.length === 0 && (
        <p className="text-xs text-text-muted">{loading ? "Loading items…" : "No items yet"}</p>
      )}
      <ItemBoard columns={columns} name={summary.entry.name} steps={steps} onChanged={reload} />
    </div>
  );
}

interface ItemBoardProps {
  columns: BoardColumn[];
  name: string;
  steps: PipelineStep[];
  onChanged: () => void;
}

function ItemBoard({ columns, name, steps, onChanged }: ItemBoardProps) {
  const [focus, setFocus] = useState<[number, number]>([0, 0]);
  const boardRef = useRef<HTMLDivElement>(null);
  const firstFilled = Math.max(
    0,
    columns.findIndex((column) => column.cards.length > 0)
  );
  const [column, row] = columns[focus[0]]?.cards[focus[1]] ? focus : [firstFilled, 0];

  const move = (event: KeyboardEvent<HTMLElement>) => {
    const delta = MOVES[event.key];
    if (!delta || event.target !== event.currentTarget) return;
    event.preventDefault();
    const next = neighbour(columns, [column, row], delta);
    if (!next) return;
    setFocus(next);
    boardRef.current?.querySelector<HTMLElement>(`[data-card="${next[0]}:${next[1]}"]`)?.focus();
  };

  return (
    <div ref={boardRef} className="flex gap-3 overflow-x-auto pb-2" aria-label="Items by phase">
      {columns.map((boardColumn, columnIndex) => (
        <section
          key={boardColumn.id}
          aria-label={boardColumn.label}
          className="w-64 shrink-0 rounded-md bg-bg-tertiary/40 p-2 space-y-2"
        >
          <h4 className="flex items-center justify-between px-1 text-xs font-medium text-text-secondary">
            <span>{boardColumn.label}</span>
            <span className="text-text-muted">{boardColumn.cards.length}</span>
          </h4>
          {boardColumn.cards.map((card, rowIndex) => (
            <ItemCard
              key={card.key}
              card={card}
              name={name}
              steps={steps}
              onChanged={onChanged}
              position={`${columnIndex}:${rowIndex}`}
              focusable={columnIndex === column && rowIndex === row}
              onKeyDown={move}
              onFocus={() => setFocus([columnIndex, rowIndex])}
            />
          ))}
        </section>
      ))}
    </div>
  );
}

/** The nearest card in the direction of `delta`; across columns, the same row or the last one. */
function neighbour(
  columns: BoardColumn[],
  [column, row]: [number, number],
  [across, down]: [number, number]
): [number, number] | null {
  if (down !== 0) {
    const target = row + down;
    return target >= 0 && target < columns[column].cards.length ? [column, target] : null;
  }
  for (let index = column + across; index >= 0 && index < columns.length; index += across) {
    const cards = columns[index].cards.length;
    if (cards > 0) return [index, Math.min(row, cards - 1)];
  }
  return null;
}

interface ItemCardProps {
  card: BoardCard;
  name: string;
  steps: PipelineStep[];
  onChanged: () => void;
  position: string;
  focusable: boolean;
  onKeyDown: (event: KeyboardEvent<HTMLElement>) => void;
  onFocus: () => void;
}

function ItemCard({
  card,
  name,
  steps,
  onChanged,
  position,
  focusable,
  onKeyDown,
  onFocus,
}: ItemCardProps) {
  const openConversation = useOpenConversation();
  const { item, taskRemoved } = card;
  const target = taskRemoved ? null : card.target;
  const opensConversation = !!target?.agent && !!target.conversationId;

  const open = () => {
    if (target) openConversation?.(target);
  };

  return (
    <article
      data-card={position}
      tabIndex={focusable ? 0 : -1}
      aria-label={item.title ?? item.id}
      onFocus={(event) => event.target === event.currentTarget && onFocus()}
      onKeyDown={(event) => {
        if (event.key === "Enter" && event.target === event.currentTarget && target) {
          event.preventDefault();
          open();
          return;
        }
        onKeyDown(event);
      }}
      className="rounded-md border border-border bg-bg-secondary p-2 space-y-1 text-xs outline-none focus-visible:border-accent"
    >
      <div className="flex items-start gap-2">
        <p className="flex-1 min-w-0 font-medium text-text-primary break-words select-text">
          {item.title ?? item.id}
        </p>
        {item.test && <Badge variant="accent">Test</Badge>}
      </div>
      <dl className="grid grid-cols-[auto_1fr] gap-x-2 gap-y-0.5 text-text-secondary select-text">
        <Fact label="Item" value={`${item.id} @ ${item.version}`} mono />
        <Fact label="Issue" value={card.issue} />
        <Fact label="PR" value={card.pr} />
        <Fact label="Branch" value={card.branch} mono />
        <Fact label="Active" value={formatDuration(card.activeMs)} />
        {card.repairRounds > 0 && <Fact label="Repairs" value={String(card.repairRounds)} />}
      </dl>
      {card.reason && (
        <p className="text-warning break-words select-text">{card.reason.replace(/_/g, " ")}</p>
      )}
      {card.evidence && (
        <pre className="max-h-32 overflow-y-auto whitespace-pre-wrap break-words rounded bg-bg-tertiary p-1.5 font-mono text-[11px] text-text-secondary select-text">
          {card.evidence}
        </pre>
      )}
      {target && (
        <Button
          variant="secondary"
          className="h-6 px-2 text-xs"
          tabIndex={focusable ? 0 : -1}
          onClick={open}
        >
          {opensConversation ? "Open conversation" : "Open task"}
        </Button>
      )}
      {!taskRemoved && (
        <AutomationItemControls
          source={{ name, runId: item.run_id, item: item.id }}
          item={item}
          run={null}
          steps={steps}
          onChanged={onChanged}
        />
      )}
      {taskRemoved && (
        <p
          className="text-text-muted"
          title="The item's task and its worktrees were deleted, so there is nothing left to open."
        >
          Task removed
        </p>
      )}
    </article>
  );
}

function Fact({ label, value, mono }: { label: string; value: string | null; mono?: boolean }) {
  if (!value) return null;
  return (
    <>
      <dt className="text-text-muted">{label}</dt>
      <dd className={`min-w-0 break-all ${mono ? "font-mono" : ""}`}>{value}</dd>
    </>
  );
}
