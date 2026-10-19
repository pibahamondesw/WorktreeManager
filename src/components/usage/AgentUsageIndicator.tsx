import { useCachedUsage } from "../../hooks/useCachedUsage";
import { AGENTS } from "../../services/agents";
import { AgentId } from "../../types";
import { UsageIndicator } from "./UsageIndicator";

export function AgentUsageIndicator({ agent }: { agent: AgentId }) {
  const { label, productName, planUrl, primaryUsageWindows } = AGENTS[agent];
  const { snapshot, error, loading, refresh } = useCachedUsage(agent);
  const windows = snapshot?.usage?.windows ?? [];
  return (
    <UsageIndicator
      agentLabel={productName}
      label={label}
      planUrl={planUrl}
      usage={snapshot?.usage ?? null}
      primaryWindows={
        primaryUsageWindows
          ? windows.filter((window) => primaryUsageWindows.includes(window.id))
          : windows
      }
      error={error}
      loading={loading}
      updatedAt={snapshot?.updatedAt}
      emptyMessage={`No current usage data. Use ${productName} to update it.`}
      onRefresh={refresh}
    />
  );
}
