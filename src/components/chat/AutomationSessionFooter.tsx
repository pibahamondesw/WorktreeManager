import { useState } from "react";
import { Button } from "../ui/Button";
import { AutomationSessionSource } from "../../services/automationSessions";

interface AutomationSessionFooterProps {
  /** Null while the runs have not shown which automation the session belongs to. */
  source: AutomationSessionSource | null;
  /** Present when its item can be taken over, as in the Pipeline. */
  onTakeOver?: () => Promise<void>;
}

/** Takes the composer's place in an automation's conversation, which only the automation drives. */
export function AutomationSessionFooter({ source, onTakeOver }: AutomationSessionFooterProps) {
  const [taking, setTaking] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const owner = source ? `the ${source.automation} automation` : "an automation";

  const takeOver = async (perform: () => Promise<void>) => {
    setTaking(true);
    setError(null);
    try {
      await perform();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setTaking(false);
    }
  };

  return (
    <div className="max-w-3xl mx-auto flex flex-col gap-1.5 text-xs">
      <div className="flex items-center gap-3">
        <p role="note" className="flex-1 text-text-muted select-text">
          This conversation belongs to {owner}. Closing the tab does not stop it.
        </p>
        {onTakeOver && (
          <Button
            variant="secondary"
            className="h-6 px-2 text-xs"
            loading={taking}
            disabled={taking}
            title="Stop the engine on this step and continue its conversation yourself"
            onClick={() => void takeOver(onTakeOver)}
          >
            Take over
          </Button>
        )}
      </div>
      {error && <p className="text-danger break-words select-text">{error}</p>}
    </div>
  );
}
