import { useState } from "react";
import {
  AutomationSummary,
  Limit,
  nextAutomationItem,
  provenance,
  runAutomationContinuously,
  setAutomationLimit,
} from "../../services/automations";
import {
  EffectiveLimit,
  currentPace,
  effectiveLimits,
  paceLabel,
} from "../../services/automationBoard";
import { Button } from "../ui/Button";

type Busy = "next" | "continuous" | Limit;

interface AutomationOperatorProps {
  summary: AutomationSummary;
  onChanged: () => void;
}

/** Next, Continuous and the live limits of a campaign; Pause and Resume sit in the header. */
export function AutomationOperator({ summary, onChanged }: AutomationOperatorProps) {
  const { entry, state } = summary;
  const [busy, setBusy] = useState<Busy | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const pace = currentPace(state);
  const paceOverride = state?.overrides?.pace;

  const act = async (action: Busy, perform: () => Promise<string | null>) => {
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

  const next = () =>
    act("next", async () => {
      const { run } = await nextAutomationItem(entry.name, "user");
      if (!run) return "One more item granted for the next run";
      if (run.status === "skipped")
        return `One more item granted; run skipped: ${run.skip_reason?.replace(/_/g, " ")}`;
      return `One more item granted; run ${run.run_id} started`;
    });

  const continuous = () =>
    act("continuous", async () => {
      await runAutomationContinuously(entry.name, "user");
      return null;
    });

  const setLimit = (limit: Limit, value: number | null) =>
    act(limit, async () => {
      await setAutomationLimit(entry.name, "user", limit, value);
      return null;
    });

  return (
    <div className="rounded-md border border-border p-3 space-y-3 text-xs">
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-medium text-text-primary">{paceLabel(pace)}</span>
        {paceOverride && (
          <span className="text-text-muted select-text">{provenance(paceOverride)}</span>
        )}
        <div className="ml-auto flex gap-2">
          <Button
            variant="secondary"
            className="h-7 px-3 text-xs"
            loading={busy === "next"}
            disabled={busy !== null}
            title="Pending items keep going; one more item is taken"
            onClick={() => void next()}
          >
            Next
          </Button>
          <Button
            variant="secondary"
            className="h-7 px-3 text-xs"
            loading={busy === "continuous"}
            disabled={busy !== null || pace.mode === "continuous"}
            title="Take new items as capacity allows"
            onClick={() => void continuous()}
          >
            Continuous
          </Button>
        </div>
      </div>
      {entry.definition && (
        <div className="flex flex-wrap gap-4">
          {effectiveLimits(entry.definition, state).map((limit) => (
            <LimitField
              key={limit.limit}
              limit={limit}
              busy={busy === limit.limit}
              disabled={busy !== null}
              onSet={(value) => void setLimit(limit.limit, value)}
            />
          ))}
        </div>
      )}
      {message && <p className="text-text-muted select-text">{message}</p>}
      {error && <p className="text-danger break-words select-text">{error}</p>}
    </div>
  );
}

interface LimitFieldProps {
  limit: EffectiveLimit;
  busy: boolean;
  disabled: boolean;
  onSet: (value: number | null) => void;
}

function LimitField({ limit, busy, disabled, onSet }: LimitFieldProps) {
  const [draft, setDraft] = useState<string | null>(null);
  const shown = draft ?? (limit.value === null ? "" : String(limit.value));
  const parsed = Number(shown);
  const valid = Number.isInteger(parsed) && parsed >= 1 && parsed !== limit.value;

  const apply = () => {
    if (!valid) return;
    onSet(parsed);
    setDraft(null);
  };

  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-center gap-2">
        <label className="text-text-muted" htmlFor={`automation-limit-${limit.limit}`}>
          {limit.label}
        </label>
        <input
          id={`automation-limit-${limit.limit}`}
          type="number"
          min={1}
          value={shown}
          placeholder="free"
          disabled={disabled}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") apply();
            if (event.key === "Escape" && draft !== null) {
              event.stopPropagation();
              setDraft(null);
            }
          }}
          className="w-16 rounded-md border border-border bg-bg-tertiary px-2 py-1 text-text-primary outline-none focus:border-accent"
        />
        {valid && (
          <Button
            variant="secondary"
            className="h-6 px-2 text-xs"
            loading={busy}
            disabled={disabled}
            onClick={apply}
          >
            Set
          </Button>
        )}
        {limit.override && !valid && (
          <Button
            variant="ghost"
            className="h-6 px-2 text-xs"
            loading={busy}
            disabled={disabled}
            title="Go back to the definition's value"
            onClick={() => onSet(null)}
          >
            Reset
          </Button>
        )}
      </div>
      {limit.override && (
        <span className="text-text-muted select-text">Set {provenance(limit.override)}</span>
      )}
    </div>
  );
}
