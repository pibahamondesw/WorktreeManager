import { useEffect, useState } from "react";
import {
  AgentDescriptor,
  cachedAgentCatalog,
  describeAgent,
  loadAgentCatalog,
} from "../services/agents";
import { AgentId } from "../types";

/** The agent catalog, starting from the last read and refreshed on mount. */
export function useAgentCatalog(): AgentDescriptor[] {
  const [catalog, setCatalog] = useState(cachedAgentCatalog);
  useEffect(() => {
    let stale = false;
    loadAgentCatalog().then(
      (found) => !stale && setCatalog(found),
      () => undefined
    );
    return () => {
      stale = true;
    };
  }, []);
  return catalog;
}

export const useAgent = (id: AgentId): AgentDescriptor => describeAgent(id, useAgentCatalog());
