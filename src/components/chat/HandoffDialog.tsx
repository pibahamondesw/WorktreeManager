import { Button } from "../ui/Button";
import { Modal } from "../ui/Modal";
import { useAgentCatalog } from "../../hooks/useAgentCatalog";
import { describeAgent } from "../../services/agents";
import { AgentId } from "../../types";

export type HandoffChoice = "continue" | "new";

interface HandoffDialogProps {
  from: AgentId;
  to: AgentId;
  onChoose: (choice: HandoffChoice) => void;
  onClose: () => void;
}

/** Asks how a conversation in progress moves to another agent, and what does not move with it. */
export function HandoffDialog({ from, to, onChoose, onClose }: HandoffDialogProps) {
  const catalog = useAgentCatalog();
  const source = describeAgent(from, catalog).label;
  const target = describeAgent(to, catalog).label;
  return (
    <Modal open onClose={onClose} title={`Switch to ${target}?`}>
      <div className="px-6 py-4 flex flex-col gap-3 text-sm text-text-secondary">
        <p>
          {target} can continue this conversation from a summary: your messages, {source}&apos;s
          replies, its plans and the tools it used. The earlier messages stay visible above.
        </p>
        <ul className="list-disc pl-5 flex flex-col gap-1 text-xs">
          <li>
            {source}&apos;s reasoning, raw tool output and approvals given for the session do not
            carry over.
          </li>
          <li>
            The first {target} turn reads the whole summary without a prompt cache, so it is slower
            and uses more of your plan.
          </li>
          <li>Messages from before the switch cannot be rewound.</li>
          <li>{source} stops now; its conversation stays in Conversations to resume it later.</li>
        </ul>
        <div className="flex justify-end gap-2 pt-2">
          <Button variant="ghost" className="h-8 text-xs" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="secondary" className="h-8 text-xs" onClick={() => onChoose("new")}>
            New conversation
          </Button>
          <Button autoFocus className="h-8 text-xs" onClick={() => onChoose("continue")}>
            Continue in {target}
          </Button>
        </div>
      </div>
    </Modal>
  );
}
