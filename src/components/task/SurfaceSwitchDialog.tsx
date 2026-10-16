import { useEffect, useState } from "react";
import { Modal } from "../ui/Modal";
import { Button } from "../ui/Button";
import { AgentActivity } from "../../services/agentActivity";
import { interruptedAgent, surfaceLabel, SurfaceSwitchChoice } from "../../embedded/surfaceSwitch";
import { SurfaceSwitchPrompt } from "../../hooks/useOpenTask";

interface SurfaceSwitchDialogProps {
  prompt: SurfaceSwitchPrompt | null;
  agentActivity?: AgentActivity;
  onChoose: (choice: SurfaceSwitchChoice) => void;
}

/** Focus moves to the dialog on open; give it back to where it was (e.g. the palette input). */
function useRestoreFocusOnUnmount() {
  const [previous] = useState(() => document.activeElement);
  useEffect(
    () => () => {
      if (previous instanceof HTMLElement && previous.isConnected) previous.focus();
    },
    [previous]
  );
}

/** Asks before opening a task in a new embedded surface would close the live one. */
export function SurfaceSwitchDialog({ prompt, ...props }: SurfaceSwitchDialogProps) {
  return prompt ? <SurfaceSwitchModal prompt={prompt} {...props} /> : null;
}

function SurfaceSwitchModal({
  prompt,
  agentActivity,
  onChoose,
}: SurfaceSwitchDialogProps & { prompt: SurfaceSwitchPrompt }) {
  useRestoreFocusOnUnmount();
  const current = surfaceLabel(prompt.current);
  const requested = surfaceLabel(prompt.requested);
  const workingAgent = interruptedAgent(prompt.closing, agentActivity);
  const closingInProgress = prompt.phase === "closing";
  return (
    <Modal open onClose={() => onChoose("cancel")} title={`${current} is already open`}>
      <div className="p-6 space-y-4 select-text">
        <p className="text-sm text-text-secondary">
          This task already has {current} open. Opening it in {requested} will close it.
        </p>
        {workingAgent && (
          <p role="alert" className="text-sm text-warning">
            {workingAgent} is working in this task; closing it will interrupt its turn. The
            conversation can be resumed later.
          </p>
        )}
        {prompt.error && (
          <div className="rounded-lg bg-danger/10 border border-danger/20 px-3 py-2">
            <p className="text-sm text-danger select-text whitespace-pre-wrap">{prompt.error}</p>
          </div>
        )}
        <div className="flex flex-col gap-2 pt-2">
          <Button
            autoFocus
            onClick={() => onChoose("keep")}
            disabled={closingInProgress}
            className="w-full justify-center"
          >
            Keep using {current}
          </Button>
          <Button
            variant={workingAgent ? "danger" : "secondary"}
            onClick={() => onChoose("replace")}
            loading={closingInProgress}
            className="w-full justify-center"
          >
            Close {current} and open {requested}
          </Button>
        </div>
      </div>
    </Modal>
  );
}
