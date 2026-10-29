import { useEffect, useRef } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import { Operations } from "../services/operations";
import { CliBridgeRequest, dispatchCliBridge } from "../services/cliBridge";
import { AgentEvent } from "../services/agentActivity";

export function useCliBridge(
  operations: Operations | undefined,
  ready: boolean,
  onAgentEvent?: (event: AgentEvent) => { taskId: string | null }
) {
  const onAgentEventRef = useRef(onAgentEvent);
  useEffect(() => {
    onAgentEventRef.current = onAgentEvent;
  }, [onAgentEvent]);

  useEffect(() => {
    if (!operations || !ready) return;
    let disposed = false;
    const session = crypto.randomUUID();
    const channel = new Channel<{ token: string; request: CliBridgeRequest }>();
    channel.onmessage = ({ token, request }) => {
      if (disposed) return;
      void dispatchCliBridge(
        operations,
        request,
        (message) => {
          void invoke("cli_bridge_progress", { session, token, message }).catch(() => {});
        },
        onAgentEventRef.current && ((event) => onAgentEventRef.current!(event))
      )
        .then((response) => invoke("cli_bridge_complete", { session, token, response }))
        .catch(() => {});
    };
    void invoke("cli_bridge_register", { session, channel })
      .then(() => {
        if (disposed) void invoke("cli_bridge_unregister", { session }).catch(() => {});
      })
      .catch(() => {});
    return () => {
      disposed = true;
      void invoke("cli_bridge_unregister", { session }).catch(() => {});
    };
  }, [operations, ready]);
}
