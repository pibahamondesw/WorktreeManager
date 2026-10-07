import { useCachedUsage } from "../../hooks/useCachedUsage";
import { CODEX_PLAN_URL } from "../../services/usage";
import { UsageIndicator } from "./UsageIndicator";

export function CodexUsageIndicator() {
  const { snapshot, error, loading, refresh } = useCachedUsage("codex_cached_usage");
  return (
    <UsageIndicator
      agentLabel="Codex"
      label="Codex"
      planUrl={CODEX_PLAN_URL}
      usage={snapshot?.usage ?? null}
      primaryWindows={snapshot?.usage?.windows ?? []}
      error={error}
      loading={loading}
      updatedAt={snapshot?.updatedAt}
      emptyMessage="No current usage data. Use Codex to update it."
      onRefresh={refresh}
    />
  );
}
