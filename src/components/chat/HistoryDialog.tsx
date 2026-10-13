import { useEffect, useRef, useState } from "react";
import { ChatItem, HistoryAction, RewindPreview } from "../../services/chat";
import { Button } from "../ui/Button";
import { Modal } from "../ui/Modal";

interface HistoryDialogProps {
  item: ChatItem;
  action: Exclude<HistoryAction, "conversation">;
  disabled: boolean;
  onPreview: () => Promise<RewindPreview>;
  onApply: (action: HistoryAction, confirmation?: string) => Promise<void>;
  onClose: () => void;
}

export function HistoryDialog({
  item,
  action,
  disabled,
  onPreview,
  onApply,
  onClose,
}: HistoryDialogProps) {
  const [preview, setPreview] = useState<RewindPreview | null>(null);
  const [pending, setPending] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const initialPreview = useRef(onPreview);

  useEffect(() => {
    let disposed = false;
    void initialPreview
      .current()
      .then((preview) => {
        if (!disposed) setPreview(preview);
      })
      .catch((error: unknown) => {
        if (!disposed) setError(String(error));
      })
      .finally(() => {
        if (!disposed) setPending(false);
      });
    return () => {
      disposed = true;
    };
  }, []);

  const apply = async () => {
    setPending(true);
    setError(null);
    try {
      if (!preview) {
        setPreview(await onPreview());
      } else {
        await onApply(action, preview?.token);
        onClose();
      }
    } catch (e) {
      setError(String(e));
      setPreview(null);
    } finally {
      setPending(false);
    }
  };

  return (
    <Modal
      open
      onClose={() => {
        if (!pending) onClose();
      }}
      title={action === "both" ? "Fork conversation and rewind code" : "Rewind code"}
    >
      <div className="p-6 flex flex-col gap-4">
        <p className="text-sm text-text-secondary">
          Restore files to the state before this message.
          {action === "both" && " The original conversation stays in Conversations."}
        </p>
        <p className="text-sm text-text-primary whitespace-pre-wrap select-text max-h-24 overflow-y-auto">
          {item.text}
        </p>
        {!item.checkpoint && (
          <p className="text-xs text-text-muted">
            No code checkpoint is available for this message.
          </p>
        )}
        {preview && (
          <div className="text-sm text-text-secondary flex flex-col gap-2">
            <p>
              This replaces the current contents of these files, including your edits. Your current
              files will be backed up. Ignored files stay unchanged.
            </p>
            <ul
              aria-label="Files to restore"
              className="max-h-48 overflow-auto text-xs font-mono select-text"
            >
              {preview.files.map((file) => (
                <li key={file}>{file}</li>
              ))}
            </ul>
            {preview.files.length === 0 && <p>No file changes.</p>}
          </div>
        )}
        {pending && !preview && (
          <p role="status" className="text-sm text-text-muted">
            Loading file changes…
          </p>
        )}
        {error && (
          <p role="alert" className="text-sm text-danger select-text">
            {error}
          </p>
        )}
        <div className="flex justify-end gap-2">
          <Button variant="ghost" disabled={pending} onClick={onClose}>
            Cancel
          </Button>
          <Button disabled={pending || disabled || !item.checkpoint} onClick={() => void apply()}>
            {pending ? "Working…" : preview ? "Confirm restore" : "Preview file changes"}
          </Button>
        </div>
      </div>
    </Modal>
  );
}
