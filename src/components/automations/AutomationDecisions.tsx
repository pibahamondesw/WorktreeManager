import { useState } from "react";
import {
  AutomationSummary,
  decideForAutomation,
  provenance,
  revokeAutomationDecision,
} from "../../services/automations";
import { Button } from "../ui/Button";

interface AutomationDecisionsProps {
  summary: AutomationSummary;
  onChanged: () => void;
}

/** Standing rules every later agent step follows; turns in progress that can steer take new ones. */
export function AutomationDecisions({ summary, onChanged }: AutomationDecisionsProps) {
  const { entry, state } = summary;
  const decisions = state?.decisions ?? [];
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const text = draft.trim();

  const act = async (action: string, perform: () => Promise<string | null>) => {
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

  const add = () => {
    if (!text || busy !== null) return;
    void act("add", async () => {
      const { steered } = await decideForAutomation(entry.name, text);
      setDraft("");
      return steered > 0
        ? `Recorded; ${steered} turn${steered === 1 ? "" : "s"} in progress took it`
        : "Recorded; the next agent steps follow it";
    });
  };

  const revoke = (id: string) =>
    void act(id, async () => {
      await revokeAutomationDecision(entry.name, id);
      return null;
    });

  return (
    <div className="space-y-3 text-xs">
      {decisions.length === 0 ? (
        <p className="text-text-muted">No decisions yet</p>
      ) : (
        <ul aria-label="Decisions" className="space-y-2">
          {decisions.map((decision) => (
            <li
              key={decision.id}
              className="flex items-start gap-3 rounded-md border border-border p-3"
            >
              <div className="min-w-0 flex-1 select-text">
                <p className="whitespace-pre-wrap break-words text-text-primary">{decision.text}</p>
                <p className="text-text-muted">Recorded {provenance(decision)}</p>
              </div>
              <Button
                variant="ghost"
                className="h-6 px-2 text-xs"
                loading={busy === decision.id}
                disabled={busy !== null}
                title="Later agent steps no longer follow it"
                onClick={() => revoke(decision.id)}
              >
                Revoke
              </Button>
            </li>
          ))}
        </ul>
      )}
      <textarea
        aria-label="New decision"
        placeholder="A rule every later agent step follows, e.g. Never use define_method"
        value={draft}
        rows={2}
        maxLength={2000}
        disabled={busy !== null}
        onChange={(event) => setDraft(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
            event.preventDefault();
            add();
          }
        }}
        className="w-full rounded-lg border border-border bg-bg-tertiary px-3 py-2 text-xs text-text-primary placeholder:text-text-muted outline-none focus:border-accent"
      />
      <div className="flex items-center gap-2">
        <Button
          className="h-7 px-3 text-xs"
          loading={busy === "add"}
          disabled={!text || busy !== null}
          title="Cmd+Enter"
          onClick={add}
        >
          Record decision
        </Button>
        {message && <span className="text-text-muted select-text">{message}</span>}
      </div>
      {error && (
        <p role="alert" className="text-danger break-words select-text">
          {error}
        </p>
      )}
    </div>
  );
}
