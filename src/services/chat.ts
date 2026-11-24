import { Channel, invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import { AgentId } from "../types";
import { PlanUsage } from "./usage";

export type ChatStatus =
  | { kind: "starting" }
  | { kind: "idle" }
  | { kind: "busy" }
  | { kind: "exited"; code: number | null }
  | { kind: "failed"; message: string };

type ChatItemKind =
  | "user"
  | "assistant"
  | "reasoning"
  | "command"
  | "fileChange"
  | "tool"
  | "error"
  | "notice"
  | "review"
  | "handoff"
  | "agent"
  | "question";

export interface AskedQuestion {
  header: string;
  question: string;
  answer?: string;
}

export interface ChatItem {
  id: string;
  kind: ChatItemKind;
  text: string;
  title?: string;
  detail?: string;
  /** Bytes in the full output when `detail` holds only its end, or none of it. */
  detailLength?: number;
  status?: string;
  checkpoint?: string;
  durationMs?: number;
  /** Carried from this agent's conversation; read-only here. */
  origin?: AgentId;
  /** The subagent run (an `agent` item's id) this item is a step of. */
  parent?: string;
  questions?: AskedQuestion[];
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

export type PendingRequest = {
  id: string;
  title: string;
  detail?: string;
  format: "text" | "diff" | "markdown";
} & (
  | {
      kind: "approval";
      decisions: { id: string; label: string }[];
      feedback?: boolean;
      editablePlan?: boolean;
    }
  | { kind: "question"; questions: ChatQuestion[] }
  | ({ kind: "elicitation"; serverName: string } & (
      | { mode: "form"; requestedSchema: Record<string, unknown> | boolean }
      | { mode: "url"; url: string; elicitationId: string }
    ))
  | { kind: "unsupported" }
);

interface ModelOption {
  id: string;
  label: string;
  description?: string;
  efforts: string[];
  images: boolean;
}

export interface ChatImage {
  mediaType: string;
  data: string;
}

interface ModeOption {
  id: string;
  label: string;
  description: string;
}

type CommandAction =
  | { kind: "insert"; text: string }
  | { kind: "model" }
  | { kind: "effort" }
  | { kind: "mode"; mode: string }
  | { kind: "compact" }
  | { kind: "review" }
  | { kind: "clear" }
  | { kind: "hooks" }
  | { kind: "status" };

export interface CommandOption {
  name: string;
  description: string;
  argumentHint?: string;
  action: CommandAction;
}

export interface TodoItem {
  text: string;
  status: "pending" | "inProgress" | "completed";
}

export interface ChatControls {
  models: ModelOption[];
  model: string | null;
  effort: string | null;
  modes: ModeOption[];
  mode: string | null;
  commands: CommandOption[];
  todos: TodoItem[];
  context: { used: number; max: number } | null;
  usage: PlanUsage | null;
  hooks?: HookControls;
}

export interface HookOption {
  key: string;
  eventName: string;
  sourcePath: string;
  currentHash: string;
  enabled: boolean;
  isManaged: boolean;
  trustStatus: string;
  matcher: string | null;
  handlerType: string;
  command: string | null;
  server: string | null;
  tool: string | null;
}

export interface HookControls {
  items: HookOption[];
  loading: boolean;
  error: string | null;
  warnings: string[];
}

export type ChatSetting =
  | { kind: "model"; value: string }
  | { kind: "effort"; value: string }
  | { kind: "mode"; value: string }
  | { kind: "hooks" }
  | { kind: "hook"; value: { key: string; hash: string; enabled?: boolean; trust?: boolean } };

export interface FileMatch {
  path: string;
  name: string;
}

export const EMPTY_CONTROLS: ChatControls = {
  models: [],
  model: null,
  effort: null,
  modes: [],
  mode: null,
  commands: [],
  todos: [],
  context: null,
  usage: null,
};

export const acceptsImages = (controls: ChatControls) =>
  controls.models.find((model) => model.id === controls.model)?.images ?? true;

export function effortsFor(controls: ChatControls): string[] {
  return controls.models.find((model) => model.id === controls.model)?.efforts ?? [];
}

export type ChatEvent =
  | { type: "items"; items: ChatItem[] }
  | { type: "upsert"; item: ChatItem }
  | { type: "remove"; id: string }
  | { type: "delta"; id: string; field: "text" | "detail"; delta: string }
  | { type: "pending"; request: PendingRequest }
  | { type: "resolved"; id: string }
  | { type: "status"; status: ChatStatus }
  | { type: "controls"; controls: ChatControls }
  | { type: "earlier"; items: ChatItem[]; earlier: number };

export interface ChatSnapshot {
  generation: number;
  status: ChatStatus;
  items: ChatItem[];
  /** Items before `items` not loaded yet; `chatLoadEarlier` sends the page before the oldest one. */
  earlier?: number;
  pending: PendingRequest[];
  controls: ChatControls;
}

export interface ChatResponse {
  decision?: string;
  message?: string;
  plan?: string;
  answers?: Record<string, string[]>;
  elicitation?: {
    action: "accept" | "decline" | "cancel";
    content: unknown;
  };
}

/** A user session is its task agent's primary chat; automations run beside it. */
type ChatOrigin = "user" | "automation";

export interface ChatInfo {
  sessionId: string;
  taskId: string;
  agent: AgentId;
  /** The chat tab a user session runs in; absent for the main tab. */
  tab?: string;
  origin: ChatOrigin;
  status: ChatStatus;
  /** An approval or question is waiting for an answer. */
  waiting: boolean;
  /** Background subagents keep the agent working after its turn ends. */
  backgroundAgents: boolean;
  /** The current turn reported an agent error; cleared by the next prompt. */
  hasError?: boolean;
  /** The turn running, or the last one, is an idle compaction the app started, not the user. */
  idleCompaction?: boolean;
}

export const isLive = (status: ChatStatus) => status.kind !== "exited" && status.kind !== "failed";

/** Mirror of `ChatSnapshot::apply` in Rust; keep the two in step. */
export function applyChatEvent(snapshot: ChatSnapshot, event: ChatEvent): ChatSnapshot {
  switch (event.type) {
    case "items":
      return { ...snapshot, items: event.items, earlier: 0 };
    case "upsert": {
      const index = snapshot.items.findIndex((item) => item.id === event.item.id);
      const items =
        index === -1
          ? [...snapshot.items, event.item]
          : snapshot.items.map((item, i) => (i === index ? event.item : item));
      return { ...snapshot, items };
    }
    case "remove":
      return { ...snapshot, items: snapshot.items.filter((item) => item.id !== event.id) };
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
    case "controls":
      return { ...snapshot, controls: event.controls };
    case "earlier":
      return { ...snapshot, items: [...event.items, ...snapshot.items], earlier: event.earlier };
  }
}

export type ChatOpenMode = "attach" | "restart" | "fresh";

/** How a chat pane opens: a picked conversation, or a fresh one continuing another agent's. */
export interface ChatOpenIntent {
  agent: AgentId;
  mode: ChatOpenMode;
  conversation?: string;
  handoffFrom?: AgentId;
}

interface ChatOpenArgs {
  taskId: string;
  agent: AgentId;
  folders: string[];
  mode: ChatOpenMode;
  conversation?: string;
  handoffFrom?: AgentId;
  tab?: string;
  /** Attaches to this session of the task's agent instead of the tab's own, whoever started it. */
  sessionId?: string;
  onEvents: (generation: number, events: ChatEvent[]) => void;
}

export type OpenedChat = ChatSnapshot & { sessionId: string };

export function chatOpen({ onEvents, ...args }: ChatOpenArgs): Promise<OpenedChat> {
  const channel = new Channel<{ generation: number; events: ChatEvent[] }>();
  channel.onmessage = (message) => onEvents(message.generation, message.events);
  return invoke<OpenedChat>("chat_open", { ...args, onEvent: channel });
}

export interface ChatSessionSummary {
  id: string;
  title: string;
  updatedAt: number;
  current: boolean;
}

export interface ChatSessionList {
  sessions: ChatSessionSummary[];
  canRename: boolean;
  canArchive: boolean;
}

export const chatSessions = (taskId: string, agent: AgentId, folders: string[]) =>
  invoke<ChatSessionList>("chat_sessions", { taskId, agent, folders });

export const chatSessionRename = (
  taskId: string,
  agent: AgentId,
  folders: string[],
  id: string,
  title: string,
  tab?: string
) => invoke<void>("chat_session_rename", { taskId, agent, folders, id, title, tab });

export const chatSessionArchive = (taskId: string, agent: AgentId, folders: string[], id: string) =>
  invoke<void>("chat_session_archive", { taskId, agent, folders, id });

export const chatSend = (
  taskId: string,
  agent: AgentId,
  text: string,
  images: ChatImage[] = [],
  tab?: string
) => invoke<void>("chat_send", { taskId, agent, text, images, tab });

export type HistoryAction = "code" | "conversation" | "both";

export interface RewindPreview {
  token: string;
  files: string[];
}

export const chatRewindPreview = (
  taskId: string,
  agent: AgentId,
  generation: number,
  itemId: string,
  tab?: string
) => invoke<RewindPreview>("chat_rewind_preview", { taskId, agent, generation, itemId, tab });

export const chatHistory = (
  taskId: string,
  agent: AgentId,
  generation: number,
  itemId: string,
  action: HistoryAction,
  confirmation?: string,
  tab?: string
) => invoke<void>("chat_history", { taskId, agent, generation, itemId, action, confirmation, tab });

export interface FileChange {
  folder: string;
  path: string;
  previousPath?: string;
  status: "added" | "modified" | "deleted" | "renamed";
  additions: number;
  deletions: number;
  binary: boolean;
  content: string;
  generated?: boolean;
}

export interface ChangeSet {
  before: string;
  after: string;
  files: FileChange[];
}

export interface ChatChanges {
  turns: { items: string[]; changes: ChangeSet; branch: ChangeSet | null }[];
  conversation: ChangeSet | null;
}

export type FileContent =
  | { kind: "text"; text: string }
  | { kind: "image"; mime: string; data: string }
  | { kind: "binary"; size: number };

export interface FileDiff {
  before: FileContent | null;
  after: FileContent | null;
}

const folderName = (folder: string) => folder.split("/").filter(Boolean).pop() ?? folder;

export function totals(files: FileChange[]) {
  return files.reduce(
    (sum, file) => ({
      additions: sum.additions + file.additions,
      deletions: sum.deletions + file.deletions,
    }),
    { additions: 0, deletions: 0 }
  );
}

export function byRepository(files: FileChange[]) {
  const groups = new Map<string, FileChange[]>();
  for (const file of files) groups.set(file.folder, [...(groups.get(file.folder) ?? []), file]);
  return [...groups].map(([folder, repositoryFiles]) => ({
    folder,
    name: folderName(folder),
    files: repositoryFiles,
  }));
}

export const chatChanges = (taskId: string, agent: AgentId, tab?: string) =>
  invoke<ChatChanges>("chat_changes", { taskId, agent, tab });

export const chatFileDiff = (
  taskId: string,
  agent: AgentId,
  changes: ChangeSet,
  file: FileChange,
  tab?: string
) =>
  invoke<FileDiff>("chat_file_diff", {
    taskId,
    agent,
    tab,
    before: changes.before,
    after: changes.after,
    folder: file.folder,
    path: file.path,
    previousPath: file.previousPath ?? null,
  });

export const reviewKeyOf = (file: FileChange) => `${file.folder}\n${file.path}\n${file.content}`;

export const chatReviewedFiles = (taskId: string, agent: AgentId, tab?: string) =>
  invoke<string[]>("chat_reviewed_files", { taskId, agent, tab });

export const chatSetFileReviewed = (
  taskId: string,
  agent: AgentId,
  file: FileChange,
  reviewed: boolean,
  tab?: string
) =>
  invoke<string[]>("chat_set_file_reviewed", {
    taskId,
    agent,
    tab,
    key: reviewKeyOf(file),
    reviewed,
  });

export const chatConfigure = (taskId: string, agent: AgentId, setting: ChatSetting, tab?: string) =>
  invoke<void>("chat_configure", { taskId, agent, setting, tab });

export const chatCompact = (taskId: string, agent: AgentId, tab?: string) =>
  invoke<void>("chat_compact", { taskId, agent, tab });

export type ReviewTarget =
  | { type: "uncommittedChanges" }
  | { type: "baseBranch"; branch: string }
  | { type: "commit"; sha: string; title: null }
  | { type: "custom"; instructions: string };

export const chatReview = (taskId: string, agent: AgentId, target: ReviewTarget, tab?: string) =>
  invoke<void>("chat_review", { taskId, agent, target, tab });

export const chatRefreshUsage = (taskId: string, agent: AgentId, tab?: string) =>
  invoke<void>("chat_refresh_usage", { taskId, agent, tab });

export const chatFileSearch = (folders: string[], query: string) =>
  invoke<FileMatch[]>("chat_file_search", { folders, query });

export interface DroppedImage {
  name: string;
  mediaType: string;
  data: string;
}

export const chatReadImage = (path: string) => invoke<DroppedImage>("chat_read_image", { path });

export const chatClipboardText = () => invoke<string>("chat_clipboard_text");

export interface MarkdownDocument {
  path: string;
  text: string;
}

export interface PreviewFile {
  path: string;
  content: FileContent;
}

export const chatReadFile = (folders: string[], base: string | null, path: string) =>
  invoke<PreviewFile>("chat_read_file", { folders, base, path });

export const chatWriteFile = (folders: string[], path: string, original: string, text: string) =>
  invoke<PreviewFile>("chat_write_file", { folders, path, original, text });

export const chatReadMarkdown = (folders: string[], path: string) =>
  invoke<MarkdownDocument>("chat_read_markdown", { folders, path });

export const chatInterrupt = (taskId: string, agent: AgentId, tab?: string) =>
  invoke<void>("chat_interrupt", { taskId, agent, tab });

/** Redirects the turn in progress with a message queued during it. */
export const chatSteerQueued = (taskId: string, agent: AgentId, itemId: string, tab?: string) =>
  invoke<void>("chat_steer_queued", { taskId, agent, itemId, tab });

export const chatUnqueue = (taskId: string, agent: AgentId, itemId: string, tab?: string) =>
  invoke<void>("chat_unqueue", { taskId, agent, itemId, tab });

/** Keeps a queued message from running while it is edited; resolves to its text. */
export const chatHoldQueued = (taskId: string, agent: AgentId, itemId: string, tab?: string) =>
  invoke<string>("chat_hold_queued", { taskId, agent, itemId, tab });

/** How an edit of a queued message ends; without `text` the message stays as it was. */
export interface QueuedEdit {
  text?: string;
  steer?: boolean;
}

/** Ends the edit of a held message, which runs at once if no turn is in progress. */
export const chatReleaseQueued = (
  taskId: string,
  agent: AgentId,
  itemId: string,
  edit: QueuedEdit,
  tab?: string
) => invoke<void>("chat_release_queued", { taskId, agent, itemId, edit, tab });

export const chatRespond = (
  taskId: string,
  agent: AgentId,
  requestId: string,
  response: ChatResponse,
  generation: number,
  tab?: string
) => invoke<void>("chat_respond", { taskId, agent, requestId, response, generation, tab });

export const chatDetach = (
  taskId: string,
  agent: AgentId,
  generation: number,
  sessionId: string,
  tab?: string
) =>
  invoke<void>("chat_detach", { taskId, agent, generation, sessionId, tab }).catch(() => undefined);

/** How much of an expanded row's output the pane receives. */
export type DetailView = "tail" | "full";

export const chatExpandDetail = (
  sessionId: string,
  generation: number,
  itemId: string,
  view: DetailView | null
) =>
  invoke<void>("chat_expand_detail", { sessionId, generation, itemId, view }).catch(
    () => undefined
  );

export const chatLoadEarlier = (sessionId: string, generation: number) =>
  invoke<void>("chat_load_earlier", { sessionId, generation }).catch(() => undefined);

export const chatStop = (taskId: string, agent: AgentId, tab?: string) =>
  invoke<void>("chat_stop", { taskId, agent, tab });

export const chatClose = (taskId: string) => invoke<void>("chat_close", { taskId });

export const chatList = () => invoke<ChatInfo[]>("chat_list");

export const onChatStatus = (callback: (info: ChatInfo) => void): Promise<UnlistenFn> =>
  listen<ChatInfo>("chat-status", (e) => callback(e.payload));

/** The tab every task has until tabs are recorded; it keeps the chat of the task before tabs. */
export const MAIN_TAB = "main";

export interface ChatTab {
  id: string;
  agent: AgentId;
  conversationId: string | null;
}

/** A task's open chat tabs, in order; `mainAgent` runs its main tab until tabs are recorded. */
/** Without a main agent, a task whose tabs were never shown has none. */
export const chatTabs = (taskId: string, mainAgent: AgentId | null, folders: string[]) =>
  invoke<ChatTab[]>("chat_tabs", { taskId, mainAgent, folders });

export const chatTabOpen = (taskId: string, mainAgent: AgentId, agent: AgentId) =>
  invoke<Omit<ChatTab, "conversationId">>("chat_tab_open", { taskId, mainAgent, agent });

export const chatTabSetAgent = (taskId: string, mainAgent: AgentId, tab: string, agent: AgentId) =>
  invoke<void>("chat_tab_set_agent", { taskId, mainAgent, tab, agent });

export const chatTabClose = (taskId: string, mainAgent: AgentId, tab: string) =>
  invoke<void>("chat_tab_close", { taskId, mainAgent, tab });

export const onChatTabs = (callback: (taskId: string) => void): Promise<UnlistenFn> =>
  listen<{ taskId: string }>("chat-tabs", (e) => callback(e.payload.taskId));
