import { useState } from "react";
import { Button } from "../ui/Button";
import { TokenInput } from "./TokenInput";
import { validateLinearToken } from "../../services/linear";
import { AppState } from "../../types";
import logo from "../../assets/logo.png";

interface SetupWizardProps {
  initialSetup: AppState["setup"];
  onComplete: (setup: AppState["setup"], options: { enableVault: boolean }) => void;
}

export function SetupWizard({ initialSetup, onComplete }: SetupWizardProps) {
  const [linearKey, setLinearKey] = useState(initialSetup.linearApiKey ?? "");
  const [linearValid, setLinearValid] = useState(false);
  const [linearUser, setLinearUser] = useState<string | null>(null);
  const [enableVault, setEnableVault] = useState(false);

  const handleFinish = () => {
    onComplete(
      {
        linearApiKey: linearValid ? linearKey : null,
        isComplete: true,
      },
      { enableVault }
    );
  };

  return (
    <div className="flex items-center justify-center h-full" data-drag-region>
      <div className="w-[420px] flex flex-col items-center gap-8">
        {/* Logo */}
        <div className="flex flex-col items-center gap-3">
          <img src={logo} alt="WorktreeManager" className="w-40 h-40 rounded-2xl object-cover" />
          <h1 className="text-xl font-semibold text-text-primary">WorktreeManager</h1>
          <p className="text-sm text-text-secondary text-center">Manage git worktrees with ease</p>
        </div>

        {/* Token input */}
        <div className="w-full space-y-6">
          <TokenInput
            title="Connect Linear"
            description="Enter your Linear API key to search issues and manage worktrees."
            linkUrl="https://linear.app/settings/account/security"
            linkText="Generate API key"
            value={linearKey}
            onChange={setLinearKey}
            onValidate={async (key) => {
              const result = await validateLinearToken(key);
              setLinearValid(result.valid);
              setLinearUser(result.name ?? null);
              return result;
            }}
            isValid={linearValid}
            validatedName={linearUser}
            placeholder="lin_api_..."
          />

          <label className="flex items-start gap-2.5 cursor-pointer">
            <input
              type="checkbox"
              checked={enableVault}
              onChange={(e) => setEnableVault(e.target.checked)}
              className="mt-0.5 accent-accent cursor-pointer"
            />
            <span className="text-sm text-text-secondary">
              Set up an Obsidian vault for task notes
              <span className="block text-xs text-text-muted mt-0.5">
                One note per task, kept in sync by the app. You can enable this later from the
                sidebar.
              </span>
            </span>
          </label>
        </div>

        {/* Finish */}
        <div className="flex items-center justify-between w-full">
          <button
            onClick={handleFinish}
            className="text-sm text-text-muted hover:text-text-primary transition-colors cursor-pointer"
          >
            Skip for now
          </button>
          <Button onClick={handleFinish} disabled={!linearValid}>
            Get Started
          </Button>
        </div>
        {!linearValid && (
          <p className="text-xs text-text-muted text-center -mt-4">
            You can configure Linear per-project later
          </p>
        )}
      </div>
    </div>
  );
}
