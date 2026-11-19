import { KeyboardEvent, useEffect, useMemo, useRef, useState } from "react";
import { useAutomationRuns } from "../../hooks/useAutomations";
import {
  AttemptSource,
  AutomationRun,
  PipelineStep,
  TargetRepo,
  isRunActive,
  runResult,
  runStartedAt,
} from "../../services/automations";
import {
  StepProgress,
  StepState,
  pipelineLayout,
  stepKindName,
  stepProgress,
  stepSummary,
  transitionKey,
  traversedTransitions,
  unknownAttemptSteps,
} from "../../services/automationPipeline";
import { conversationTarget } from "../../services/automationInbox";
import { Badge } from "../ui/Badge";
import { Button } from "../ui/Button";
import { AutomationStepsEditor } from "./AutomationStepsEditor";
import { useOpenConversation } from "./openConversation";
import { AutomationStepDetail } from "./AutomationStepDetail";
import { AutomationPermissions } from "./AutomationPermissions";
import { AutomationItemControls } from "./AutomationItemControls";
import { AutomationRunCancel } from "./AutomationRunCancel";
import { JumpList, LANE_WIDTH, NODE_HEIGHT, NODE_WIDTH, PipelineJumps } from "./PipelineJumps";

const LIVE_REFRESH_MS = 2000;

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

const humanize = (value: string) => value.replace(/_/g, " ");

interface AutomationPipelineProps {
  name: string;
  steps: PipelineStep[];
  /** The definition's target repositories, to name a command step's repository. */
  repos?: TargetRepo[];
  /** Called after the editor saved the definition. */
  onChanged?: () => void;
}

export function AutomationPipeline({
  name,
  steps,
  repos = [],
  onChanged,
}: AutomationPipelineProps) {
  const [editing, setEditing] = useState(false);
  if (editing) {
    return (
      <AutomationStepsEditor
        name={name}
        onClose={() => setEditing(false)}
        onSaved={() => {
          setEditing(false);
          onChanged?.();
        }}
      />
    );
  }
  return <PipelineRuns name={name} steps={steps} repos={repos} onEdit={() => setEditing(true)} />;
}

interface PipelineRunsProps {
  name: string;
  steps: PipelineStep[];
  repos: TargetRepo[];
  onEdit: () => void;
}

function PipelineRuns({ name, steps, repos, onEdit }: PipelineRunsProps) {
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
  const traversed = useMemo(() => (item ? traversedTransitions(steps, item) : null), [steps, item]);
  const unknownSteps = useMemo(() => unknownAttemptSteps(steps, item), [steps, item]);

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
        {item?.defined && (
          <span className="select-text" title={`Version ${item.defined.version}`}>
            Item {item.defined.id}
            {item.defined.title ? ` · ${item.defined.title}` : ""}
          </span>
        )}
        {live && <span className="text-success">Live</span>}
        {run && live && (
          <AutomationRunCancel key={run.id} name={name} runId={run.id} onCanceled={reload} />
        )}
        <Button variant="secondary" className="ml-auto h-7 px-3 text-xs" onClick={onEdit}>
          Edit steps
        </Button>
      </div>
      {run && item && (
        <AutomationItemControls
          source={{ name, runId: run.id, item: item.id }}
          item={item}
          run={run}
          steps={steps}
          onChanged={reload}
        />
      )}
      <PipelineGraph
        steps={steps}
        progress={progress}
        traversed={traversed}
        run={run}
        repos={repos}
        source={run && item ? { name, runId: run.id, item: item.id } : undefined}
      />
      {unknownSteps.length > 0 && (
        <p className="text-xs text-text-muted select-text">
          Also ran steps no longer in the definition: {unknownSteps.join(", ")}
        </p>
      )}
      {run?.permission_requests?.length ? (
        <AutomationPermissions requests={run.permission_requests} />
      ) : null}
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
  /** Times each transition was taken by the selected item; null without one. */
  traversed: Map<string, number> | null;
  run: AutomationRun | null;
  repos: TargetRepo[];
  source: AttemptSource | undefined;
}

function PipelineGraph({ steps, progress, traversed, run, repos, source }: PipelineGraphProps) {
  const openConversation = useOpenConversation();
  const layout = useMemo(() => pipelineLayout(steps), [steps]);
  const [focusIndex, setFocusIndex] = useState(0);
  const [openStepId, setOpenStepId] = useState<string | null>(null);
  const listRef = useRef<HTMLOListElement>(null);
  const openStep = steps.find((step) => step.id === openStepId);
  const followedNext = (step: PipelineStep) =>
    traversed?.has(transitionKey({ from: step.id, kind: "next" })) ?? false;

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
                    data-taken={followedNext(step)}
                    className={`absolute left-6 top-full h-6 ${
                      followedNext(step) ? "w-0.5 bg-accent" : "w-px bg-border"
                    }`}
                  />
                )}
            </li>
          ))}
        </ol>
        <PipelineJumps
          edges={layout.edges}
          steps={steps}
          lanes={layout.lanes}
          traversed={traversed}
        />
      </div>
      <JumpList edges={layout.edges} steps={steps} traversed={traversed} />
      {openStep && (
        <AutomationStepDetail
          step={openStep}
          repos={repos}
          progress={progress.get(openStep.id)}
          source={source}
          onOpenConversation={conversationOpener(
            openConversation,
            steps,
            run,
            progress.get(openStep.id)
          )}
        />
      )}
    </div>
  );
}

function conversationOpener(
  openConversation: ReturnType<typeof useOpenConversation>,
  steps: PipelineStep[],
  run: AutomationRun | null,
  progress: StepProgress | undefined
) {
  const attempt = progress?.latest;
  const target = run && attempt?.session_id ? conversationTarget(steps, run, attempt) : null;
  return openConversation && target ? () => openConversation(target) : undefined;
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
          {stepSummary(step, progress?.latest ?? null)}
        </span>
      </span>
      <span className="flex flex-col items-end gap-0.5 flex-shrink-0">
        <Badge variant={STATE_BADGES[state]}>{humanize(state)}</Badge>
        {attempts > 1 && <span className="text-[10px] text-text-muted">×{attempts}</span>}
      </span>
    </button>
  );
}
