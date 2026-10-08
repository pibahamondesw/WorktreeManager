import { Fragment, useState, type ReactNode } from "react";
import { Button } from "../ui/Button";
import {
  shortcutDisplayValues,
  compactShortcutLabels,
  formatShortcut,
  type Binding,
  type Shortcut,
  type ShortcutOverrides,
} from "../../shortcuts/catalog";

const sections = [
  {
    id: "general",
    label: "General",
    description:
      "Across the main app views. Availability can change while typing or when a dialog is open.",
  },
  { id: "list", label: "Task list", description: "While browsing tasks in a workspace." },
  { id: "task", label: "Task view", description: "While a task is open." },
  {
    id: "palette",
    label: "Search palette",
    description:
      "While searching tasks and commands. These assignments are independent of other views.",
  },
  {
    id: "chat",
    label: "Chat",
    description: "In the message composer, option pickers and approval controls.",
  },
  {
    id: "editor",
    label: "App & embedded editor",
    description: "App navigation that also works when the embedded editor has focus.",
  },
  { id: "modal", label: "Dialogs", description: "Standard navigation inside dialogs and forms." },
] as const;

function sectionFor(entry: Binding): string {
  if (entry.native) return "editor";
  if (entry.contexts.length === 1) return entry.contexts[0];
  if (entry.id.startsWith("task.")) return "task";
  return "general";
}

function compactNumberedEntries(entries: Binding[]): Binding[] {
  const seen = new Set<string>();
  return entries.flatMap((entry) => {
    if (!entry.numberedGroup) return [entry];
    if (seen.has(entry.numberedGroup)) return [];
    seen.add(entry.numberedGroup);
    return [
      {
        ...entry,
        label: entry.label.replace(/ \d+$/, ""),
        defaults: [{ ...entry.defaults[0], key: "0–9" }],
      },
    ];
  });
}

export function ShortcutKeys({ values }: { values: Shortcut[] }) {
  return (
    <span className="inline-flex flex-wrap justify-end items-center gap-1.5 text-xs text-text-secondary">
      {values.length === 0 && <span className="text-text-muted">Unassigned</span>}
      {compactShortcutLabels(values).map((label, index) => (
        <Fragment key={label}>
          {index > 0 && (
            <span aria-hidden="true" className="text-text-muted">
              /
            </span>
          )}
          <kbd className="rounded border border-border bg-bg-tertiary px-1.5 py-0.5 font-mono whitespace-nowrap">
            {label}
          </kbd>
        </Fragment>
      ))}
    </span>
  );
}

interface Props {
  entries: Binding[];
  overrides: ShortcutOverrides;
  selected: Binding | null;
  onSelect: (entry: Binding | null) => void;
  onRestore: (entry: Binding) => void;
  unavailable: (entry: Binding) => boolean;
  conflicts: (entry: Binding) => string;
  busy: boolean;
  searching: boolean;
  editor: ReactNode;
}

export function ShortcutTable({
  entries,
  overrides,
  selected,
  onSelect,
  onRestore,
  unavailable,
  conflicts,
  busy,
  searching,
  editor,
}: Props) {
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  return (
    <div className="space-y-5">
      {entries.length === 0 && <p className="text-sm text-text-muted">No matching shortcuts.</p>}
      {sections.map((section) => {
        const rows = compactNumberedEntries(
          entries.filter((entry) => sectionFor(entry) === section.id)
        );
        if (!rows.length) return null;
        const expanded = searching || !collapsed.has(section.id);
        return (
          <section key={section.id}>
            <h4>
              <button
                type="button"
                aria-expanded={expanded}
                aria-controls={`shortcut-section-${section.id}`}
                className="w-full flex items-center gap-2 py-2 text-left text-sm font-semibold text-text-primary cursor-pointer focus-visible:outline-accent"
                disabled={busy || searching}
                onClick={() => {
                  if (selected && sectionFor(selected) === section.id) onSelect(null);
                  setCollapsed((current) => {
                    const next = new Set(current);
                    if (next.has(section.id)) next.delete(section.id);
                    else next.add(section.id);
                    return next;
                  });
                }}
              >
                <span aria-hidden="true" className="text-text-muted">
                  {expanded ? "▾" : "▸"}
                </span>
                {section.label}
                <span className="text-xs font-normal text-text-muted">{rows.length}</span>
              </button>
            </h4>
            <div id={`shortcut-section-${section.id}`} hidden={!expanded}>
              <p className="pb-3 text-xs text-text-muted">{section.description}</p>
              <div className="divide-y divide-border border-y border-border">
                {rows.map((entry) => {
                  const custom = entry.editable && Object.hasOwn(overrides, entry.id);
                  const problem = conflicts(entry);
                  return (
                    <Fragment key={entry.id}>
                      <div
                        className="flex items-center gap-3 py-2"
                        role="group"
                        aria-label={entry.label}
                      >
                        <div className="min-w-0 flex-1">
                          <div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
                            <span className="text-sm text-text-primary">{entry.label}</span>
                            <span
                              className={`text-xs ${custom ? "text-accent" : "text-text-muted"}`}
                            >
                              {entry.editable ? (custom ? "Custom" : "Default") : "Standard"}
                            </span>
                          </div>
                          {!entry.numberedGroup && unavailable(entry) && (
                            <p className="text-xs text-text-muted">Currently unavailable</p>
                          )}
                          {problem && <p className="text-xs text-warning">{problem}</p>}
                        </div>
                        <div className="max-w-[40%] shrink-0">
                          <ShortcutKeys values={shortcutDisplayValues(entry, overrides)} />
                        </div>
                        <div className="flex w-28 shrink-0 items-center justify-end gap-1">
                          {entry.editable ? (
                            <>
                              <Button
                                className="px-2.5 py-1 text-xs"
                                disabled={busy}
                                aria-expanded={selected?.id === entry.id}
                                aria-controls={`shortcut-editor-${entry.id}`}
                                aria-label={`Change ${entry.label} (${section.label})`}
                                onClick={() => onSelect(selected?.id === entry.id ? null : entry)}
                              >
                                Change
                              </Button>
                              <span className="inline-flex w-6 justify-center">
                                {custom && (
                                  <button
                                    type="button"
                                    className="text-text-secondary hover:text-text-primary cursor-pointer disabled:opacity-40 focus-visible:outline-accent"
                                    disabled={busy}
                                    aria-label={`Restore defaults for ${entry.label} (${section.label})`}
                                    title={`Restore defaults: ${entry.defaults.map(formatShortcut).join(" / ")}`}
                                    onClick={() => onRestore(entry)}
                                  >
                                    ↺
                                  </button>
                                )}
                              </span>
                            </>
                          ) : (
                            <span
                              className="text-xs text-text-muted"
                              title="Standard navigation cannot be changed"
                            >
                              Not editable
                            </span>
                          )}
                        </div>
                      </div>
                      {selected?.id === entry.id && (
                        <div id={`shortcut-editor-${entry.id}`} className="bg-bg-tertiary/40 p-4">
                          {editor}
                        </div>
                      )}
                    </Fragment>
                  );
                })}
              </div>
            </div>
          </section>
        );
      })}
    </div>
  );
}
