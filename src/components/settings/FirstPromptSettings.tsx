import { FormEvent, useState } from "react";
import { Button } from "../ui/Button";
import { FirstPromptTemplateField } from "./FirstPromptTemplateField";
import { firstPromptProblem } from "../../services/firstPrompt";

const EXAMPLE = `Task: {issue.identifier} {issue.title}
{issue.url}

{issue.description}

Branch: {task.branch}
Repositories:
{repos}
Task log: {note.path}`;

interface FirstPromptSettingsProps {
  template: string;
  onSave: (template: string) => Promise<void>;
}

export function FirstPromptSettings({ template, onSave }: FirstPromptSettingsProps) {
  const [draft, setDraft] = useState(template);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  async function handleSubmit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await onSave(draft);
      setSaved(true);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form onSubmit={(event) => void handleSubmit(event)} className="px-6 pt-3 pb-6 space-y-3">
      <p className="text-xs text-text-muted">
        A new agent conversation in a task starts with this message in the composer, ready to edit.
        Chats opened with wtm task chat or an automation send it ahead of their message. A workspace
        can replace it in its settings. Leave it empty to start conversations blank.
      </p>
      <FirstPromptTemplateField
        label="Template"
        value={draft}
        placeholder={EXAMPLE}
        onChange={(value) => {
          setDraft(value);
          setSaved(false);
        }}
      />
      {error && <p className="text-xs text-danger select-text">{error}</p>}
      <div className="flex items-center gap-2">
        <Button
          type="submit"
          disabled={busy || draft === template || firstPromptProblem(draft) !== null}
        >
          Save template
        </Button>
        {saved && draft === template && <span className="text-xs text-text-muted">Saved</span>}
      </div>
    </form>
  );
}
