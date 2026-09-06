import { useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Modal } from "../ui/Modal";
import { Button } from "../ui/Button";
import { Badge } from "../ui/Badge";
import { CopyIcon, ExternalLinkIcon, SpinnerIcon } from "../ui/Icons";
import { CheckSeverity, CheckStatus, DependencyCheck, DoctorReport } from "../../services/doctor";

interface DoctorModalProps {
  open: boolean;
  onClose: () => void;
  report: DoctorReport | null;
  running: boolean;
  onRecheck: () => void;
}

const STATUS_LABELS: Record<CheckStatus, string> = {
  ok: "OK",
  missing: "Not installed",
  broken: "Not working",
  unknown: "Not checked",
};

const BADGE_VARIANTS: Record<CheckSeverity, "success" | "warning" | "danger"> = {
  ok: "success",
  warning: "warning",
  error: "danger",
};

const DOT_COLORS: Record<CheckSeverity, string> = {
  ok: "bg-success",
  warning: "bg-warning",
  error: "bg-danger",
};

/**
 * Per-dependency health, with the command that installs whatever is missing. Advisory only —
 * the app works (or fails) exactly the same whether this is open or not.
 */
export function DoctorModal({ open, onClose, report, running, onRecheck }: DoctorModalProps) {
  const [copiedId, setCopiedId] = useState<string | null>(null);

  const copy = (id: string, command: string) => {
    void navigator.clipboard.writeText(command);
    setCopiedId(id);
    setTimeout(() => setCopiedId((current) => (current === id ? null : current)), 1500);
  };

  return (
    <Modal open={open} onClose={onClose} title="Dependencies" wide>
      <div className="px-6 py-4 border-b border-border flex items-center justify-between gap-4">
        <p className="text-xs text-text-muted">{summary(report, running)}</p>
        <Button variant="secondary" onClick={onRecheck} disabled={running} loading={running}>
          {running ? "Checking…" : "Re-check"}
        </Button>
      </div>

      {!report ? (
        <div className="px-6 py-10 flex items-center justify-center">
          <SpinnerIcon size={20} className="text-text-muted" />
        </div>
      ) : (
        <div>
          {report.checks.map((check) => (
            <div
              key={check.id}
              className="px-6 py-3 border-b border-border last:border-b-0 flex gap-3"
            >
              <span
                className={`mt-1.5 w-2 h-2 rounded-full flex-shrink-0 ${DOT_COLORS[check.severity]}`}
              />
              <div className="min-w-0 flex-1 space-y-1">
                <div className="flex items-center gap-2 flex-wrap">
                  <span className="text-sm text-text-primary">{check.label}</span>
                  <Badge variant={BADGE_VARIANTS[check.severity]}>
                    {STATUS_LABELS[check.status]}
                  </Badge>
                </div>
                <p className="text-xs text-text-muted">
                  {check.scope === "repository" && "Optional for repositories. "}
                  {check.reason}
                </p>
                {check.detail && (
                  <p className="text-xs text-text-secondary font-mono select-text break-all">
                    {check.detail}
                  </p>
                )}
                {check.severity !== "ok" && (
                  <Remedy check={check} copiedId={copiedId} onCopy={copy} />
                )}
              </div>
            </div>
          ))}
        </div>
      )}

      <div className="px-6 py-4 border-t border-border">
        <p className="text-xs text-text-muted">
          Launched from Finder, the app starts with a minimal PATH, so tools are looked up the same
          way its editor and Claude launchers do — through your login shell profiles. A tool
          installed only in an interactive-shell-specific path may not be found here.
        </p>
      </div>
    </Modal>
  );
}

function Remedy({
  check,
  copiedId,
  onCopy,
}: {
  check: DependencyCheck;
  copiedId: string | null;
  onCopy: (id: string, command: string) => void;
}) {
  if (!check.install && !check.url) return null;

  return (
    <div className="flex items-center gap-2 pt-1">
      {check.install && (
        <button
          onClick={() => onCopy(check.id, check.install!)}
          className="min-w-0 flex items-center gap-2 rounded-lg bg-bg-tertiary border border-border px-2.5 py-1.5 hover:bg-bg-hover transition-colors cursor-pointer"
          title="Copy install command"
        >
          <span className="text-xs font-mono text-text-primary truncate">{check.install}</span>
          <span className="flex items-center gap-1 text-xs text-text-muted flex-shrink-0">
            <CopyIcon /> {copiedId === check.id ? "Copied" : "Copy"}
          </span>
        </button>
      )}
      {check.url && (
        <button
          onClick={() => void openUrl(check.url!).catch(() => undefined)}
          className="flex items-center gap-1 text-xs text-accent hover:underline cursor-pointer flex-shrink-0"
        >
          Docs <ExternalLinkIcon />
        </button>
      )}
    </div>
  );
}

function summary(report: DoctorReport | null, running: boolean): string {
  if (!report) return running ? "Checking your machine…" : "Not checked yet.";
  const suggestions = report.checks.filter(
    (check) => check.scope === "repository" && check.severity !== "ok"
  ).length;
  if (report.errors === 0 && report.warnings === 0 && suggestions === 0) {
    return "Everything this setup needs is installed.";
  }
  const parts = [
    suggestions > 0 ? `${suggestions} optional repo ${plural(suggestions, "suggestion")}` : null,
    report.errors > 0 ? `${report.errors} ${plural(report.errors, "problem")}` : null,
    report.warnings > 0 ? `${report.warnings} ${plural(report.warnings, "note")}` : null,
  ].filter((p): p is string => p !== null);
  return `${parts.join(", ")}. Nothing here blocks the app.`;
}

function plural(count: number, word: string): string {
  return count === 1 ? word : `${word}s`;
}
