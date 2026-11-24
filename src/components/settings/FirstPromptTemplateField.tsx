import { useId } from "react";
import {
  AUTOMATION_VARIABLES,
  TASK_VARIABLES,
  firstPromptProblem,
} from "../../services/firstPrompt";

const textareaClass =
  "w-full rounded-md border border-border bg-bg-secondary px-2 py-1 text-xs text-text-primary outline-none focus:border-accent font-mono resize-y";

const variableList = (names: readonly string[]) => names.map((name) => `{${name}}`).join(" ");

interface FirstPromptTemplateFieldProps {
  label: string;
  value: string;
  placeholder?: string;
  onChange: (value: string) => void;
}

export function FirstPromptTemplateField({
  label,
  value,
  placeholder,
  onChange,
}: FirstPromptTemplateFieldProps) {
  const id = useId();
  const hintId = `${id}-hint`;
  const problem = firstPromptProblem(value);
  return (
    <div className="space-y-1">
      <label htmlFor={id} className="block text-xs text-text-secondary">
        {label}
      </label>
      <textarea
        id={id}
        className={textareaClass}
        rows={6}
        spellCheck={false}
        value={value}
        placeholder={placeholder}
        aria-describedby={hintId}
        aria-invalid={problem !== null}
        onChange={(event) => onChange(event.target.value)}
      />
      <span id={hintId} className="block text-xs text-text-muted select-text">
        Fills a new conversation&apos;s first message. A line whose variables are all empty is left
        out. Variables: {variableList(TASK_VARIABLES)}. Chats an automation opens also fill{" "}
        {variableList(AUTOMATION_VARIABLES)} {"{item.data.<key>}"}.
      </span>
      {problem && <span className="block text-xs text-danger select-text">{problem}</span>}
    </div>
  );
}
