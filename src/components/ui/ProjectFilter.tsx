import { Task } from "../../types";
import { NO_PROJECT, taskProjectKey } from "../../search/projects";

export function ProjectFilter({
  tasks,
  value,
  onChange,
}: {
  tasks: Task[];
  value: string;
  onChange: (value: string) => void;
}) {
  const projects = new Map(
    tasks.map((task) => [taskProjectKey(task), task.linearProjectName ?? "No project"])
  );
  return (
    <select
      aria-label="Linear project"
      value={value}
      onKeyDown={(event) => event.stopPropagation()}
      onChange={(event) => onChange(event.target.value)}
      className="max-w-56 rounded-md border border-border bg-bg-tertiary px-2 py-1 text-xs text-text-primary outline-none focus:border-accent"
    >
      <option value="">All projects</option>
      {[...projects]
        .filter(([id]) => id !== NO_PROJECT)
        .sort((a, b) => a[1].localeCompare(b[1]))
        .map(([id, name]) => (
          <option key={id} value={id}>
            {name}
          </option>
        ))}
      <option value={NO_PROJECT}>No project</option>
    </select>
  );
}
