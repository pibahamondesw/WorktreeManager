import { useState } from "react";
import { cancelRun } from "../../services/automations";
import { Button } from "../ui/Button";

interface AutomationRunCancelProps {
  name: string;
  runId: string;
  onCanceled: () => void;
}

/** Cancels an active run after the owner confirms it. */
export function AutomationRunCancel({ name, runId, onCanceled }: AutomationRunCancelProps) {
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const cancel = async () => {
    setBusy(true);
    setError(null);
    try {
      await cancelRun(name, runId);
      setConfirming(false);
      onCanceled();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  if (!confirming)
    return (
      <Button
        variant="danger"
        className="h-7 px-3 text-xs"
        title="Stop this run: its open items end canceled"
        onClick={() => setConfirming(true)}
      >
        Cancel run
      </Button>
    );

  return (
    <div className="flex basis-full flex-wrap items-center gap-2" aria-label="Confirm cancel run">
      <span className="text-text-secondary">
        Cancel this run? Its open items end canceled and agent turns in progress stop; tasks, PRs
        and issues stay.
      </span>
      <Button
        variant="ghost"
        className="h-7 px-3 text-xs"
        disabled={busy}
        onClick={() => setConfirming(false)}
      >
        Keep running
      </Button>
      <Button
        variant="danger"
        className="h-7 px-3 text-xs"
        loading={busy}
        onClick={() => void cancel()}
      >
        Cancel run
      </Button>
      {error && <p className="basis-full text-danger break-words select-text">{error}</p>}
    </div>
  );
}
