import { useEffect, useState } from "react";
import { Button } from "../ui/Button";
import {
  chatResumeCancel,
  chatResumeSchedule,
  chatResumeScheduled,
  ScheduledResume,
} from "../../services/chat";
import { formatReset } from "../../services/usage";
import { AgentId } from "../../types";

/** Past the time a resume is due, the app takes it within its 30-second check. */
const RESUME_CHECK_DELAY_MS = 35_000;

interface UsageLimitBannerProps {
  taskId: string;
  agent: AgentId;
  agentLabel: string;
  tab?: string;
  folders: string[];
  /** When the usage limit that stopped the latest prompt renews, in unix seconds. */
  resetsAt: number | null;
  /** Changes when a message is sent, which ends a scheduled resume. */
  prompts: number;
  onSnooze?: (until: string) => Promise<void>;
}

export function UsageLimitBanner({
  taskId,
  agent,
  agentLabel,
  tab,
  folders,
  resetsAt,
  prompts,
  onSnooze,
}: UsageLimitBannerProps) {
  const [scheduled, setScheduled] = useState<ScheduledResume | null>(null);
  const [working, setWorking] = useState<"resume" | "snooze" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [checks, setChecks] = useState(0);

  useEffect(() => {
    let stale = false;
    chatResumeScheduled(taskId, agent, tab)
      .then((found) => {
        if (!stale) setScheduled(found);
      })
      .catch(() => undefined);
    return () => {
      stale = true;
    };
  }, [taskId, agent, tab, prompts, checks]);

  useEffect(() => {
    if (!scheduled) return;
    const due = Math.max(scheduled.resumeAt * 1000 - Date.now(), 0);
    const timer = setTimeout(() => setChecks((count) => count + 1), due + RESUME_CHECK_DELAY_MS);
    return () => clearTimeout(timer);
  }, [scheduled]);

  const perform = async (kind: "resume" | "snooze", action: () => Promise<void>) => {
    setWorking(kind);
    setError(null);
    try {
      await action();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setWorking(null);
    }
  };

  const now = Date.now();
  const renewsAt = resetsAt !== null && resetsAt * 1000 > now ? resetsAt : null;
  const displayReset = scheduled?.resumeAt ?? renewsAt;
  if (displayReset === null) return null;

  const updateResume = async () => {
    if (scheduled) {
      await chatResumeCancel(taskId, agent, tab);
      setScheduled(null);
    } else {
      setScheduled(await chatResumeSchedule(taskId, agent, folders, displayReset, tab));
    }
  };

  return (
    <div className="max-w-3xl mx-auto mb-2 flex flex-col gap-1.5 text-xs">
      <div className="flex flex-wrap items-center gap-3 rounded-md border border-border bg-bg-secondary px-3 py-2">
        <p role="status" className="basis-48 grow text-text-secondary select-text">
          {scheduled
            ? `${formatReset(scheduled.resumeAt, now, "Continues")}, when the usage limit renews.`
            : `${agentLabel} hit its usage limit. ${formatReset(displayReset, now)}.`}
        </p>
        <Button
          variant="secondary"
          className="h-6 px-2 text-xs"
          loading={working === "resume"}
          disabled={working !== null}
          title={
            scheduled ? undefined : "Sends “Continue” once the limit renews, even after a restart"
          }
          onClick={() => void perform("resume", updateResume)}
        >
          {scheduled ? "Cancel resume" : "Resume when it renews"}
        </Button>
        {onSnooze && renewsAt !== null && (
          <Button
            variant="secondary"
            className="h-6 px-2 text-xs"
            loading={working === "snooze"}
            disabled={working !== null}
            onClick={() =>
              void perform("snooze", async () => {
                const until = new Date(renewsAt * 1000);
                if (until.getTime() > Date.now()) await onSnooze(until.toISOString());
              })
            }
          >
            Snooze until it renews
          </Button>
        )}
      </div>
      {error && (
        <p role="alert" className="text-danger break-words select-text">
          {error}
        </p>
      )}
    </div>
  );
}
