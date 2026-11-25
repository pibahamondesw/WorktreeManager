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
}

/** Offers to continue the chat once the usage limit renews, and shows when it will. */
export function UsageLimitBanner({
  taskId,
  agent,
  agentLabel,
  tab,
  folders,
  resetsAt,
  prompts,
}: UsageLimitBannerProps) {
  const [scheduled, setScheduled] = useState<ScheduledResume | null>(null);
  const [working, setWorking] = useState(false);
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

  const perform = async (action: () => Promise<ScheduledResume | null>) => {
    setWorking(true);
    setError(null);
    try {
      setScheduled(await action());
    } catch (reason) {
      setError(String(reason));
    } finally {
      setWorking(false);
    }
  };

  const renewsAt = resetsAt !== null && resetsAt * 1000 > Date.now() ? resetsAt : null;
  if (scheduled) {
    return (
      <Banner
        message={`${formatReset(scheduled.resumeAt, Date.now(), "Continues")}, when the usage limit renews.`}
        action="Cancel resume"
        working={working}
        error={error}
        onAction={() =>
          void perform(async () => {
            await chatResumeCancel(taskId, agent, tab);
            return null;
          })
        }
      />
    );
  }
  if (renewsAt === null) return null;
  return (
    <Banner
      message={`${agentLabel} hit its usage limit. ${formatReset(renewsAt)}.`}
      action="Resume when it renews"
      title="Sends “Continue” once the limit renews, even after a restart"
      working={working}
      error={error}
      onAction={() => void perform(() => chatResumeSchedule(taskId, agent, folders, renewsAt, tab))}
    />
  );
}

interface BannerProps {
  message: string;
  action: string;
  title?: string;
  working: boolean;
  error: string | null;
  onAction: () => void;
}

function Banner({ message, action, title, working, error, onAction }: BannerProps) {
  return (
    <div className="max-w-3xl mx-auto mb-2 flex flex-col gap-1.5 text-xs">
      <div className="flex items-center gap-3 rounded-md border border-border bg-bg-secondary px-3 py-2">
        <p role="status" className="flex-1 text-text-secondary select-text">
          {message}
        </p>
        <Button
          variant="secondary"
          className="h-6 px-2 text-xs"
          loading={working}
          disabled={working}
          title={title}
          onClick={onAction}
        >
          {action}
        </Button>
      </div>
      {error && (
        <p role="alert" className="text-danger break-words select-text">
          {error}
        </p>
      )}
    </div>
  );
}
