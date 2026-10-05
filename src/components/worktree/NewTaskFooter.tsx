import { Button } from "../ui/Button";

interface NewTaskFooterProps {
  error: string | null;
  creating: boolean;
  status: string;
  canCreate: boolean;
  onClose: () => void;
  onCreate: () => void;
}

export function NewTaskFooter({
  error,
  creating,
  status,
  canCreate,
  onClose,
  onCreate,
}: NewTaskFooterProps) {
  return (
    <>
      {error && (
        <div className="rounded-lg bg-danger/10 border border-danger/20 px-3 py-2">
          <p className="text-sm text-danger select-text cursor-text whitespace-pre-wrap wrap-break-word">
            {error}
          </p>
        </div>
      )}
      <div className="flex items-center justify-between">
        {creating && status ? <span className="text-xs text-text-muted">{status}</span> : <span />}
        <div className="flex gap-3">
          <Button variant="ghost" onClick={onClose} disabled={creating}>
            Cancel
          </Button>
          <Button onClick={onCreate} loading={creating} disabled={!canCreate}>
            Create Task
          </Button>
        </div>
      </div>
    </>
  );
}
