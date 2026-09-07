import { useState } from "react";
import { TaskHeader } from "./TaskHeader";
import { TerminalPane } from "../terminal/TerminalPane";
import { useKeyboardShortcuts } from "../../hooks/useKeyboardShortcuts";
import { SessionStatus } from "../../hooks/useTerminalSession";
import { Task, TaskSurface } from "../../types";

interface TaskViewProps {
  task: Task;
  surface: TaskSurface;
  sidebarCollapsed: boolean;
  onExpandSidebar: () => void;
  onBack: () => void;
}

/** A task opened inside the app: compact header on top, the task's surface filling the rest. */
export function TaskView({ task, surface, sidebarCollapsed, onExpandSidebar, onBack }: TaskViewProps) {
  const [sessionStatus, setSessionStatus] = useState<SessionStatus>({ kind: "connecting" });

  useKeyboardShortcuts({
    "meta+[": { handler: onBack, inTextFields: true },
    Escape: { handler: onBack },
  });

  return (
    <div className="flex-1 flex flex-col min-h-0 min-w-0">
      <TaskHeader
        task={task}
        sessionStatus={sessionStatus}
        sidebarCollapsed={sidebarCollapsed}
        onExpandSidebar={onExpandSidebar}
        onBack={onBack}
      />
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
    default:
      return (
        <div className="flex-1 flex items-center justify-center text-sm text-text-muted">
          This task opens in an external editor.
        </div>
      );
  }
}
