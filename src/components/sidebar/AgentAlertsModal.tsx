import { useEffect, useState } from "react";
import { Button } from "../ui/Button";
import { AgentId } from "../../types";
import { AGENT_IDS, AGENTS } from "../../services/agents";
import {
  AgentAlertSettings,
  AgentHooksStatus,
  AlertCategory,
  HooksState,
  SYSTEM_SOUNDS,
  agentHooksStatus,
  installAgentHooks,
  previewSound,
  removeAgentHooks,
} from "../../services/agentActivity";

interface AgentAlertSettingsPanelProps {
  open: boolean;
  alerts: AgentAlertSettings;
  onAlertsChange: (alerts: AgentAlertSettings) => void;
}

const ACTION_LABELS: Record<HooksState, string> = {
  missing: "Install hooks",
  outdated: "Update hooks",
  installed: "Remove hooks",
};

const CATEGORIES: { id: AlertCategory; label: string }[] = [
  { id: "waiting", label: "Needs your input" },
  { id: "done", label: "Finished" },
];

export function AgentAlertSettingsPanel({
  open,
  alerts,
  onAlertsChange,
}: AgentAlertSettingsPanelProps) {
  const [hooks, setHooks] = useState<AgentHooksStatus | null>(null);
  const [busy, setBusy] = useState<AgentId | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setError(null);
    void agentHooksStatus()
      .then(setHooks)
      .catch((e) => setError(String(e)));
  }, [open]);

  const toggleHooks = async (agent: AgentId) => {
    setBusy(agent);
    setError(null);
    try {
      await (hooks?.[agent] === "installed" ? removeAgentHooks(agent) : installAgentHooks(agent));
      setHooks(await agentHooksStatus());
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const setSound = (category: AlertCategory, sound: string | null) => {
    onAlertsChange({ ...alerts, sounds: { ...alerts.sounds, [category]: sound } });
    if (sound) void previewSound(sound).catch(() => undefined);
  };

  const setNotification = (category: AlertCategory, enabled: boolean) =>
    onAlertsChange({
      ...alerts,
      notifications: { ...alerts.notifications, [category]: enabled },
    });

  return (
    <div className="p-6 space-y-5">
      <div className="space-y-2">
        <p className="text-sm text-text-secondary">
          Global hooks tell WorktreeManager when an agent starts working, needs your input or
          finishes in embedded terminals and editors, Cursor and external terminals. Embedded chats
          report it on their own. Subagent activity is ignored. Alerts require WorktreeManager to be
          running. Your other hooks are kept as they are.
        </p>
        {AGENT_IDS.map((id) => (
          <div
            key={id}
            className="flex items-center justify-between gap-3 rounded-lg bg-bg-tertiary border border-border px-3 py-2"
          >
            <div className="min-w-0">
              <p className="text-sm text-text-primary">{AGENTS[id].productName}</p>
              <p className="text-xs font-mono text-text-muted truncate select-text">
                {AGENTS[id].alertHooksConfig}
              </p>
              {AGENTS[id].alertHooksTrust && hooks?.[id] !== "missing" && (
                <p className="text-xs text-text-muted select-text">{AGENTS[id].alertHooksTrust}</p>
              )}
            </div>
            <Button
              variant={hooks?.[id] === "installed" ? "ghost" : "primary"}
              disabled={!hooks || busy !== null}
              loading={busy === id}
              onClick={() => void toggleHooks(id)}
            >
              {ACTION_LABELS[hooks?.[id] ?? "missing"]}
            </Button>
          </div>
        ))}
        {error && <p className="text-sm text-danger select-text">{error}</p>}
      </div>

      <div className="space-y-2">
        {CATEGORIES.map((category) => (
          <div key={category.id} className="flex items-center gap-3">
            <span className="text-sm text-text-primary flex-1">{category.label}</span>
            <select
              value={alerts.sounds[category.id] ?? ""}
              onChange={(e) => setSound(category.id, e.target.value || null)}
              className="bg-bg-tertiary border border-border rounded-md px-2 py-1 text-xs text-text-primary cursor-pointer"
              aria-label={`${category.label} sound`}
            >
              <option value="">No sound</option>
              {SYSTEM_SOUNDS.map((sound) => (
                <option key={sound} value={sound}>
                  {sound}
                </option>
              ))}
            </select>
            <label className="flex items-center gap-1.5 text-xs text-text-secondary cursor-pointer">
              <input
                type="checkbox"
                checked={alerts.notifications[category.id]}
                onChange={(e) => setNotification(category.id, e.target.checked)}
              />
              Notify
            </label>
          </div>
        ))}
        <p className="text-xs text-text-muted">
          Notifications appear only while WorktreeManager is in the background; clicking one opens
          its task.
        </p>
      </div>
    </div>
  );
}
