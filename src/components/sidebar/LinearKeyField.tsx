import { Button } from "../ui/Button";
import { SuccessCircleIcon } from "../ui/Icons";

interface LinearKeyFieldProps {
  linearKey: string;
  linearValid: boolean;
  linearUser: string | null;
  linearValidating: boolean;
  linearError: string | null;
  onChange: (key: string) => void;
  onValidate: () => void;
  label?: string;
  autoFocus?: boolean;
}

/** Linear API-key input with a Validate button and connection status. */
export function LinearKeyField({
  linearKey,
  linearValid,
  linearUser,
  linearValidating,
  linearError,
  onChange,
  onValidate,
  label = "Linear API key (optional)",
  autoFocus,
}: LinearKeyFieldProps) {
  return (
    <div className="flex flex-col gap-1.5">
      <label className="text-sm font-medium text-text-secondary">{label}</label>
      <div className="flex gap-2">
        <input
          className={`flex-1 rounded-lg border bg-bg-tertiary px-3 py-2 text-sm text-text-primary placeholder:text-text-muted outline-none transition-colors focus:border-accent ${
            linearError ? "border-danger" : "border-border"
          }`}
          placeholder="lin_api_..."
          value={linearKey}
          onChange={(e) => onChange(e.target.value)}
          type="password"
          autoFocus={autoFocus}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              onValidate();
            }
          }}
        />
        <Button
          variant="secondary"
          onClick={onValidate}
          loading={linearValidating}
          disabled={!linearKey.trim() || linearValid}
        >
          {linearValid ? "Valid" : "Validate"}
        </Button>
      </div>
      {linearValid && linearUser && (
        <div className="flex items-center gap-1.5 text-xs text-success">
          <SuccessCircleIcon size={12} />
          Connected as {linearUser}
        </div>
      )}
      {linearError && <p className="text-xs text-danger">{linearError}</p>}
      <p className="text-xs text-text-muted">
        Connect Linear to search issues when creating worktrees
      </p>
    </div>
  );
}
