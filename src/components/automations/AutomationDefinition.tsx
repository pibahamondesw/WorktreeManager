import { useState } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { openEditorForWorktree } from "../../services/openEditor";
import { AutomationEntry, DefinitionIssue } from "../../services/automations";
import { EDITOR_APPS, EditorApp } from "../../types";
import { Button } from "../ui/Button";

/** Editors that run outside the app and accept a single file. */
const FILE_EDITORS: EditorApp[] = ["cursor", "zed"];

interface AutomationDefinitionProps {
  entry: AutomationEntry;
  editorApp: EditorApp;
}

export function AutomationDefinition({ entry, editorApp }: AutomationDefinitionProps) {
  const [error, setError] = useState<string | null>(null);
  const editor = FILE_EDITORS.includes(editorApp)
    ? EDITOR_APPS.find((app) => app.id === editorApp)
    : undefined;

  const openInEditor = () => {
    if (!editor) return;
    setError(null);
    void openEditorForWorktree(editor.id, [entry.path], `automation-${entry.name}`, null, {
      onError: setError,
    });
  };
  const reveal = () => {
    setError(null);
    revealItemInDir(entry.path).catch(() => setError("Could not reveal the file in Finder"));
  };

  return (
    <div className="space-y-4">
      <p className="text-xs font-mono text-text-secondary break-all select-text">{entry.path}</p>
      <div className="flex gap-2">
        {editor && (
          <Button className="h-8 text-xs" onClick={openInEditor}>
            Open in {editor.label}
          </Button>
        )}
        <Button variant="secondary" className="h-8 text-xs" onClick={reveal}>
          Reveal in Finder
        </Button>
      </div>
      {error && <p className="text-sm text-danger break-words select-text">{error}</p>}
      <IssueList title="Errors" issues={entry.errors} tone="text-danger" />
      <IssueList title="Warnings" issues={entry.warnings} tone="text-warning" />
    </div>
  );
}

function IssueList({
  title,
  issues,
  tone,
}: {
  title: string;
  issues: DefinitionIssue[];
  tone: string;
}) {
  if (issues.length === 0) return null;
  return (
    <section className="space-y-1">
      <h3 className={`text-xs font-semibold ${tone}`}>{title}</h3>
      <ul className="space-y-1 text-xs select-text">
        {issues.map((issue, index) => (
          <li key={index} className="text-text-secondary break-words">
            {issue.field && <span className="font-mono text-text-primary">{issue.field}: </span>}
            {issue.message}
          </li>
        ))}
      </ul>
    </section>
  );
}
