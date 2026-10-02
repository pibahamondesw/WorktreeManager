import { useEffect, useRef } from "react";
import { Modal } from "../ui/Modal";
import { BellIcon, NotebookIcon, SunIcon, WrenchIcon } from "../ui/Icons";
import { ThemeSettings } from "../ui/ThemePicker";
import { VaultSettings } from "../sidebar/VaultSettingsModal";
import { AgentAlertSettingsPanel } from "../sidebar/AgentAlertsModal";
import { DependencySettings } from "../doctor/DoctorModal";
import { VaultAgent } from "../../services/vault";
import { VaultConfig } from "../../types";
import { AgentAlertSettings } from "../../services/agentActivity";
import { CheckSeverity, DoctorReport } from "../../services/doctor";

export type SettingsSection = "theme" | "vault" | "dependencies" | "alerts";

const SECTIONS = [
  { id: "theme", label: "Theme", icon: SunIcon },
  { id: "vault", label: "Obsidian vault", icon: NotebookIcon },
  { id: "dependencies", label: "Dependencies", icon: WrenchIcon },
  { id: "alerts", label: "Agent alerts", icon: BellIcon },
] as const;

interface SettingsModalProps {
  section: SettingsSection | null;
  onSectionChange: (section: SettingsSection) => void;
  onClose: () => void;
  themeId: string;
  onThemeChange: (id: string) => void;
  customColors: Record<string, string> | null;
  onCustomColorsChange: (colors: Record<string, string>) => void;
  vault: VaultConfig;
  onVaultChange: (vault: VaultConfig) => void | Promise<void>;
  onRepairVaultAgents: (agent?: VaultAgent) => Promise<void>;
  alerts: AgentAlertSettings;
  onAlertsChange: (alerts: AgentAlertSettings) => void;
  report: DoctorReport | null;
  running: boolean;
  severity: CheckSeverity | null;
  onRecheck: () => void;
}

export function SettingsModal(props: SettingsModalProps) {
  const { section, onSectionChange, onClose } = props;
  const contentRef = useRef<HTMLDivElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const open = section !== null;

  useEffect(() => {
    if (panelRef.current) panelRef.current.scrollTop = 0;
  }, [section]);

  useEffect(() => {
    if (!open) return;
    const previous = document.activeElement;
    contentRef.current?.querySelector<HTMLButtonElement>('[aria-selected="true"]')?.focus();
    return () => {
      if (previous instanceof HTMLElement && previous.isConnected) previous.focus();
    };
  }, [open]);

  return (
    <Modal
      open={open}
      onClose={onClose}
      title="Settings"
      size="settings"
      onKeyDown={(event) => {
        if (event.key !== "Tab") return;
        const dialog = event.currentTarget;
        const controls = Array.from(
          dialog?.querySelectorAll<HTMLElement>(
            'button, input, select, textarea, a[href], [tabindex="0"]'
          ) ?? []
        ).filter(
          (element) =>
            !element.closest("[hidden]") && !element.matches(":disabled") && element.tabIndex >= 0
        );
        const first = controls[0];
        const last = controls[controls.length - 1];
        if (event.shiftKey && document.activeElement === first) {
          event.preventDefault();
          last?.focus();
        } else if (!event.shiftKey && document.activeElement === last) {
          event.preventDefault();
          first?.focus();
        }
      }}
    >
      {open && (
        <div ref={contentRef} className="flex h-full min-h-0">
          <div
            role="tablist"
            aria-label="Settings sections"
            aria-orientation="vertical"
            className="w-44 shrink-0 border-r border-border p-3 space-y-1 bg-bg-tertiary/30"
          >
            {SECTIONS.map(({ id, label, icon: Icon }, index) => (
              <button
                key={id}
                id={`settings-tab-${id}`}
                role="tab"
                aria-selected={section === id}
                aria-controls={`settings-panel-${id}`}
                tabIndex={section === id ? 0 : -1}
                onClick={() => onSectionChange(id)}
                onKeyDown={(event) => {
                  let next: number;
                  if (event.key === "ArrowDown") next = (index + 1) % SECTIONS.length;
                  else if (event.key === "ArrowUp")
                    next = (index + SECTIONS.length - 1) % SECTIONS.length;
                  else if (event.key === "Home") next = 0;
                  else if (event.key === "End") next = SECTIONS.length - 1;
                  else return;
                  event.preventDefault();
                  onSectionChange(SECTIONS[next].id);
                  document.getElementById(`settings-tab-${SECTIONS[next].id}`)?.focus();
                }}
                className={`w-full flex items-center gap-2 px-3 py-2 rounded-lg text-xs text-left transition-colors cursor-pointer focus-visible:outline-accent ${section === id ? "bg-bg-active text-text-primary" : "text-text-secondary hover:bg-bg-hover hover:text-text-primary"}`}
              >
                <Icon size={14} />
                {label}
                {id === "dependencies" && props.severity && props.severity !== "ok" && (
                  <span
                    aria-label="Dependencies need attention"
                    className={`ml-auto w-1.5 h-1.5 shrink-0 rounded-full ${props.severity === "error" ? "bg-danger" : "bg-warning"}`}
                  />
                )}
              </button>
            ))}
          </div>
          <div ref={panelRef} className="flex-1 min-w-0 min-h-0 overflow-y-auto">
            {SECTIONS.map(({ id, label }) => (
              <section
                key={id}
                id={`settings-panel-${id}`}
                role="tabpanel"
                aria-labelledby={`settings-tab-${id}`}
                hidden={section !== id}
                tabIndex={0}
              >
                <h3 className="px-6 pt-5 text-sm font-semibold text-text-primary">{label}</h3>
                {id === "theme" && (
                  <ThemeSettings
                    currentThemeId={props.themeId}
                    onThemeChange={props.onThemeChange}
                    customColors={props.customColors}
                    onCustomColorsChange={props.onCustomColorsChange}
                  />
                )}
                {id === "vault" && (
                  <VaultSettings
                    vault={props.vault}
                    onVaultChange={props.onVaultChange}
                    onRepairAgents={props.onRepairVaultAgents}
                    onClose={onClose}
                  />
                )}
                {id === "dependencies" && (
                  <DependencySettings
                    open={section === id}
                    report={props.report}
                    running={props.running}
                    onRecheck={props.onRecheck}
                    onRepairVaultAgents={props.onRepairVaultAgents}
                  />
                )}
                {id === "alerts" && (
                  <AgentAlertSettingsPanel
                    open={section === id}
                    alerts={props.alerts}
                    onAlertsChange={props.onAlertsChange}
                  />
                )}
              </section>
            ))}
          </div>
        </div>
      )}
    </Modal>
  );
}
