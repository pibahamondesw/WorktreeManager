import { useCachedUsage } from "../../hooks/useCachedUsage";
import { useAgent } from "../../hooks/useAgentCatalog";
import { agentPresentation } from "../../services/agents";
import { AgentId } from "../../types";
import { UsageIndicator } from "./UsageIndicator";

export function AgentUsageIndicator({ agent }: { agent: AgentId }) {
  const { label } = useAgent(agent);
  const { planUrl, primaryUsageWindows } = agentPresentation(agent);
  const { snapshot, error, loading, refresh } = useCachedUsage(agent);
  const windows = snapshot?.usage?.windows ?? [];
  return (
    <UsageIndicator
      agentLabel={label}
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
      emptyMessage={`No current usage data. Use ${label} to update it.`}
      onRefresh={refresh}
    />
  );
}
