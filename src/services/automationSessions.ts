import { invoke } from "@tauri-apps/api/core";
import {
  AutomationEntry,
  AutomationItem,
  AutomationRun,
  PipelineStep,
  StepAttempt,
  listAutomationRuns,
} from "./automations";
import { AutomationRuns } from "./automationInbox";

/** Runs per automation searched for a session; a live session belongs to one of the newest. */
const RECENT_RUNS = 10;

/** What an automation session works on: the newest attempt that used it. */
export interface AutomationSessionSource {
  automation: string;
  run: AutomationRun;
  item: AutomationItem;
  attempt: StepAttempt;
  steps: PipelineStep[];
}

export function sessionTitle(source: AutomationSessionSource): string {
  const step = source.steps.find((candidate) => candidate.id === source.attempt.step);
  return `${source.automation} · ${step?.label ?? source.attempt.step}`;
}

/** The source of each of `sessionIds` the runs mention; a later attempt replaces an earlier one. */
export function findSessionSources(
  automations: AutomationRuns[],
  sessionIds: string[]
): Record<string, AutomationSessionSource> {
  const wanted = new Set(sessionIds);
  const sources: Record<string, AutomationSessionSource> = {};
  for (const { entry, runs } of automations) {
    const steps = entry.definition?.steps ?? [];
    const oldestFirst = [...runs].sort((a, b) => (a.id < b.id ? -1 : 1));
    for (const run of oldestFirst)
      for (const item of run.items)
        for (const attempt of item.steps)
          if (attempt.session_id && wanted.has(attempt.session_id))
            sources[attempt.session_id] = { automation: entry.name, run, item, attempt, steps };
  }
  return sources;
}

/** Searches the newest runs of every automation; one whose runs fail to load is skipped. */
export async function loadSessionSources(
  sessionIds: string[]
): Promise<Record<string, AutomationSessionSource>> {
  const entries = await invoke<AutomationEntry[]>("automations_list");
  const pages = await Promise.allSettled(
    entries.map((entry) => listAutomationRuns(entry.name, null, RECENT_RUNS))
  );
  const automations = entries.flatMap((entry, index) => {
    const page = pages[index];
    return page.status === "fulfilled" ? [{ entry, runs: page.value.runs }] : [];
  });
  return findSessionSources(automations, sessionIds);
}
