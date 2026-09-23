import { useState } from "react";
import { TaskSetupProgress } from "./TaskSetupProgress";
import { TaskHeader } from "./TaskHeader";
import { TerminalPane } from "../terminal/TerminalPane";
import { ChatPane } from "../chat/ChatPane";
import { AgentSurfaceSwitcher } from "./AgentSurfaceSwitcher";
import { EditorPane } from "../editor/EditorPane";
import { editorClose } from "../../services/codeEditor";
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
}

/** A task opened inside the app: compact header on top, the task's surface filling the rest. */
export function TaskView({
  task,
  surface: openedSurface,
  linearInfo,
  gitStatus,
  sidebarCollapsed,
  onExpandSidebar,
  onBack,
}: TaskViewProps) {
  const [picked, setPicked] = useState<{ from: TaskSurface; to: TaskSurface } | null>(null);
  const surface = picked?.from === openedSurface ? picked.to : openedSurface;
  const [sessionStatus, setSessionStatus] = useState<SessionStatus>({ kind: "connecting" });
  const [closeError, setCloseError] = useState<string | null>(null);
  const [closing, setClosing] = useState(false);

  const closeEditor = async () => {
    setClosing(true);
    setCloseError(null);
    try {
      await editorClose(task.id);
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
        onCloseEditor={surface.kind === "editor" ? () => void closeEditor() : undefined}
        closingEditor={closing}
        surfaceSwitcher={
          <AgentSurfaceSwitcher
            surface={surface}
            onChange={(to) => setPicked({ from: openedSurface, to })}
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
