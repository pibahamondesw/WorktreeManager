import { useState } from "react";
import {
  AutomationSummary,
  StartedRun,
  pauseAutomation,
  resumeAutomation,
  startAutomationRun,
} from "../../services/automations";
import { Button } from "../ui/Button";

type Action = "run" | "test" | "toggle";

interface AutomationActionsProps {
  summary: AutomationSummary;
  onChanged: () => void;
}

function startedMessage({ run_id, status, skip_reason }: StartedRun, test: boolean) {
  const kind = test ? "Test run" : "Run";
  if (status === "skipped") return `${kind} skipped: ${skip_reason?.replace(/_/g, " ")}`;
  return `${kind} ${run_id} started`;
}

export function AutomationActions({ summary, onChanged }: AutomationActionsProps) {
  const { entry, state } = summary;
  const [busy, setBusy] = useState<Action | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const enabled = state?.enabled ?? false;
  const valid = entry.definition !== null;

  const act = async (action: Action, perform: () => Promise<string | null>) => {
    setBusy(action);
    setError(null);
    try {
      setMessage(await perform());
      onChanged();
    } catch (reason) {
      setMessage(null);
      setError(String(reason));
    } finally {
      setBusy(null);
    }
  };

  const start = (test: boolean) =>
    act(test ? "test" : "run", async () =>
      startedMessage(await startAutomationRun(entry.name, test), test)
    );

  const toggle = () =>
    act("toggle", async () => {
      if (enabled) await pauseAutomation(entry.name, "user");
      else await resumeAutomation(entry.name, "user");
      return null;
    });

  const neverEnabled = !state || state.pause_reason === "new";
  const toggleLabel = enabled ? "Pause" : neverEnabled ? "Enable" : "Resume";

  return (
    <div className="flex flex-col items-end gap-1">
      <div className="flex items-center gap-2">
        {entry.definition?.trigger.manual && (
          <Button
            variant="secondary"
            className="h-7 px-3 text-xs"
            loading={busy === "run"}
            disabled={busy !== null}
            onClick={() => void start(false)}
          >
            Run now
          </Button>
        )}
        <Button
          variant="secondary"
          className="h-7 px-3 text-xs"
          loading={busy === "test"}
          disabled={!valid || busy !== null}
          title="Runs without changing the automation's state; captures the permissions it asks for"
          onClick={() => void start(true)}
        >
          Test run
        </Button>
        <Button
          variant={enabled ? "secondary" : "primary"}
          className="h-7 px-3 text-xs"
          loading={busy === "toggle"}
          disabled={(!enabled && !valid) || busy !== null}
          onClick={() => void toggle()}
        >
          {toggleLabel}
        </Button>
      </div>
      {message && <p className="text-xs text-text-muted select-text">{message}</p>}
      {error && <p className="text-xs text-danger break-words select-text">{error}</p>}
    </div>
  );
}
