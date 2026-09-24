import { useState } from "react";
import { TaskSetupProgress } from "./TaskSetupProgress";
import { TaskHeader } from "./TaskHeader";
import { TerminalPane } from "../terminal/TerminalPane";
import { ChatPane } from "../chat/ChatPane";
import { AgentSurfaceSwitcher } from "./AgentSurfaceSwitcher";
import { EditorPane } from "../editor/EditorPane";
import { editorClose } from "../../services/codeEditor";
import { chatStop } from "../../services/chat";
import { terminalStop } from "../../services/terminal";
import { useKeyboardShortcuts } from "../../hooks/useKeyboardShortcuts";
import { SessionStatus } from "../../hooks/useTerminalSession";
import { GitStatus, IssueLinearInfo, Task, TaskSurface } from "../../types";

interface TaskViewProps {
  task: Task;
  surface: TaskSurface;
  linearInfo?: IssueLinearInfo;
  gitStatus?: GitStatus;
  sidebarCollapsed: boolean;
  onExpandSidebar: () => void;
  onBack: () => void;
  onSwitchSurface: (surface: TaskSurface) => void;
}

/** A task opened inside the app: compact header on top, the task's surface filling the rest. */
export function TaskView({
  task,
  surface,
  linearInfo,
  gitStatus,
  sidebarCollapsed,
  onExpandSidebar,
  onBack,
  onSwitchSurface,
}: TaskViewProps) {
  const [switching, setSwitching] = useState(false);
  const [sessionStatus, setSessionStatus] = useState<SessionStatus>({ kind: "connecting" });
  const [closeError, setCloseError] = useState<string | null>(null);
  const [closing, setClosing] = useState(false);

  /** Ends the running surface's process; agent sessions keep their conversation for resuming. */
  const stopSurface = () => {
    if (surface.kind === "editor") return editorClose(task.id);
    if (surface.kind === "chat") return chatStop(task.id, surface.agent);
    if (surface.kind === "terminal") return terminalStop(task.id, surface.agent);
    return Promise.resolve();
  };

  /** Chat and terminal are exclusive: stop the running one before the other takes over. */
  const switchSurface = async (to: TaskSurface) => {
    setSwitching(true);
    setCloseError(null);
    try {
      await stopSurface();
      onSwitchSurface(to);
    } catch (error) {
      setCloseError(String(error));
    } finally {
      setSwitching(false);
    }
  };

  const closeEditor = async () => {
    setClosing(true);
    setCloseError(null);
    try {
      await stopSurface();
      onBack();
    } catch (error) {
      setCloseError(String(error));
    } finally {
      setClosing(false);
    }
  };

  useKeyboardShortcuts({
    "meta+[": { handler: onBack, inTextFields: true },
    Escape: { handler: onBack },
  });

  return (
    <div className="flex-1 flex flex-col min-h-0 min-w-0">
      <TaskHeader
        task={task}
        linearInfo={linearInfo}
        gitStatus={gitStatus}
        sessionStatus={sessionStatus}
        sidebarCollapsed={sidebarCollapsed}
        onExpandSidebar={onExpandSidebar}
        onBack={onBack}
        onCloseEditor={surface.kind !== "external" ? () => void closeEditor() : undefined}
        closingEditor={closing}
        surfaceSwitcher={
          <AgentSurfaceSwitcher
            surface={surface}
            disabled={switching}
            onChange={(to) => void switchSurface(to)}
          />
        }
      />
      <TaskSetupProgress taskId={task.id} />
      {closeError && (
        <p role="alert" className="px-4 py-2 text-sm text-danger">
          {closeError}
        </p>
      )}
      <TaskSurfaceView task={task} surface={surface} onStatusChange={setSessionStatus} />
    </div>
  );
}

function TaskSurfaceView({
  task,
  surface,
  onStatusChange,
}: {
  task: Task;
  surface: TaskSurface;
  onStatusChange: (status: SessionStatus) => void;
}) {
  switch (surface.kind) {
    case "editor":
      return (
        <EditorPane
          taskId={task.id}
          folders={task.members.map((member) => member.path)}
          onStatusChange={onStatusChange}
        />
      );
    case "terminal":
      return (
        <TerminalPane
          taskId={task.id}
          agent={surface.agent}
          folders={task.members.map((m) => m.path)}
          branchName={task.branchName}
          onStatusChange={onStatusChange}
        />
      );
    case "chat":
      return (
        <ChatPane
          key={surface.agent}
          taskId={task.id}
          agent={surface.agent}
          folders={task.members.map((m) => m.path)}
          onStatusChange={onStatusChange}
        />
      );
    default:
      return (
        <div className="flex-1 flex items-center justify-center text-sm text-text-muted">
          This task opens in an external editor.
        </div>
      );
  }
}
