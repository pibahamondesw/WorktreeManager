import { ReactNode } from "react";

export function EmptyState({
  title,
  action,
  className = "",
}: {
  title: ReactNode;
  action?: ReactNode;
  className?: string;
}) {
  return (
    <div className={`text-center ${className}`}>
      <p className="text-sm text-text-muted">{title}</p>
      {action && <div className="mt-2">{action}</div>}
    </div>
  );
}
