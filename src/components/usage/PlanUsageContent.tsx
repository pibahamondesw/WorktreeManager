import { formatReset, PlanUsage, UsageTone, usageTone, UsageWindow } from "../../services/usage";

const BAR_TONE: Record<UsageTone, string> = {
  normal: "bg-accent",
  warning: "bg-warning",
  danger: "bg-danger",
};

interface PlanUsageContentProps {
  usage: PlanUsage | null;
  error?: string | null;
}

export function PlanUsageContent({ usage, error }: PlanUsageContentProps) {
  const failure = error ?? usage?.error;
  if (failure && !usage?.windows.length)
    return <p className="text-sm text-danger select-text whitespace-pre-wrap">{failure}</p>;
  if (!usage) return <p className="text-sm text-text-muted">Loading usage…</p>;
  if (!usage.supported)
    return (
      <p className="text-sm text-text-muted">
        This agent version does not report plan usage. Update it to see your limits.
      </p>
    );
  return (
    <div className="flex flex-col gap-4">
      {failure && <p className="text-xs text-danger select-text whitespace-pre-wrap">{failure}</p>}
      {(usage.plan || usage.account) && (
        <div className="flex items-baseline justify-between gap-3 text-sm select-text">
          <span className="text-text-primary capitalize">{usage.plan}</span>
          <span className="text-text-muted truncate">{usage.account}</span>
        </div>
      )}
      {usage.windows.length === 0 ? (
        <p className="text-sm text-text-muted">No rate limits reported for this plan.</p>
      ) : (
        usage.windows.map((window) => <UsageBar key={window.id} window={window} />)
      )}
      {usage.session && (
        <pre className="text-xs text-text-secondary whitespace-pre-wrap select-text font-mono">
          {usage.session}
        </pre>
      )}
    </div>
  );
}

function UsageBar({ window }: { window: UsageWindow }) {
  const percent = Math.round(Math.min(100, Math.max(0, window.usedPercent)));
  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-baseline justify-between text-sm">
        <span className="text-text-primary">{window.label}</span>
        <span className="text-text-secondary">{percent}% used</span>
      </div>
      <div
        role="progressbar"
        aria-label={window.label}
        aria-valuenow={percent}
        aria-valuemin={0}
        aria-valuemax={100}
        className="h-1.5 rounded-full bg-bg-hover overflow-hidden"
      >
        <div
          className={`h-full rounded-full ${BAR_TONE[usageTone(percent)]}`}
          style={{ width: `${percent}%` }}
        />
      </div>
      {window.resetsAt !== null && (
        <span className="text-xs text-text-muted">{formatReset(window.resetsAt)}</span>
      )}
    </div>
  );
}
