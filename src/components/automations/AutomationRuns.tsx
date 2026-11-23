import { useAutomationRuns } from "../../hooks/useAutomations";
import { runDuration, runResult, runSource, runStartedAt } from "../../services/automations";
import { Button } from "../ui/Button";

const RUN_ROW_COLUMNS = "grid grid-cols-[11rem_minmax(0,1fr)_minmax(0,2fr)_5rem] gap-x-4";

export function AutomationRuns({ name }: { name: string }) {
  const { runs, error, loading, loadMore } = useAutomationRuns(name);

  if (error) return <p className="text-sm text-danger break-words select-text">{error}</p>;
  if (runs.length === 0)
    return <p className="text-sm text-text-muted">{loading ? "Loading runs…" : "No runs yet"}</p>;

  return (
    <div className="space-y-3">
      <div role="table" aria-label="Runs" className="text-xs select-text">
        <div role="rowgroup" className="text-text-muted">
          <div role="row" className={RUN_ROW_COLUMNS}>
            <span role="columnheader" className="py-1.5 font-medium">
              Date
            </span>
            <span role="columnheader" className="py-1.5 font-medium">
              Source
            </span>
            <span role="columnheader" className="py-1.5 font-medium">
              Result
            </span>
            <span role="columnheader" className="py-1.5 font-medium">
              Duration
            </span>
          </div>
        </div>
        <div role="rowgroup" className="text-text-secondary">
          {runs.map((run) => (
            <div
              key={run.id}
              role="row"
              className={`${RUN_ROW_COLUMNS} offscreen-lazy [contain-intrinsic-size:auto_1.8125rem] border-t border-border`}
            >
              <span role="cell" className="py-1.5 whitespace-nowrap" title={run.id}>
                {runStartedAt(run).toLocaleString()}
              </span>
              <span role="cell" className="py-1.5 break-words">
                {runSource(run)}
              </span>
              <span role="cell" className="py-1.5 break-words">
                {runResult(run)}
              </span>
              <span role="cell" className="py-1.5 whitespace-nowrap">
                {runDuration(run) ?? "—"}
              </span>
            </div>
          ))}
        </div>
      </div>
      {loadMore && (
        <Button variant="secondary" className="h-8 text-xs" loading={loading} onClick={loadMore}>
          Load older runs
        </Button>
      )}
    </div>
  );
}
