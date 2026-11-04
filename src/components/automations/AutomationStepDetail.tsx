import { ReactNode, useState } from "react";
import { Button } from "../ui/Button";
import { PipelineStep, StepAttempt, StepEffect } from "../../services/automations";
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
  progress: StepProgress | undefined;
  /** Opens the latest attempt's conversation in its task; false when the task is gone. */
  onOpenConversation?: () => boolean;
}

export function AutomationStepDetail({
  step,
  progress,
  onOpenConversation,
}: AutomationStepDetailProps) {
  const [openError, setOpenError] = useState<string | null>(null);
  const rejectTransition = onReject(step);
  const facts: Fact[] = [
    ...kindFacts(step, progress?.latest ?? null),
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
    </section>
  );
}

function kindFacts({ kind }: PipelineStep, latest: StepAttempt | null): Fact[] {
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
      ["Repository", kind.command.repo],
      ["Log and receipt", NOT_RECORDED],
    ];
  }
  if ("wait" in kind) return [["Condition", waitConditionLabel(kind.wait)]];
  if ("approval" in kind) {
    return [
      ["Title", kind.approval.title],
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
