import { useEffect, useState } from "react";
import { ChatInfo, chatList, MAIN_TAB, onChatStatus } from "../services/chat";

/** The owner's chat running in a task's tab, as last reported; null until one reports. */
export function useTabChat(taskId: string, agent: string, tab: string): ChatInfo | null {
  const [chat, setChat] = useState<ChatInfo | null>(null);
  useEffect(() => {
    let disposed = false;
    setChat(null);
    const record = (info: ChatInfo) => {
      const runsTab =
        info.taskId === taskId &&
        info.agent === agent &&
        info.origin === "user" &&
        (info.tab ?? MAIN_TAB) === tab;
      if (!disposed && runsTab) setChat(info);
    };
    const listening = onChatStatus(record);
    chatList()
      .then((listed) => listed.forEach(record))
      .catch(() => undefined);
    return () => {
      disposed = true;
      void listening.then((stop) => stop()).catch(() => undefined);
    };
  }, [taskId, agent, tab]);
  return chat;
}
