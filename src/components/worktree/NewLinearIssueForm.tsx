import { Dispatch, SetStateAction, useEffect, useId, useState } from "react";
import { LinearService } from "../../services/linear";
import { LinearIssue, LinearTeam } from "../../types";
import { Input } from "../ui/Input";

export interface NewLinearIssueDraft {
  title: string;
  description: string;
  teamId: string;
}

interface NewLinearIssueFormProps {
  linear: LinearService;
  value: NewLinearIssueDraft;
  onChange: Dispatch<SetStateAction<NewLinearIssueDraft>>;
  disabled: boolean;
  createdIssue: LinearIssue | null;
  onSubmit: () => void;
}

export function NewLinearIssueForm({
  linear,
  value,
  onChange,
  disabled,
  createdIssue,
  onSubmit,
}: NewLinearIssueFormProps) {
  const [teams, setTeams] = useState<LinearTeam[]>([]);
  const [error, setError] = useState<string | null>(null);
  const id = useId();
  const fieldsDisabled = disabled || !!createdIssue;

  useEffect(() => {
    let cancelled = false;
    linear
      .listTeams()
      .then((loaded) => {
        if (cancelled) return;
        setTeams(loaded);
        onChange((current) => ({
          ...current,
          teamId: loaded.some((team) => team.id === current.teamId)
            ? current.teamId
            : loaded[0]?.id || "",
        }));
      })
      .catch(() => {
        if (!cancelled) setError("Could not load Linear teams");
      });
    return () => {
      cancelled = true;
    };
  }, [linear, onChange]);

  return (
    <div className="flex flex-col gap-3">
      <Input
        label="Issue title"
        aria-label="Issue title"
        placeholder="What needs to be done?"
        disabled={fieldsDisabled}
        value={value.title}
        onChange={(e) => onChange({ ...value, title: e.target.value })}
        autoFocus
        onKeyDown={(e) => {
          if (e.key === "Enter" && !disabled && value.title.trim() && value.teamId) onSubmit();
        }}
      />
      <div className="flex flex-col gap-1.5">
        <label htmlFor={`${id}-team`} className="text-sm font-medium text-text-secondary">
          Team
        </label>
        <select
          id={`${id}-team`}
          className="w-full rounded-lg border border-border bg-bg-tertiary px-3 py-2 text-sm text-text-primary outline-none focus:border-accent transition-colors"
          disabled={fieldsDisabled || teams.length === 0}
          value={value.teamId}
          onChange={(e) => onChange({ ...value, teamId: e.target.value })}
        >
          {teams.map((team) => (
            <option key={team.id} value={team.id}>
              {team.name} ({team.key})
            </option>
          ))}
        </select>
      </div>
      <div className="flex flex-col gap-1.5">
        <label htmlFor={`${id}-description`} className="text-sm font-medium text-text-secondary">
          Description (optional)
        </label>
        <textarea
          id={`${id}-description`}
          className="w-full rounded-lg border border-border bg-bg-tertiary px-3 py-2 text-sm text-text-primary placeholder:text-text-muted outline-none focus:border-accent transition-colors resize-none"
          rows={3}
          disabled={fieldsDisabled}
          value={value.description}
          onChange={(e) => onChange({ ...value, description: e.target.value })}
        />
      </div>
      {error && (
        <p role="alert" className="text-sm text-danger select-text">
          {error}
        </p>
      )}
      <p className="text-xs text-text-muted select-text">
        {createdIssue
          ? `Issue ${createdIssue.identifier} was created. Retrying will reuse it for the task.`
          : "The issue is created and assigned to you, and its branch name is used for the task."}
      </p>
    </div>
  );
}
