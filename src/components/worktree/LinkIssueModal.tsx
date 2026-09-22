import { useState } from "react";
import { LinearIssue, Task } from "../../types";
import { Modal } from "../ui/Modal";
import { Button } from "../ui/Button";
import { LinearIssuePicker } from "./LinearIssuePicker";
import { Badge } from "../ui/Badge";

export function LinkIssueModal({
  task,
  configured,
  onLink,
  onClose,
}: {
  task: Task;
  configured: boolean;
  onLink: (taskId: string, issue: string) => Promise<Task>;
  onClose: () => void;
}) {
  const [selected, setSelected] = useState<LinearIssue | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const close = () => {
    if (!saving) onClose();
  };

  const link = async () => {
    if (saving || !selected) return;
    setSaving(true);
    setError(null);
    try {
      await onLink(task.id, selected.identifier);
      onClose();
    } catch (error) {
      setError(error instanceof Error ? error.message : "Could not link the issue.");
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal open onClose={close} title="Link Linear issue" wide={configured}>
      {configured ? (
        <div className="flex flex-col h-[60vh]">
          <LinearIssuePicker
            onSelect={(issue) => {
              setSelected(issue);
              setError(null);
            }}
          >
            {selected ? (
              <div className="p-6 space-y-4">
                <button
                  type="button"
                  disabled={saving}
                  onClick={() => {
                    setSelected(null);
                    setError(null);
                  }}
                  className="text-sm text-text-muted hover:text-text-primary cursor-pointer disabled:opacity-50"
                >
                  Back to results
                </button>
                <div className="bg-bg-tertiary rounded-lg p-4 space-y-3">
                  <p className="text-xs text-text-muted font-mono">{selected.identifier}</p>
                  <h3 className="text-sm font-medium text-text-primary">{selected.title}</h3>
                  {selected.stateName && <Badge>{selected.stateName}</Badge>}
                  {selected.projectName && (
                    <p className="text-xs text-text-secondary">Project: {selected.projectName}</p>
                  )}
                </div>
                <p className="text-sm text-text-secondary select-text">Task: {task.branchName}</p>
                {error && (
                  <p role="alert" className="text-sm text-danger select-text">
                    {error}
                  </p>
                )}
                <div className="flex justify-end gap-3">
                  <Button variant="ghost" onClick={close} disabled={saving}>
                    Cancel
                  </Button>
                  <Button onClick={link} loading={saving}>
                    Link issue
                  </Button>
                </div>
              </div>
            ) : undefined}
          </LinearIssuePicker>
        </div>
      ) : (
        <p className="p-6 text-sm text-text-secondary">Configure Linear in this workspace first.</p>
      )}
    </Modal>
  );
}
