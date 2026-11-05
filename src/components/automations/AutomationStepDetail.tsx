import { ReactNode, useEffect, useState } from "react";
import { Button } from "../ui/Button";
import {
  AttemptSource,
  PipelineStep,
  StepAttempt,
  StepEffect,
  StepReceipt,
  TargetRepo,
  formatDuration,
  readStepReceipt,
} from "../../services/automations";
import {
  StepProgress,
  onReject,
  stepKindName,
  transitionLabel,
  waitConditionLabel,
  waitFacts,
} from "../../services/automationPipeline";
import { approvalAnswer } from "../../services/automationInbox";

const NOT_RECORDED = "Not recorded for this run yet";

type Fact = [label: string, value: ReactNode];

interface AutomationStepDetailProps {
  step: PipelineStep;
  repos?: TargetRepo[];
  progress: StepProgress | undefined;
  /** Where the latest attempt ran, to read a command step's receipt and log. */
  source?: AttemptSource;
  /** Opens the latest attempt's conversation in its task; false when the task is gone. */
  onOpenConversation?: () => boolean;
}

export function AutomationStepDetail({
  step,
  repos = [],
  progress,
  source,
  onOpenConversation,
}: AutomationStepDetailProps) {
  const [openError, setOpenError] = useState<string | null>(null);
  const latest = progress?.latest ?? null;
  const receipt = useStepReceipt("command" in step.kind ? source : undefined, latest);
  const rejectTransition = onReject(step);
  const facts: Fact[] = [
    ...kindFacts(step, latest, repos),
    ...("command" in step.kind ? receiptFacts(receipt) : []),
    ...attemptFacts(progress),
    ...waitFacts(progress?.latest ?? null),
    ...effectFacts(progress?.latest?.effects ?? []),
    ["Next", step.next ?? "Following step"],
    ["On fail", transitionLabel(step.onFail)],
    ...(rejectTransition ? [["On reject", transitionLabel(rejectTransition)] as Fact] : []),
  ];
  return (
    <section
      aria-label={`Step ${step.label}`}
      className="rounded-lg border border-border bg-bg-tertiary/30 p-4 space-y-3"
    >
      <div className="flex items-center gap-3">
        <h4 className="text-xs font-semibold text-text-primary">
          {step.label} <span className="font-normal text-text-muted">{stepKindName(step)}</span>
        </h4>
        {onOpenConversation && (
          <Button
            variant="secondary"
            className="ml-auto h-7 px-2 text-xs"
            onClick={() =>
              setOpenError(onOpenConversation() ? null : "The task of this run no longer exists")
            }
          >
            Open conversation
          </Button>
        )}
      </div>
      {openError && (
        <p role="alert" className="text-xs text-danger select-text">
          {openError}
        </p>
      )}
      <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-xs select-text">
        {facts.map(([label, value]) => (
          <div key={label} className="contents">
            <dt className="text-text-muted">{label}</dt>
            <dd className="text-text-secondary break-words">{value}</dd>
          </div>
        ))}
      </dl>
      {"command" in step.kind && receipt.loaded && <CommandLog loaded={receipt.loaded} />}
    </section>
  );
}

function kindFacts(
  { kind }: PipelineStep,
  latest: StepAttempt | null,
  repos: TargetRepo[]
): Fact[] {
  if ("agent" in kind) {
    const { agent, skill } = kind.agent;
    return [
      ["Agent", [agent.id, agent.model, agent.effort].filter(Boolean).join(" · ")],
      ...(skill ? [["Skill", skill] as Fact] : []),
    ];
  }
  if ("command" in kind) {
    return [
      ["Command", <code className="font-mono">{kind.command.command.join(" ")}</code>],
      [
        "Repository",
        repos.find((repo) => repo.id === kind.command.repo)?.name ?? kind.command.repo,
      ],
    ];
  }
  if ("wait" in kind) return [["Condition", waitConditionLabel(kind.wait)]];
  if ("approval" in kind) {
    return [
      ["Title", latest?.wait?.title ?? kind.approval.title],
      ...(kind.approval.detail ? [["Detail", kind.approval.detail] as Fact] : []),
      ...(kind.approval.params && Object.keys(kind.approval.params).length > 0
        ? [["Params", JSON.stringify(kind.approval.params)] as Fact]
        : []),
      ["Inbox entry", approvalState(latest)],
    ];
  }
  return [["Effects", kind.publish.map((effect) => effect.effect).join(", ")]];
}

