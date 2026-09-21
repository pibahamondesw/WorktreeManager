import { useState } from "react";
import { Task } from "../../types";
import { Modal } from "../ui/Modal";
import { Button } from "../ui/Button";

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
  const [issue, setIssue] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const close = () => {
    if (!saving) onClose();
  };

  return (
    <Modal open onClose={close} title="Link Linear issue">
      <form
        className="p-6 space-y-4"
        onSubmit={async (event) => {
          event.preventDefault();
          if (saving || !configured || !issue.trim()) return;
          setSaving(true);
          setError(null);
          try {
            await onLink(task.id, issue.trim());
            onClose();
          } catch (error) {
            setError(error instanceof Error ? error.message : "Could not link the issue.");
          } finally {
            setSaving(false);
          }
        }}
      >
        <p className="text-sm text-text-secondary select-text">{task.branchName}</p>
        {configured ? (
          <label className="flex flex-col gap-2 text-sm text-text-secondary">
            Linear issue ID
            <input
              autoFocus
              value={issue}
              onChange={(event) => setIssue(event.target.value)}
              placeholder="WOR-123"
              disabled={saving}
              className="rounded-lg border border-border bg-bg-tertiary px-3 py-2 text-text-primary outline-none focus:border-accent"
            />
          </label>
        ) : (
          <p className="text-sm text-text-secondary">Configure Linear in this workspace first.</p>
        )}
        {error && (
          <p role="alert" className="text-sm text-danger select-text">
            {error}
          </p>
        )}
        <div className="flex justify-end gap-3">
          <Button type="button" variant="ghost" onClick={close} disabled={saving}>
            Cancel
          </Button>
          <Button type="submit" loading={saving} disabled={!configured || !issue.trim()}>
            Link issue
          </Button>
        </div>
      </form>
    </Modal>
  );
}
