import { AgentId } from "../types";
import {
  ApprovalDefault,
  AutomationEntry,
  AutomationItem,
  AutomationRun,
  PermissionRequest,
  PipelineStep,
  ReportPolicy,
  StepAttempt,
  humanize,
  isRunActive,
  runResult,
  runStartedAt,
} from "./automations";

export type InboxKind =
  | "findings"
  | "failed"
  | "needs_attention"
  | "noop"
  | "completed"
  | "question"
  | "permission"
  | "approval";

/** Where an entry leads: an agent conversation in the run's task. */
export interface ConversationTarget {
  taskId: string;
  /** Null when the definition no longer names the step's agent: the task opens as usual. */
  agent: AgentId | null;
  /** Null while its run is active, so the chat never resumes a conversation still in use. */
  conversationId: string | null;
}

export interface InboxEntry {
  key: string;
  automation: string;
  runId: string;
  kind: InboxKind;
  title: string;
  detail: string | null;
  at: number;
  test: boolean;
  /** Null when the run has no task. */
  target: ConversationTarget | null;
  /** A noop run under `archive_noop`: archived on arrival and never notified. */
  autoArchived: boolean;
  notify: boolean;
  /** Set on approval entries. */
  approval?: ApprovalRequest;
}

/** One attempt of an approval step, and how it was answered once it was. */
export interface ApprovalRequest {
  runId: string;
  item: string;
  step: string;
  attempt: number;
  /** The values the approver may edit, as the definition sets them. */
  params: Record<string, unknown>;
  timeoutAt: number | null;
  default: ApprovalDefault | null;
  /** Null while it waits for an answer. */
  answer: string | null;
}

/** Entries answered in the Inbox itself, so opening one shows it there rather than its task. */
export const opensInInbox = (item: InboxEntry) => item.kind === "approval";

/** Read and archived entry keys, kept in app data. */
export interface InboxMarks {
  read: string[];
  archived: string[];
}

export interface InboxItem extends InboxEntry {
  read: boolean;
  archived: boolean;
}

export interface AutomationRuns {
  entry: AutomationEntry;
  runs: AutomationRun[];
}

export const EMPTY_INBOX_MARKS: InboxMarks = { read: [], archived: [] };

const DEFAULT_REPORT: ReportPolicy = { inbox: "on_findings", archiveNoop: true, notify: true };
const MAX_MARKS = 1000;
const MAX_DETAIL = 280;

const reportPolicy = (entry: AutomationEntry) => entry.definition?.report ?? DEFAULT_REPORT;

/**
 * Everything that needs the human, newest first. An entry is keyed so a repeat (a question asked
 * again by a later run or attempt) keeps the first one and its read and archived state.
 */
export function inboxEntries(automations: AutomationRuns[]): InboxEntry[] {
  const byKey = new Map<string, InboxEntry>();
  for (const { entry, runs } of automations) {
    const policy = reportPolicy(entry);
    const oldestFirst = [...runs].sort((a, b) => (a.id < b.id ? -1 : 1));
    for (const run of oldestFirst) {
      const entries = [
        ...approvalEntries(entry, run, policy),
        ...(policy.inbox === "never" ? [] : runEntries(entry, run, policy)),
      ];
      for (const inboxEntry of entries) {
        if (!byKey.has(inboxEntry.key)) byKey.set(inboxEntry.key, inboxEntry);
      }
    }
  }
  return [...byKey.values()].sort((a, b) => b.at - a.at);
}

function runEntries(entry: AutomationEntry, run: AutomationRun, policy: ReportPolicy) {
  const outcome = outcomeEntry(entry, run, policy);
  const requests = (run.permission_requests ?? []).map((request, index) =>
    requestEntry(entry, run, policy, request, index)
  );
  return outcome ? [outcome, ...requests] : requests;
}

/** Approvals are the only way to answer from the app, so even `inbox: never` shows them. */
function approvalEntries(
  entry: AutomationEntry,
  run: AutomationRun,
  policy: ReportPolicy
): InboxEntry[] {
  const steps = entry.definition?.steps ?? [];
  return run.items.flatMap((item) =>
    item.steps.flatMap((attempt) => {
      const step = steps.find((candidate) => candidate.id === attempt.step);
      if (!step || !("approval" in step.kind) || !attempt.wait) return [];
      const { title, detail, params, default: fallback } = step.kind.approval;
      const answer = approvalAnswer(attempt);
      return [
        {
          ...runFacts(entry, run),
          key: `approval:${entry.name}:${item.id}:${attempt.step}:${run.id}:${attempt.attempt}`,
          kind: "approval" as const,
          title: attempt.wait.title ?? title,
          detail: answer ?? (detail ? clip(detail) : itemLabel(item)),
          target: conversationTarget(steps, run, attempt),
          autoArchived: false,
          notify: policy.notify,
          approval: {
            runId: run.id,
            item: item.id,
            step: attempt.step,
            attempt: attempt.attempt,
            params: params ?? {},
            timeoutAt: attempt.wait.timeout_at_ms ?? null,
            default: fallback ?? null,
            answer,
          },
        },
      ];
    })
  );
}

