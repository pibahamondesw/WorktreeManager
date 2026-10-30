import { useAutomationRuns } from "../../hooks/useAutomations";
import { runDuration, runResult, runStartedAt } from "../../services/automations";
import { Button } from "../ui/Button";

export function AutomationRuns({ name }: { name: string }) {
  const { runs, error, loading, loadMore } = useAutomationRuns(name);

  if (error) return <p className="text-sm text-danger break-words select-text">{error}</p>;
  if (runs.length === 0)
    return <p className="text-sm text-text-muted">{loading ? "Loading runs…" : "No runs yet"}</p>;

  return (
    <div className="space-y-3">
      <table className="w-full text-xs select-text">
        <thead className="text-left text-text-muted">
          <tr>
            <th className="py-1.5 pr-4 font-medium">Date</th>
            <th className="py-1.5 pr-4 font-medium">Source</th>
            <th className="py-1.5 pr-4 font-medium">Result</th>
            <th className="py-1.5 font-medium">Duration</th>
          </tr>
        </thead>
        <tbody className="text-text-secondary">
          {runs.map((run) => (
            <tr key={run.id} className="border-t border-border">
              <td className="py-1.5 pr-4 whitespace-nowrap" title={run.id}>
                {runStartedAt(run).toLocaleString()}
              </td>
              <td className="py-1.5 pr-4">
                {run.trigger?.source ?? "unknown"}
                {run.trigger?.test ? " · test" : ""}
              </td>
              <td className="py-1.5 pr-4">{runResult(run)}</td>
              <td className="py-1.5 whitespace-nowrap">{runDuration(run) ?? "—"}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {loadMore && (
        <Button variant="secondary" className="h-8 text-xs" loading={loading} onClick={loadMore}>
          Load older runs
        </Button>
      )}
    </div>
  );
}
