import { useEffect, useState } from "react";
import { DiffThread } from "../components/chat/diffComments";

const pendingBySession = new Map<string, DiffThread[]>();

export function useDiffThreads(sessionKey: string) {
  const [threads, setThreads] = useState(() => pendingBySession.get(sessionKey) ?? []);

  useEffect(() => {
    if (threads.length) pendingBySession.set(sessionKey, threads);
    else pendingBySession.delete(sessionKey);
  }, [sessionKey, threads]);

  return [threads, setThreads] as const;
}
