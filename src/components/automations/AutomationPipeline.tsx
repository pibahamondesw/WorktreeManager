import { KeyboardEvent, useEffect, useMemo, useRef, useState } from "react";
import { useAutomationRuns } from "../../hooks/useAutomations";
import {
  AutomationRun,
  PipelineStep,
  isRunActive,
  runResult,
  runStartedAt,
} from "../../services/automations";
import {
  EdgeKind,
  PipelineEdge,
  StepProgress,
  StepState,
  pipelineLayout,
  stepKindName,
  stepProgress,
  stepSummary,
} from "../../services/automationPipeline";
import { Badge } from "../ui/Badge";
import { AutomationStepDetail } from "./AutomationStepDetail";

const LIVE_REFRESH_MS = 2000;
const NODE_HEIGHT = 56;
const ROW_PITCH = NODE_HEIGHT + 24;
const NODE_WIDTH = 288;
const LANE_WIDTH = 14;
const ARROW_OFFSET = 6;

const STATE_BADGES: Record<StepState, "default" | "success" | "warning" | "danger" | "accent"> = {
  not_started: "default",
  pending: "default",
  running: "accent",
  waiting: "warning",
  succeeded: "success",
  failed: "danger",
  rejected: "warning",
  canceled: "default",
  skipped: "default",
};

const EDGE_CLASSES: Record<EdgeKind, string> = {
  next: "text-text-muted",
  on_fail: "text-danger",
  on_reject: "text-warning",
};

const EDGE_LABELS: Record<EdgeKind, string> = {
  next: "next",
  on_fail: "on fail",
  on_reject: "on reject",
};

const humanize = (value: string) => value.replace(/_/g, " ");

export function AutomationPipeline({ name, steps }: { name: string; steps: PipelineStep[] }) {
  const { runs, error, loading, reload } = useAutomationRuns(name);
  const [runId, setRunId] = useState<string | null>(null);
  const [itemId, setItemId] = useState<string | null>(null);

  const run =
    runs.find((candidate) => candidate.id === runId) ??
    runs.find((candidate) => candidate.items.length > 0) ??
    runs[0] ??
    null;
  const item = run?.items.find((candidate) => candidate.id === itemId) ?? run?.items[0] ?? null;
  const live = run !== null && isRunActive(run);

  useEffect(() => {
    if (!live) return;
    const interval = window.setInterval(reload, LIVE_REFRESH_MS);
    return () => window.clearInterval(interval);
  }, [live, reload]);

  const progress = useMemo(() => stepProgress(steps, item), [steps, item]);

  return (
    <div className="space-y-4">
      {error && <p className="text-sm text-danger break-words select-text">{error}</p>}
      <div className="flex flex-wrap items-center gap-3 text-xs text-text-muted">
        {runs.length === 0 ? (
          <span>{loading ? "Loading runs…" : "No runs yet: showing the definition"}</span>
        ) : (
          <RunPicker runs={runs} run={run} onSelect={setRunId} />
        )}
        {run && run.items.length > 1 && (
          <select
            aria-label="Item"
            value={item?.id}
            onChange={(event) => setItemId(event.target.value)}
            className="bg-bg-tertiary border border-border rounded-md px-2 py-1 text-xs text-text-primary cursor-pointer"
          >
            {run.items.map((candidate) => (
              <option key={candidate.id} value={candidate.id}>
                {candidate.title ?? candidate.id} · {humanize(candidate.status)}
              </option>
            ))}
          </select>
        )}
        {live && <span className="text-success">Live</span>}
      </div>
      <PipelineGraph steps={steps} progress={progress} />
    </div>
  );
}

interface RunPickerProps {
  runs: AutomationRun[];
  run: AutomationRun | null;
  onSelect: (id: string) => void;
}

function RunPicker({ runs, run, onSelect }: RunPickerProps) {
  return (
    <select
      aria-label="Run"
      value={run?.id}
      onChange={(event) => onSelect(event.target.value)}
      className="bg-bg-tertiary border border-border rounded-md px-2 py-1 text-xs text-text-primary cursor-pointer"
    >
      {runs.map((candidate) => (
        <option key={candidate.id} value={candidate.id}>
          {runStartedAt(candidate).toLocaleString()} · {runResult(candidate)}
        </option>
      ))}
    </select>
  );
}

interface PipelineGraphProps {
  steps: PipelineStep[];
  progress: Map<string, StepProgress>;
}