const itemLabel = (item: AutomationItem) => (item.title ? `Item ${item.title}` : null);

/** How an approval attempt was answered, or null while it waits. */
export function approvalAnswer(attempt: StepAttempt): string | null {
  if (attempt.status === "waiting" || attempt.status === "pending") return null;
  const answer = (attempt.result ?? attempt.signal ?? {}) as {
    decision?: unknown;
    comment?: unknown;
    default?: unknown;
  };
  if (answer.decision !== "approve" && answer.decision !== "reject")
    return attempt.cause ? humanize(attempt.cause) : humanize(attempt.status);
  const decided = answer.decision === "approve" ? "Approved" : "Rejected";
  const how = answer.default === true ? `${decided} by default after the timeout` : decided;
  return typeof answer.comment === "string" && answer.comment.trim()
    ? clip(`${how}: ${answer.comment}`)
    : how;
}

/** How a param shows in its input: strings as they are, other values as JSON. */
export const paramText = (value: unknown) =>
  typeof value === "string" ? value : JSON.stringify(value);

/** The params whose text changed, read back as their original type: strings stay text. */
export function editedParams(
  params: Record<string, unknown>,
  texts: Record<string, string>
): { params: Record<string, unknown> } | { error: string } {
  const edited: Record<string, unknown> = {};
  for (const [key, original] of Object.entries(params)) {
    const text = texts[key] ?? paramText(original);
    if (text === paramText(original)) continue;
    if (typeof original === "string") {
      edited[key] = text;
      continue;
    }
    try {
      edited[key] = JSON.parse(text);
    } catch {
      return { error: `${key} must be JSON, like ${paramText(original)}` };
    }
  }
  return { params: edited };
}

function outcomeKind(run: AutomationRun, { inbox, archiveNoop }: ReportPolicy): InboxKind | null {
  if (run.status === "failed") return "failed";
  if (run.status !== "completed") return null;
  if (run.outcome === "needs_attention") return "needs_attention";
  if (run.outcome === "noop") return archiveNoop || inbox === "always" ? "noop" : null;
  if (run.outcome === "reported") return inbox === "on_failure" ? null : "findings";
  return inbox === "always" ? "completed" : null;
}

function outcomeEntry(
  entry: AutomationEntry,
  run: AutomationRun,
  policy: ReportPolicy
): InboxEntry | null {
  const kind = outcomeKind(run, policy);
  if (!kind) return null;
  const attempts = run.items.flatMap((item) => item.steps);
  const autoArchived = kind === "noop" && policy.archiveNoop;
  return {
    ...runFacts(entry, run),
    key: `run:${entry.name}:${run.id}`,
    kind,
    title: outcomeTitle(kind, run),
    detail: outcomeDetail(run, attempts.at(-1)),
    target: conversationTarget(
      entry.definition?.steps ?? [],
      run,
      [...attempts].reverse().find((attempt) => attempt.session_id)
    ),
    autoArchived,
    notify: policy.notify && !autoArchived,
  };
}

function outcomeTitle(kind: InboxKind, run: AutomationRun): string {
  if (kind === "findings") return "Findings to review";
  if (kind === "needs_attention") return "Needs attention";
  if (kind === "noop") return "Nothing to report";
  return runResult(run);
}

function outcomeDetail(run: AutomationRun, last: StepAttempt | undefined): string | null {
  const resting = run.items.find((item) => item.cause);
  const text =
    run.task_error?.message ??
    resultSummary(last?.result) ??
    last?.reply ??
    resting?.cause ??
    (last?.cause ? `${last.step}: ${humanize(last.cause)}` : null);
  return text ? clip(text) : null;
}

function resultSummary(result: unknown): string | null {
  if (!result || typeof result !== "object") return null;
  const { summary, findings } = result as { summary?: unknown; findings?: unknown };
  const count = Array.isArray(findings) ? findings.length : 0;
  const found = count > 0 ? `${count} finding${count === 1 ? "" : "s"}` : null;
  const text = typeof summary === "string" && summary.trim() ? summary : null;
  return [found, text].filter(Boolean).join(": ") || null;
}

