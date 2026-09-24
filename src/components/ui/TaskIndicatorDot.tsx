import { INDICATOR_LABELS, TaskIndicator } from "../../services/agentActivity";

const INDICATOR_CLASSES: Record<TaskIndicator, string> = {
  input: "bg-warning",
  working: "bg-success animate-pulse",
  idle: "bg-success",
  ended: "bg-text-muted",
};

export function TaskIndicatorDot({
  indicator,
  title = INDICATOR_LABELS[indicator],
}: {
  indicator: TaskIndicator;
  title?: string;
}) {
  return (
    <span
      className={`w-1.5 h-1.5 rounded-full flex-shrink-0 ${INDICATOR_CLASSES[indicator]}`}
      title={title}
    />
  );
}