function PipelineGraph({ steps, progress }: PipelineGraphProps) {
  const layout = useMemo(() => pipelineLayout(steps), [steps]);
  const [focusIndex, setFocusIndex] = useState(0);
  const [openStepId, setOpenStepId] = useState<string | null>(null);
  const listRef = useRef<HTMLOListElement>(null);
  const openStep = steps.find((step) => step.id === openStepId);

  const focusStep = (index: number) => {
    setFocusIndex(index);
    listRef.current?.querySelectorAll<HTMLElement>("button")[index]?.focus();
  };

  const navigate = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    const target =
      event.key === "ArrowDown"
        ? index + 1
        : event.key === "ArrowUp"
          ? index - 1
          : event.key === "Home"
            ? 0
            : event.key === "End"
              ? steps.length - 1
              : null;
    if (target === null || target < 0 || target >= steps.length) return;
    event.preventDefault();
    focusStep(target);
  };

  const toggle = (index: number) => {
    setFocusIndex(index);
    const { id } = steps[index];
    setOpenStepId((current) => (current === id ? null : id));
  };

  return (
    <div className="space-y-4">
      <div className="relative" style={{ width: NODE_WIDTH + layout.lanes * LANE_WIDTH + 12 }}>
        <ol ref={listRef} aria-label="Pipeline steps" className="space-y-6">
          {steps.map((step, index) => (
            <li key={step.id} className="relative">
              <StepNode
                step={step}
                progress={progress.get(step.id)}
                open={step.id === openStepId}
                focusable={index === focusIndex}
                onOpen={() => toggle(index)}
                onKeyDown={(event) => navigate(event, index)}
              />
              {index < steps.length - 1 &&
                (step.next === null || step.next === steps[index + 1].id) && (
                  <span
                    aria-hidden="true"
                    className="absolute left-6 top-full h-6 w-px bg-border"
                  />
                )}
            </li>
          ))}
        </ol>
        <PipelineJumps edges={layout.edges} steps={steps} lanes={layout.lanes} />
      </div>
      {openStep && <AutomationStepDetail step={openStep} progress={progress.get(openStep.id)} />}
    </div>
  );
}

interface StepNodeProps {
  step: PipelineStep;
  progress: StepProgress | undefined;
  open: boolean;
  focusable: boolean;
  onOpen: () => void;
  onKeyDown: (event: KeyboardEvent<HTMLButtonElement>) => void;
}

function StepNode({ step, progress, open, focusable, onOpen, onKeyDown }: StepNodeProps) {
  const state = progress?.state ?? "not_started";
  const attempts = progress?.attempts ?? 0;
  return (
    <button
      type="button"
      aria-pressed={open}
      aria-current={progress?.current ? "step" : undefined}
      tabIndex={focusable ? 0 : -1}
      onClick={onOpen}
      onKeyDown={onKeyDown}
      style={{ width: NODE_WIDTH, height: NODE_HEIGHT }}
      className={`flex items-center gap-3 rounded-lg border px-3 text-left cursor-pointer transition-colors outline-none focus-visible:ring-2 focus-visible:ring-accent ${
        open ? "bg-bg-hover" : "bg-bg-tertiary hover:bg-bg-hover"
      } ${progress?.current ? "border-accent" : "border-border"} ${
        state === "not_started" ? "border-dashed" : ""
      }`}
    >
      <span className="min-w-0 flex-1">
        <span className="flex items-baseline gap-2 text-xs">
          <span className="font-medium text-text-primary truncate">{step.label}</span>
          <span className="text-text-muted">{stepKindName(step)}</span>
        </span>
        <span className="block text-xs text-text-muted font-mono truncate">
          {stepSummary(step)}
        </span>
      </span>
      <span className="flex flex-col items-end gap-0.5 flex-shrink-0">
        <Badge variant={STATE_BADGES[state]}>{humanize(state)}</Badge>
        {attempts > 1 && <span className="text-[10px] text-text-muted">×{attempts}</span>}
      </span>
    </button>
  );
}

interface PipelineJumpsProps {
  edges: PipelineEdge[];
  steps: PipelineStep[];
  lanes: number;
}

function PipelineJumps({ edges, steps, lanes }: PipelineJumpsProps) {
  if (edges.length === 0) return null;
  const center = (index: number) => index * ROW_PITCH + NODE_HEIGHT / 2;
  return (
    <svg
      aria-hidden="true"
      className="absolute top-0 pointer-events-none overflow-visible"
      style={{ left: NODE_WIDTH }}
      width={lanes * LANE_WIDTH + 12}
      height={steps.length * ROW_PITCH - (ROW_PITCH - NODE_HEIGHT)}
    >
      {edges.map((edge) => {
        const x = (edge.lane + 1) * LANE_WIDTH;
        const start = center(edge.from) - ARROW_OFFSET;
        const end = center(edge.to) + ARROW_OFFSET;
        const label = `${steps[edge.from].id} → ${steps[edge.to].id} ${EDGE_LABELS[edge.kind]}${
          edge.limit ? ` (${edge.limit})` : ""
        }`;
        return (
          <g
            key={`${edge.from}-${edge.kind}`}
            data-edge={label}
            className={EDGE_CLASSES[edge.kind]}
            fill="none"
            stroke="currentColor"
            strokeWidth={1.25}
          >
            <title>{label}</title>
            <path
              d={`M0 ${start} H${x} V${end} H2`}
              strokeDasharray={edge.kind === "next" ? undefined : "4 3"}
            />
            <path d={`M7 ${end - 4} L2 ${end} L7 ${end + 4}`} />
          </g>
        );
      })}
    </svg>
  );
}
