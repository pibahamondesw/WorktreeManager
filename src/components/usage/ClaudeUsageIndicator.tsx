import { useCachedUsage } from "../../hooks/useCachedUsage";
import { UsageIndicator } from "./UsageIndicator";

export function ClaudeUsageIndicator() {
  const { snapshot, error, loading, refresh } = useCachedUsage("claude_cached_usage");
  return (
    <UsageIndicator
      agentLabel="Claude Code"
      label="Claude"
      usage={snapshot?.usage ?? null}
      primaryWindows={(snapshot?.usage?.windows ?? []).filter((window) =>
        ["five_hour", "seven_day"].includes(window.id)
      )}
      error={error}
      loading={loading}
      updatedAt={snapshot?.updatedAt}
      emptyMessage="No current usage data. Use Claude Code to update it."
      onRefresh={refresh}
    />
  );
}