/** What each effect of the latest publish attempt did, or recorded in a Test run. */
function effectFacts(effects: StepEffect[]): Fact[] {
  return effects.map(
    ({ effect, status, detail, reference, warnings }, index): Fact => [
      `${index + 1}. ${effect}`,
      [status, detail, reference, ...warnings.map((warning) => `warning: ${warning}`)]
        .filter(Boolean)
        .join(" · "),
    ]
  );
}

const ATTEMPT_FIELDS: [label: string, field: keyof StepAttempt][] = [
  ["Process end", "process_end"],
  ["Exit code", "exit_code"],
  ["Outcome", "outcome"],
  ["Verdict", "verdict"],
  ["Stage", "stage"],
  ["Cause", "cause"],
  ["HEAD", "head"],
  ["Session", "session_id"],
  ["Conversation", "conversation_id"],
];

function attemptFacts(progress: StepProgress | undefined): Fact[] {
  const latest = progress?.latest;
  if (!latest) return [["Status", "Not started"]];
  return [
    ["Status", `${latest.status.replace(/_/g, " ")} · attempt ${latest.attempt}`],
    ...ATTEMPT_FIELDS.flatMap(([label, field]): Fact[] =>
      latest[field] == null ? [] : [[label, String(latest[field])]]
    ),
  ];
}

function approvalState(latest: StepAttempt | null): string {
  if (!latest?.wait) return NOT_RECORDED;
  return approvalAnswer(latest) ?? "Waiting for an answer in the Inbox";
}

interface ReceiptState {
  loaded: StepReceipt | null;
  error: string | null;
}

/** Reads the receipt of the latest attempt again whenever that attempt changes status. */
function useStepReceipt(source: AttemptSource | undefined, latest: StepAttempt | null) {
  const [state, setState] = useState<ReceiptState>({ loaded: null, error: null });
  const { name, runId, item } = source ?? {};
  const step = latest?.step;
  const attempt = latest?.attempt;
  const status = latest?.status;
  useEffect(() => {
    setState({ loaded: null, error: null });
    if (name === undefined || runId === undefined || item === undefined) return;
    if (step === undefined || attempt === undefined) return;
    let current = true;
    readStepReceipt({ name, runId, item }, step, attempt).then(
      (loaded) => {
        if (current) setState({ loaded, error: null });
      },
      (error) => {
        if (current) setState({ loaded: null, error: String(error) });
      }
    );
    return () => {
      current = false;
    };
  }, [name, runId, item, step, attempt, status]);
  return state;
}

function receiptFacts({ loaded, error }: ReceiptState): Fact[] {
  if (error) return [["Receipt", <span className="text-danger">{error}</span>]];
  if (!loaded) return [["Log and receipt", NOT_RECORDED]];
  const { receipt, reused } = loaded;
  const end =
    receipt.exit_code === null
      ? receipt.process_end.replace(/_/g, " ")
      : `exit ${receipt.exit_code}`;
  return [
    ["Receipt", `${end} · ${formatDuration(receipt.duration_ms)}`],
    ["Fingerprint", fingerprintLabel(receipt.fingerprint)],
    ["Reused", reused ? `Yes, from attempt ${receipt.attempt}` : "No"],
  ];
}

function fingerprintLabel(fingerprint: StepReceipt["receipt"]["fingerprint"]): string {
  if (!fingerprint) return "Not bound";
  const { branch, head, worktree } = fingerprint;
  return [branch, head?.slice(0, 7), worktree.slice(0, 8)].filter(Boolean).join(" · ");
}

function CommandLog({ loaded }: { loaded: StepReceipt }) {
  return (
    <div className="space-y-1">
      <p className="text-xs text-text-muted">{loaded.log_truncated ? "Log (last 64 KB)" : "Log"}</p>
      <pre
        aria-label="Command log"
        className="max-h-64 overflow-auto rounded bg-bg-tertiary p-1.5 font-mono text-xs text-text-secondary whitespace-pre-wrap break-words select-text"
      >
        {loaded.log || "No output"}
      </pre>
    </div>
  );
}
