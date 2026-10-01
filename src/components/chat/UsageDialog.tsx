import { useEffect, useState } from "react";
import { Modal } from "../ui/Modal";
import { PlanUsageContent } from "../usage/PlanUsageContent";
import { PlanUsage } from "../../services/usage";

interface UsageDialogProps {
  open: boolean;
  usage: PlanUsage | null;
  onRefresh: () => Promise<void>;
  onClose: () => void;
}

export function UsageDialog({ open, usage, onRefresh, onClose }: UsageDialogProps) {
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setError(null);
    onRefresh().catch((e) => setError(String(e)));
  }, [open, onRefresh]);

  return (
    <Modal open={open} onClose={onClose} title="Plan usage">
      <div className="px-6 py-5">
        <PlanUsageContent usage={usage} error={error} />
      </div>
    </Modal>
  );
}
