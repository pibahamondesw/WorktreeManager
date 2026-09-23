import { Channel, invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import { AgentId } from "../types";

export type ChatStatus =
  | { kind: "starting" }
  | { kind: "idle" }
  | { kind: "busy" }
  | { kind: "exited"; code: number | null }
  | { kind: "failed"; message: string };

export type ChatItemKind =
  | "user"
  | "assistant"
  | "reasoning"
  | "command"
  | "fileChange"
  | "tool"
  | "error"
  | "notice";

export interface ChatItem {
  id: string;
  kind: ChatItemKind;
  text: string;
  title?: string;
  detail?: string;
  status?: string;
}

export interface ChatQuestion {
  id: string;
  header: string;
  question: string;
  options: { label: string; description?: string }[];
  allowOther: boolean;
  multiSelect: boolean;
  secret: boolean;
}

export type PendingRequest = { id: string; title: string; detail?: string } & (
  | { kind: "approval"; decisions: { id: string; label: string }[] }
  | { kind: "question"; questions: ChatQuestion[] }
  | { kind: "unsupported" }
);

export type ChatEvent =
  | { type: "items"; items: ChatItem[] }
  | { type: "upsert"; item: ChatItem }
  | { type: "delta"; id: string; field: "text" | "detail"; delta: string }
  | { type: "pending"; request: PendingRequest }
  | { type: "resolved"; id: string }
  | { type: "status"; status: ChatStatus };

export interface ChatSnapshot {
  generation: number;
  status: ChatStatus;
  items: ChatItem[];
  pending: PendingRequest[];
}

export interface ChatResponse {
  decision?: string;
  answers?: Record<string, string[]>;
}

export interface ChatInfo {
  taskId: string;
  agent: AgentId;
  status: ChatStatus;
}

export const isLive = (status: ChatStatus) => status.kind !== "exited" && status.kind !== "failed";

/** Mirror of `ChatSnapshot::apply` in Rust; keep the two in step. */
export function applyChatEvent(snapshot: ChatSnapshot, event: ChatEvent): ChatSnapshot {
  switch (event.type) {
    case "items":
      return { ...snapshot, items: event.items };
    case "upsert": {
      const index = snapshot.items.findIndex((item) => item.id === event.item.id);
      const items =
        index === -1
          ? [...snapshot.items, event.item]
          : snapshot.items.map((item, i) => (i === index ? event.item : item));
      return { ...snapshot, items };
    }
    case "delta":
      return {
        ...snapshot,
        items: snapshot.items.map((item) =>
          item.id !== event.id
            ? item
            : event.field === "text"
              ? { ...item, text: item.text + event.delta }
              : { ...item, detail: (item.detail ?? "") + event.delta }
        ),
      };
    case "pending":
      return {
        ...snapshot,
        pending: [...snapshot.pending.filter((p) => p.id !== event.request.id), event.request],
      };
    case "resolved":
      return { ...snapshot, pending: snapshot.pending.filter((p) => p.id !== event.id) };
    case "status":
      return {
        ...snapshot,
        status: event.status,
        pending: isLive(event.status) ? snapshot.pending : [],
      };
  }
}

interface ChatOpenArgs {
  taskId: string;
  agent: AgentId;
  folders: string[];
  restart: boolean;
  onEvent: (generation: number, event: ChatEvent) => void;
}

export function chatOpen({ onEvent, ...args }: ChatOpenArgs): Promise<ChatSnapshot> {
  const channel = new Channel<{ generation: number; event: ChatEvent }>();
  channel.onmessage = (message) => onEvent(message.generation, message.event);
  return invoke<ChatSnapshot>("chat_open", { ...args, onEvent: channel });
}

export const chatSend = (taskId: string, agent: AgentId, text: string) =>
  invoke<void>("chat_send", { taskId, agent, text });

export const chatInterrupt = (taskId: string, agent: AgentId) =>
  invoke<void>("chat_interrupt", { taskId, agent });

export const chatRespond = (
  taskId: string,
  agent: AgentId,
  requestId: string,
  response: ChatResponse
) => invoke<void>("chat_respond", { taskId, agent, requestId, response });

export const chatDetach = (taskId: string, agent: AgentId, generation: number) =>
  invoke<void>("chat_detach", { taskId, agent, generation }).catch(() => undefined);

export const chatClose = (taskId: string) => invoke<void>("chat_close", { taskId });

export const chatList = () => invoke<ChatInfo[]>("chat_list");

export const onChatStatus = (callback: (info: ChatInfo) => void): Promise<UnlistenFn> =>
  listen<ChatInfo>("chat-status", (e) => callback(e.payload));
