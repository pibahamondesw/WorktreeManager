import { useEffect, useState } from "react";
import { Modal } from "../ui/Modal";
import { Button } from "../ui/Button";
import { ChatSetting, HookControls, HookOption } from "../../services/chat";

interface HooksDialogProps {
  hooks: HookControls | undefined;
  onConfigure: (setting: ChatSetting) => Promise<void>;
  onClose: () => void;
}

export function HooksDialog({ hooks, onConfigure, onClose }: HooksDialogProps) {
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const run = async (setting: ChatSetting) => {
    setError(null);
    setPending(true);
    try {
      await onConfigure(setting);
    } catch (e) {
      setError(String(e));
    } finally {
      setPending(false);
    }
  };

  useEffect(() => {
    void onConfigure({ kind: "hooks" }).catch((e) => setError(String(e)));
  }, [onConfigure]);

  const loading = pending || hooks?.loading;
  const update = (hook: HookOption, change: { enabled?: boolean; trust?: boolean }) =>
    void run({ kind: "hook", value: { key: hook.key, hash: hook.currentHash, ...change } });

  return (
    <Modal open onClose={onClose} title="Codex hooks" wide>
      <div className="px-6 py-5 space-y-4">
        <p className="text-sm text-text-secondary">
          Review each definition before trusting it. Changes apply to Codex on this machine.
        </p>
        <Button
          variant="secondary"
          disabled={Boolean(loading)}
          onClick={() => void run({ kind: "hooks" })}
        >
          Refresh
        </Button>
        {(error || hooks?.error) && (
          <p role="alert" className="text-sm text-danger whitespace-pre-wrap select-text">
            {error || hooks?.error}
          </p>
        )}
        {hooks?.warnings.map((warning, index) => (
          <p key={index} className="text-sm text-text-secondary select-text">
            {warning}
          </p>
        ))}
        {loading || (!hooks && !error) ? (
          <p role="status" className="text-sm text-text-muted">
            Loading hooks…
          </p>
        ) : hooks && hooks.items.length === 0 && !hooks.error && !error ? (
          <p className="text-sm text-text-muted">No hooks configured for this worktree.</p>
        ) : null}
        {hooks?.items.map((hook) => {
          const trusted = hook.trustStatus === "trusted" || hook.isManaged;
          const reviewable = hook.handlerType === "command" && Boolean(hook.command);
          return (
            <section
              key={hook.key}
              aria-label={`${hook.eventName} hook`}
              className="rounded-lg border border-border p-3 space-y-2"
            >
              <div className="flex flex-wrap items-center gap-2 text-sm">
                <span className="text-text-primary">{hook.eventName}</span>
                <span className="text-text-muted">
                  {hook.isManaged ? "Managed" : hook.trustStatus} ·{" "}
                  {hook.enabled ? "Enabled" : "Disabled"}
                </span>
              </div>
              <p className="text-xs text-text-secondary break-all select-text">{hook.sourcePath}</p>
              {hook.matcher && (
                <p className="text-xs text-text-muted select-text">Matcher: {hook.matcher}</p>
              )}
              <pre className="text-xs text-text-primary whitespace-pre-wrap break-all select-text">
                {hook.command ??
                  (hook.server && hook.tool ? `${hook.server} / ${hook.tool}` : hook.handlerType)}
              </pre>
              {!hook.isManaged && (
                <div className="flex flex-wrap gap-2">
                  {!trusted && reviewable && (
                    <Button
                      disabled={Boolean(loading)}
                      onClick={() => update(hook, { trust: true, enabled: true })}
                    >
                      Trust and enable
                    </Button>
                  )}
                  {(trusted || hook.enabled) && (
                    <Button
                      variant="secondary"
                      disabled={Boolean(loading)}
                      onClick={() => update(hook, { enabled: !hook.enabled })}
                    >
                      {hook.enabled ? "Disable" : "Enable"}
                    </Button>
                  )}
                  {!trusted && !reviewable && (
                    <p className="text-xs text-text-muted">Review this definition in Codex CLI.</p>
                  )}
                </div>
              )}
            </section>
          );
        })}
      </div>
    </Modal>
  );
}
