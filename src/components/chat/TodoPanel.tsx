import { useState } from "react";
import { ChevronDownIcon } from "../ui/Icons";
import { TodoItem } from "../../services/chat";

const MARK: Record<TodoItem["status"], string> = {
  pending: "○",
  inProgress: "◐",
  completed: "●",
};

/** The agent's current plan, pinned above the composer while any step is unfinished. */
export function TodoPanel({ todos }: { todos: TodoItem[] }) {
  const [open, setOpen] = useState(true);
  const done = todos.filter((todo) => todo.status === "completed").length;
  if (todos.length === 0 || done === todos.length) return null;
  const current = todos.find((todo) => todo.status === "inProgress");
  return (
    <section
      aria-label="Plan"
      className="max-w-3xl mx-auto mb-2 rounded-lg border border-border bg-bg-secondary/60 text-xs"
    >
      <button
        type="button"
        onClick={() => setOpen(!open)}
        aria-expanded={open}
        className="w-full flex items-center gap-2 px-3 py-1.5 cursor-pointer"
      >
        <span className="font-medium text-text-secondary">
          Plan {done}/{todos.length}
        </span>
        {!open && current && (
          <span className="flex-1 min-w-0 truncate text-left text-text-muted">{current.text}</span>
        )}
        <ChevronDownIcon
          className={`ml-auto opacity-50 transition-transform ${open ? "rotate-180" : ""}`}
        />
      </button>
      {open && (
        <ol className="px-3 pb-2 flex flex-col gap-0.5">
          {todos.map((todo, index) => (
            <li
              key={index}
              className={`flex gap-2 select-text ${
                todo.status === "completed"
                  ? "text-text-muted line-through"
                  : todo.status === "inProgress"
                    ? "text-text-primary"
                    : "text-text-secondary"
              }`}
            >
              <span aria-hidden className={todo.status === "inProgress" ? "text-accent" : ""}>
                {MARK[todo.status]}
              </span>
              <span>{todo.text}</span>
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}
