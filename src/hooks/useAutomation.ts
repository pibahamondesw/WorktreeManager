import { useEffect } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import { Operations } from "../services/operations";
import { AutomationRequest, dispatchAutomation } from "../services/automation";

export function useAutomation(operations: Operations | undefined, ready: boolean) {
  useEffect(() => {
    if (!operations || !ready) return;
    let disposed = false;
    const session = crypto.randomUUID();
    const channel = new Channel<{ token: string; request: AutomationRequest }>();
    channel.onmessage = ({ token, request }) => {
      if (disposed) return;
      void dispatchAutomation(operations, request, (message) => {
        void invoke("automation_progress", { session, token, message }).catch(() => {});
      })
        .then((response) => invoke("automation_complete", { session, token, response }))
        .catch(() => {});
    };
    void invoke("automation_register", { session, channel })
      .then(() => {
        if (disposed) void invoke("automation_unregister", { session }).catch(() => {});
      })
      .catch(() => {});
    return () => {
      disposed = true;
      void invoke("automation_unregister", { session }).catch(() => {});
    };
  }, [operations, ready]);
}
