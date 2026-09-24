import { AgentView, TaskSurface } from "../../types";

interface AgentSurfaceSwitcherProps {
  surface: TaskSurface;
  disabled?: boolean;
  onChange: (surface: TaskSurface) => void;
}

const VIEWS: { id: AgentView; label: string }[] = [
  { id: "chat", label: "Chat" },
  { id: "terminal", label: "Terminal" },
];

/** Switches the open task's agent between its chat and terminal surfaces. */
export function AgentSurfaceSwitcher({ surface, disabled, onChange }: AgentSurfaceSwitcherProps) {
  if (surface.kind !== "chat" && surface.kind !== "terminal") return null;
  const { kind: current, agent } = surface;
  return (
    <div role="group" aria-label="View" className="mr-3 flex rounded-md border border-border p-0.5">
      {VIEWS.map((view) => (
        <button
          key={view.id}
          type="button"
          aria-pressed={view.id === current}
          disabled={disabled}
          onClick={() => view.id !== current && onChange({ kind: view.id, agent })}
          className={`h-5 px-2 rounded text-[0.6875rem] cursor-pointer transition-colors disabled:cursor-wait ${
            view.id === current
              ? "bg-bg-tertiary text-text-primary"
              : "text-text-muted hover:text-text-primary"
          }`}
        >
          {view.label}
        </button>
      ))}
    </div>
  );
}
