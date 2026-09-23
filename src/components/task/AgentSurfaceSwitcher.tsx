import { surfaceAgent } from "../../embedded/taskSurface";
import { TaskSurface } from "../../types";

interface AgentSurfaceSwitcherProps {
  surface: TaskSurface;
  onChange: (surface: TaskSurface) => void;
}

type Mode = "chat" | "terminal";

const MODES: { id: Mode; label: string }[] = [
  { id: "chat", label: "Chat" },
  { id: "terminal", label: "Terminal" },
];

function Segmented<T extends string>({
  label,
  options,
  value,
  onSelect,
}: {
  label: string;
  options: { id: T; label: string }[];
  value: T;
  onSelect: (id: T) => void;
}) {
  return (
    <div role="group" aria-label={label} className="flex rounded-md border border-border p-0.5">
      {options.map((option) => (
        <button
          key={option.id}
          type="button"
          aria-pressed={option.id === value}
          onClick={() => option.id !== value && onSelect(option.id)}
          className={`h-5 px-2 rounded text-[0.6875rem] cursor-pointer transition-colors ${
            option.id === value
              ? "bg-bg-tertiary text-text-primary"
              : "text-text-muted hover:text-text-primary"
          }`}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

/** Switches the open task's agent between its chat and terminal surfaces. */
export function AgentSurfaceSwitcher({ surface, onChange }: AgentSurfaceSwitcherProps) {
  const agent = surfaceAgent(surface);
  if (!agent || (surface.kind !== "chat" && surface.kind !== "terminal")) return null;
  const mode: Mode = surface.kind;
  return (
    <span className="mr-3 flex items-center">
      <Segmented
        label="View"
        options={MODES}
        value={mode}
        onSelect={(next) => onChange({ kind: next, agent })}
      />
    </span>
  );
}
