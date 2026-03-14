import { useState } from "react";
import { Button } from "../ui/Button";
import { TokenInput } from "./TokenInput";
import { WrenchIcon } from "../ui/Icons";
import { validateLinearToken } from "../../services/linear";
import { AppState } from "../../types";

interface SetupWizardProps {
  initialSetup: AppState["setup"];
  onComplete: (setup: AppState["setup"]) => void;
}

export function SetupWizard({ initialSetup, onComplete }: SetupWizardProps) {
  const [linearKey, setLinearKey] = useState(initialSetup.linearApiKey ?? "");
  const [linearValid, setLinearValid] = useState(false);
  const [linearUser, setLinearUser] = useState<string | null>(null);

  const handleFinish = () => {
    onComplete({
      linearApiKey: linearKey,
      isComplete: true,
    });
  };

  return (
    <div className="flex items-center justify-center h-full" data-tauri-drag-region>
      <div className="w-[420px] flex flex-col items-center gap-8">
        {/* Logo */}
        <div className="flex flex-col items-center gap-3">
          <div className="w-16 h-16 rounded-2xl bg-accent/20 flex items-center justify-center">
            <WrenchIcon className="text-accent" />
          </div>
          <h1 className="text-xl font-semibold text-text-primary">WorktreeManager</h1>
          <p className="text-sm text-text-secondary text-center">
            Connect your Linear account to get started
          </p>
        </div>

        {/* Token input */}
        <div className="w-full space-y-6">
          <TokenInput
            title="Connect Linear"
            description="Enter your Linear API key to search issues and manage worktrees."
            linkUrl="https://linear.app/settings/api"
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
        </div>

        {/* Finish */}
        <div className="flex items-center justify-end w-full">
          <Button onClick={handleFinish} disabled={!linearValid}>
            Get Started
          </Button>
        </div>
      </div>
    </div>
  );
}
