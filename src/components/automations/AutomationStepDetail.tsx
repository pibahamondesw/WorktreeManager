import { ReactNode } from "react";
import { PipelineStep, StepAttempt } from "../../services/automations";
import {
  StepProgress,
  onReject,
  stepKindName,
  transitionLabel,
  waitConditionLabel,
} from "../../services/automationPipeline";

const NOT_RECORDED = "Not recorded for this run yet";

type Fact = [label: string, value: ReactNode];

interface AutomationStepDetailProps {
  step: PipelineStep;
  progress: StepProgress | undefined;
}

export function AutomationStepDetail({ step, progress }: AutomationStepDetailProps) {
  const rejectTransition = onReject(step);
  const facts: Fact[] = [
    ...kindFacts(step),
    ...attemptFacts(progress),
    ["Next", step.next ?? "Following step"],
    ["On fail", transitionLabel(step.onFail)],
    ...(rejectTransition ? [["On reject", transitionLabel(rejectTransition)] as Fact] : []),
  ];
  return (
    <section
      aria-label={`Step ${step.label}`}
      className="rounded-lg border border-border bg-bg-tertiary/30 p-4 space-y-3"
    >
      <h4 className="text-xs font-semibold text-text-primary">
        {step.label} <span className="font-normal text-text-muted">{stepKindName(step)}</span>
      </h4>
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

function kindFacts({ kind }: PipelineStep): Fact[] {
  if ("agent" in kind) {
    const { agent, skill } = kind.agent;
    return [
      ["Agent", [agent.id, agent.model, agent.effort].filter(Boolean).join(" · ")],
      ...(skill ? [["Skill", skill] as Fact] : []),
      ["Session", NOT_RECORDED],
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
      ["Inbox entry", NOT_RECORDED],
    ];
  }
  return [["Details", NOT_RECORDED]];
}

const ATTEMPT_FIELDS: [label: string, field: keyof StepAttempt][] = [
  ["Process end", "process_end"],
  ["Exit code", "exit_code"],
  ["Outcome", "outcome"],
  ["Verdict", "verdict"],
  ["Stage", "stage"],
  ["Cause", "cause"],
  ["HEAD", "head"],
];

function attemptFacts(progress: StepProgress | undefined): Fact[] {
  const latest = progress?.latest;
  if (!latest) return [["Status", "Not started"]];
  return [
    ["Status", `${latest.status.replace(/_/g, " ")} · attempt ${latest.attempt}`],
    ...ATTEMPT_FIELDS.flatMap(([label, field]): Fact[] =>
      latest[field] === null ? [] : [[label, String(latest[field])]]
    ),
  ];
}
