import { useState } from "react";
import {
  AttemptSource,
  AutomationItem,
  AutomationRun,
  ItemAction,
  ItemControl,
  PipelineStep,
  RetryDefinition,
  handBackItem,
  isTakenOver,
  itemActions,
  retryItem,
  takeOverItem,
} from "../../services/automations";
import { Badge } from "../ui/Badge";
import { Button } from "../ui/Button";
import { useOpenConversation } from "./openConversation";

interface AutomationItemControlsProps {
  source: AttemptSource;
  item: AutomationItem;
  /** The item's run, when known: a routine's failed run can leave its item open to retry. */
  run: AutomationRun | null;
  steps: PipelineStep[];
  onChanged: () => void;
}

/** Take over, hand back and retry for one item; renders nothing when none applies. */
export function AutomationItemControls({
  source,
  item,
  run,
  steps,
  onChanged,
}: AutomationItemControlsProps) {
  const openConversation = useOpenConversation();
  const [busy, setBusy] = useState<ItemAction | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState("");
  const [definition, setDefinition] = useState<RetryDefinition>("current");
  const actions = itemActions(item, steps, run);
  const takenOver = isTakenOver(item);
  if (actions.length === 0) return null;

  const act = async (action: ItemAction, perform: () => Promise<ItemControl>) => {
    setBusy(action);
    setError(null);
    try {
      const control = await perform();
      onChanged();
      return control;
    } catch (reason) {
      setError(String(reason));
      return null;
    } finally {
      setBusy(null);
    }
  };

  const takeOver = async () => {
    const control = await act("take_over", () => takeOverItem(source));
    if (control?.status !== "paused") {
      if (control) setError("The step is still stopping; it pauses once its turn ends");
      return;
    }
    const step = steps.find((candidate) => candidate.id === control.step);
    const agent = step && "agent" in step.kind ? step.kind.agent.agent.id : null;
    if (control.task_id)
      openConversation?.({
        taskId: control.task_id,
        agent,
        conversationId: control.conversation_id,
      });
  };

  const handBack = async () => {
    if (await act("hand_back", () => handBackItem(source, note.trim() || null))) setNote("");
  };

  return (
    <div className="space-y-2 text-xs" aria-label="Item controls">
      {takenOver && (
        <p className="flex items-center gap-2 text-text-secondary">
          <Badge variant="warning">Taken over</Badge>
          <span>The engine sends no prompts until you hand it back.</span>
        </p>
      )}
      <div className="flex flex-wrap items-center gap-2">
        {actions.includes("take_over") && (
          <Button
            variant="secondary"
            className="h-6 px-2 text-xs"
            loading={busy === "take_over"}
            disabled={busy !== null}
            title="Stop the engine on this step and continue its conversation yourself"
            onClick={() => void takeOver()}
          >
            Take over
          </Button>
        )}
        {actions.includes("hand_back") && (
          <>
            <input
              aria-label="Note for the next step"
              value={note}
              placeholder="Note for the next step (optional)"
              disabled={busy !== null}
              onChange={(event) => setNote(event.target.value)}
              onKeyDown={(event) => event.key === "Enter" && void handBack()}
              className="min-w-0 flex-1 rounded-md border border-border bg-bg-tertiary px-2 py-1 text-text-primary outline-none focus:border-accent"
            />
            <Button
              variant="secondary"
              className="h-6 px-2 text-xs"
              loading={busy === "hand_back"}
              disabled={busy !== null}
              title="The pipeline goes on from this step"
              onClick={() => void handBack()}
            >
              Hand back
            </Button>
          </>
        )}
        {actions.includes("retry") && (
          <>
            <select
              aria-label="Definition to retry with"
              value={definition}
              disabled={busy !== null}
              onChange={(event) => setDefinition(event.target.value as RetryDefinition)}
              className="bg-bg-tertiary border border-border rounded-md px-2 py-1 text-xs text-text-primary cursor-pointer"
            >
              <option value="current">Current definition</option>
              <option value="original">Run&apos;s original definition</option>
            </select>
            <Button
              variant="secondary"
              className="h-6 px-2 text-xs"
              loading={busy === "retry"}
              disabled={busy !== null}
              title="Run again from the step it stopped at; publish effects already done are not repeated"
              onClick={() => void act("retry", () => retryItem(source, definition))}
            >
              Retry
            </Button>
          </>
        )}
      </div>
      {error && <p className="text-danger break-words select-text">{error}</p>}
    </div>
  );
}