function requestEntry(
  entry: AutomationEntry,
  run: AutomationRun,
  policy: ReportPolicy,
  request: PermissionRequest,
  index: number
): InboxEntry {
  const question = request.kind === "question";
  const attempt = run.items
    .filter((item) => request.item === undefined || item.id === request.item)
    .flatMap((item) => item.steps)
    .find((step) => step.step === request.step && step.attempt === request.attempt);
  return {
    ...runFacts(entry, run),
    key: requestKey(entry.name, run, request, index),
    kind: question ? "question" : "permission",
    title: question
      ? (questionText(request.input) ?? request.tool)
      : `${request.resolution === "denied" ? "Denied" : "Allowed by default"}: ${request.tool}`,
    detail: question ? `Unanswered in ${request.step}` : inputDetail(request),
    target: conversationTarget(entry.definition?.steps ?? [], run, attempt),
    autoArchived: false,
    notify: policy.notify,
  };
}

/** A question is keyed by what it asks, so a later run or attempt never asks it again. */
function requestKey(
  automation: string,
  run: AutomationRun,
  request: PermissionRequest,
  index: number
): string {
  if (request.kind === "question") {
    const asked = stableHash(JSON.stringify(request.input ?? request.tool));
    return `question:${automation}:${request.item ?? ""}:${request.step}:${asked}`;
  }
  if (request.session_id && request.request_id)
    return `permission:${request.session_id}:${request.request_id}`;
  return `permission:${automation}:${run.id}:${index}`;
}

function questionText(input: unknown): string | null {
  if (!Array.isArray(input)) return null;
  const asked = input
    .map((question: { question?: unknown }) => question?.question)
    .filter((text): text is string => typeof text === "string" && text.trim() !== "");
  return asked.length > 0 ? clip(asked.join(" · ")) : null;
}

function inputDetail({ step, input }: PermissionRequest): string {
  if (input === null || input === undefined) return step;
  return clip(`${step}: ${typeof input === "string" ? input : JSON.stringify(input)}`);
}

function runFacts(entry: AutomationEntry, run: AutomationRun) {
  return {
    automation: entry.name,
    runId: run.id,
    at: run.ended_at_ms ?? runStartedAt(run).getTime(),
    test: run.trigger?.test ?? false,
  };
}

/** The conversation of an agent attempt in its run's task; null without a task. */
export function conversationTarget(
  steps: PipelineStep[],
  run: AutomationRun,
  attempt: StepAttempt | undefined
): ConversationTarget | null {
  if (!run.task_id) return null;
  const step = steps.find((candidate) => candidate.id === attempt?.step);
  const agent = step && "agent" in step.kind ? step.kind.agent.agent.id : null;
  const conversationId = isRunActive(run) ? null : (attempt?.conversation_id ?? null);
  return { taskId: run.task_id, agent, conversationId };
}

export function withMarks(entries: InboxEntry[], marks: InboxMarks): InboxItem[] {
  const read = new Set(marks.read);
  const archived = new Set(marks.archived);
  return entries.map((entry) => ({
    ...entry,
    read: read.has(entry.key),
    archived: entry.autoArchived || archived.has(entry.key),
  }));
}

/** Sets or clears a mark; the oldest marks go once there are too many to keep. */
export function mark(
  marks: InboxMarks,
  field: keyof InboxMarks,
  keys: string[],
  on: boolean
): InboxMarks {
  const remaining = marks[field].filter((key) => !keys.includes(key));
  const next = on ? [...remaining, ...keys] : remaining;
  return { ...marks, [field]: next.slice(-MAX_MARKS) };
}

const isPending = (item: InboxItem) => !item.read && !item.archived;

export const unreadCount = (items: InboxItem[]) => items.filter(isPending).length;

/** The Dock badge counts unread entries of automations that notify. */
export const badgeCount = (items: InboxItem[]) =>
  items.filter((item) => isPending(item) && item.notify).length;

/**
 * Unread entries to notify about, remembered in `known` by automation. The entries of an
 * automation loaded for the first time are only remembered, so its history never notifies.
 */
export function freshEntries(
  items: InboxItem[],
  loaded: string[],
  known: Map<string, Set<string>>
): InboxItem[] {
  const firstSeen = new Set(loaded.filter((automation) => !known.has(automation)));
  firstSeen.forEach((automation) => known.set(automation, new Set()));
  return items.filter((item) => {
    const keys = known.get(item.automation);
    if (!keys || keys.has(item.key)) return false;
    keys.add(item.key);
    return !firstSeen.has(item.automation) && isPending(item) && item.notify;
  });
}

function clip(text: string): string {
  const line = text.trim().replace(/\s+/g, " ");
  return line.length > MAX_DETAIL ? `${line.slice(0, MAX_DETAIL - 1)}…` : line;
}

function stableHash(text: string): string {
  let hash = 5381;
  for (let index = 0; index < text.length; index++) {
    hash = Math.imul(hash, 33) ^ text.charCodeAt(index);
  }
  return (hash >>> 0).toString(36);
}
