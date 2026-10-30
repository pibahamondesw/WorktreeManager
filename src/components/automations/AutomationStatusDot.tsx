import { AUTOMATION_STATUS_LABELS, AutomationStatus } from "../../services/automations";

const STATUS_DOT_CLASSES: Record<AutomationStatus, string> = {
  invalid: "bg-danger",
  running: "bg-success motion-pulse-steps",
  paused: "bg-text-muted",
  degraded: "bg-warning",
  active: "bg-success",
};

export function AutomationStatusDot({ status }: { status: AutomationStatus }) {
  return (
    <span
      aria-hidden="true"
      title={AUTOMATION_STATUS_LABELS[status]}
      className={`w-1.5 h-1.5 rounded-full flex-shrink-0 ${STATUS_DOT_CLASSES[status]}`}
    />
  );
}
