import { Button } from "../ui/Button";
import { ShortcutKeys, ShortcutTable } from "./ShortcutTable";
import { useEffect, useRef, useState } from "react";
import {
  bindings,
  conflicts,
  effectiveBindings,
  eventShortcut,
  formatShortcut,
  overlaps,
  signature,
  validKey,
  type Binding,
  type Shortcut,
  type ShortcutContext,
} from "../../shortcuts/catalog";
import { setShortcutCapture, useShortcutOverrides } from "../../shortcuts/runtime";
import type { Operations } from "../../services/operations";

const contextNames: Record<ShortcutContext, string> = {
  list: "Task list",
  task: "Task view",
  chat: "Chat",
  palette: "Search palette",
  modal: "Dialogs",
};
const describeContext = (entry: Binding) =>
  entry.contexts.map((context) => contextNames[context]).join(" · ");
function searchableShortcut(value: Shortcut) {
  return [
    value.modifiers.includes("meta") ? "cmd" : "",
    value.modifiers.includes("ctrl") ? "ctrl" : "",
    value.modifiers.includes("alt") ? "alt" : "",
    value.modifiers.includes("shift") ? "shift" : "",
    value.key,
  ]
    .filter(Boolean)
    .join("+")
    .toLowerCase();
}

interface Props {
  operations: Pick<Operations, "updateShortcut" | "resetShortcuts">;
  workspaceCount: number;
  taskCount: number;
  paletteTaskCount?: number;
}
export function KeyboardShortcutsSettings({
  operations,
  workspaceCount,
  taskCount,
  paletteTaskCount = taskCount,
}: Props) {
  const overrides = useShortcutOverrides();
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState("all");
  const [selected, setSelected] = useState<Binding | null>(null);
  const [editing, setEditing] = useState<{ entry: Binding; index?: number } | null>(null);
  const [candidate, setCandidate] = useState<Shortcut | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [ready, setReady] = useState(false);
  const [pending, setPending] = useState<{ entry: Binding; values: Shortcut[] | null } | null>(
    null
  );
  const [resetting, setResetting] = useState(false);
  const captureRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!editing) return;
    let mounted = true;
    void setShortcutCapture(true)
      .then(() => {
        if (mounted) {
          setReady(true);
          captureRef.current?.focus();
        }
      })
      .catch(() => {
        if (mounted) setError("Could not suspend native shortcuts. Cancel and try again.");
      });
    const pressed = new Set<string>();
    const down = (event: KeyboardEvent) => {
      if (event.key === "Tab") return;
      if (
        event.key === "Escape" &&
        !event.metaKey &&
        !event.ctrlKey &&
        !event.altKey &&
        !event.shiftKey
      ) {
        event.preventDefault();
        event.stopImmediatePropagation();
        setEditing(null);
        setPending(null);
        return;
      }
      if (event.target !== captureRef.current) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      if (["Meta", "Control", "Alt", "Shift"].includes(event.key)) return;
      pressed.add(event.code || event.key);
      if (pressed.size > 1 || event.isComposing || !validKey(event.key)) {
        setError("Use modifiers and a single key; sequences are not supported.");
        setCandidate(null);
        return;
      }
      if (event.repeat) return;
      setError("");
      setCandidate(eventShortcut(event));
    };
    const up = (event: KeyboardEvent) => pressed.delete(event.code || event.key);
    const blur = () => pressed.clear();
    window.addEventListener("keydown", down, true);
    window.addEventListener("keyup", up, true);
    window.addEventListener("blur", blur);
    return () => {
      mounted = false;
      window.removeEventListener("keydown", down, true);
      window.removeEventListener("keyup", up, true);
      window.removeEventListener("blur", blur);
      void setShortcutCapture(false).catch(() =>
        setError("Could not restore native shortcuts. Reopen the shortcut recorder to retry.")
      );
    };
  }, [editing]);
  const run = async (action: () => Promise<unknown>) => {
    setBusy(true);
    setError("");
    try {
      await action();
      setPending(null);
      setEditing(null);
      setResetting(false);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
    }
  };
  const save = (entry: Binding, values: Shortcut[] | null) => {
    const collisions = conflicts(entry, values ?? entry.defaults, overrides);
    const reserved = collisions.filter((other) => !other.editable);
    if (reserved.length) {
      setError(`Reserved for ${reserved.map((other) => other.label).join(", ")}.`);
      return;
    }
    if (collisions.length) {
      setSelected(entry);
      setPending({ entry, values });
      return;
    }
    void run(() => operations.updateShortcut(entry.id, values));
  };
  const saveCandidate = () => {
    if (!editing || !candidate) return;
    const values = [...effectiveBindings(editing.entry, overrides)];
    if (values.some((value, index) => index !== editing.index && overlaps(value, candidate))) {
      setError("Already assigned to this action.");
      return;
    }
    if (editing.index !== undefined && signature(values[editing.index]) === signature(candidate)) {
      setEditing(null);
      return;
    }
    if (editing.index === undefined) values.push(candidate);
    else values[editing.index] = candidate;
    save(editing.entry, values);
  };
  const pendingConflicts = pending
    ? conflicts(pending.entry, pending.values ?? pending.entry.defaults, overrides)
    : [];
  const visible = bindings.filter((entry) => {
    const active = effectiveBindings(entry, overrides);
    return (
      (filter !== "custom" || (entry.editable && Object.hasOwn(overrides, entry.id))) &&
      (filter !== "unassigned" || active.length === 0) &&
      `${entry.label} ${entry.category} ${describeContext(entry)} ${[...active, ...entry.defaults].map((value) => `${formatShortcut(value)} ${searchableShortcut(value)}`).join(" ")}`
        .toLowerCase()
        .includes(query.toLowerCase())
    );
  });
  const control =
    "rounded border border-border px-2 py-1 text-xs disabled:opacity-40 hover:bg-bg-hover";
  const selectEntry = (entry: Binding | null) => {
    setSelected(entry);
    setEditing(null);
    setPending(null);
    setError("");
  };
  const startRecording = (entry: Binding, index?: number) => {
    setEditing({ entry, index });
    setCandidate(null);
    setReady(false);
    setPending(null);
    setError("");
  };
  const editor = selected && (
    <div className="space-y-3">
      {error && (
        <p role="alert" className="text-danger text-sm select-text">
          {error}
        </p>
      )}
      <div className="flex items-center justify-between gap-3">
        <p className="text-sm font-medium">Change shortcuts for {selected.label}</p>
        <button type="button" className={control} disabled={busy} onClick={() => selectEntry(null)}>
          Done
        </button>
      </div>
      {!editing && (
        <>
          <div className="space-y-2">
            {effectiveBindings(selected, overrides).map((value, index) => (
              <div key={signature(value)} className="flex items-center justify-between gap-4">
                <ShortcutKeys values={[value]} />
                <div className="flex gap-2">
                  <Button
                    variant="secondary"
                    className="px-2 py-1 text-xs"
                    disabled={busy}
                    aria-label={`Replace ${formatShortcut(value)}`}
                    onClick={() => startRecording(selected, index)}
                  >
                    Replace
                  </Button>
                  <Button
                    variant="ghost"
                    className="px-2 py-1 text-xs"
                    disabled={busy}
                    aria-label={`Remove ${formatShortcut(value)}`}
                    onClick={() =>
                      save(
                        selected,
                        effectiveBindings(selected, overrides).filter((_, i) => i !== index)
                      )
                    }
                  >
                    Remove
                  </Button>
                </div>
              </div>
            ))}
          </div>
          <Button
            variant="secondary"
            className="px-2.5 py-1 text-xs"
            disabled={busy}
            onClick={() => startRecording(selected)}
          >
            Add alternative
          </Button>
        </>
      )}
      {editing && (
        <div className="border border-border rounded p-3 space-y-2">
          <p>Press a new combination</p>
          <div
            ref={captureRef}
            tabIndex={0}
            role="group"
            aria-label="Record shortcut"
            className="border border-accent rounded p-4 focus:outline-accent"
          >
            {candidate ? formatShortcut(candidate) : "Press modifiers and one key"}
          </div>
          <p className="text-xs text-text-muted">
            One combination per shortcut. Escape cancels; Tab moves focus.
          </p>
          <button
            className={control}
            disabled={!ready || !candidate || busy}
            onClick={saveCandidate}
          >
            Save shortcut
          </button>{" "}
          <button
            className={control}
            disabled={busy}
            onClick={() => {
              setEditing(null);
              setPending(null);
            }}
          >
            Cancel
          </button>
          {candidate && (
            <p className="text-xs text-text-muted">
              {bindings
                .filter(
                  (other) =>
                    other.id !== editing.entry.id &&
                    !other.contexts.some((c) => editing.entry.contexts.includes(c)) &&
                    effectiveBindings(other, overrides).some((b) => overlaps(b, candidate))
                )
                .map((other) => `Also used by ${other.label} (${describeContext(other)})`)
                .join(" · ")}
            </p>
          )}
        </div>
      )}
      {pending && (
        <div
          role="alertdialog"
          aria-label="Reassign shortcut"
          className="border border-warning rounded p-3 space-y-2"
        >
          <p>This assignment affects:</p>
          <ul>
            {pendingConflicts.map((other) => (
              <li key={other.id}>
                {other.label} ({describeContext(other)})
                {effectiveBindings(other, overrides).every((b) =>
                  (pending.values ?? pending.entry.defaults).some((a) => overlaps(a, b))
                )
                  ? " — will have no shortcut"
                  : " — other alternatives will remain"}
              </li>
            ))}
          </ul>
          <button
            className={control}
            disabled={busy}
            onClick={() =>
              void run(() => operations.updateShortcut(pending.entry.id, pending.values, true))
            }
          >
            Reassign
          </button>{" "}
          <button className={control} disabled={busy} onClick={() => setPending(null)}>
            Cancel reassignment
          </button>
        </div>
      )}
    </div>
  );
  return (
    <div className="p-6 space-y-4">
      <div className="flex gap-2">
        <input
          aria-label="Search shortcuts"
          placeholder="Search actions or shortcuts"
          value={query}
          onChange={(e) => {
            selectEntry(null);
            setQuery(e.target.value);
          }}
          className="flex-1 min-w-0 rounded border border-border bg-bg-primary px-3 py-2"
        />
        <select
          aria-label="Filter shortcuts"
          value={filter}
          onChange={(e) => {
            selectEntry(null);
            setFilter(e.target.value);
          }}
          className="bg-bg-primary border border-border rounded"
        >
          <option value="all">All</option>
          <option value="custom">Customized</option>
          <option value="unassigned">Unassigned</option>
        </select>
        <button className={control} disabled={busy || !!editing} onClick={() => setResetting(true)}>
          Restore all
        </button>
      </div>
      {error && !selected && (
        <p role="alert" className="text-danger text-sm select-text">
          {error}
        </p>
      )}
      {resetting && (
        <div className="border border-border rounded p-3 space-y-2">
          <p>Restore all default shortcuts? This removes all personal assignments.</p>
          <button
            className={control}
            disabled={busy}
            onClick={() => void run(() => operations.resetShortcuts())}
          >
            Confirm restore
          </button>{" "}
          <button className={control} disabled={busy} onClick={() => setResetting(false)}>
            Cancel
          </button>
        </div>
      )}
      <ShortcutTable
        entries={visible}
        overrides={overrides}
        selected={selected}
        onSelect={selectEntry}
        onRestore={(entry) => {
          selectEntry(null);
          save(entry, null);
        }}
        busy={busy}
        searching={query.trim().length > 0 || filter !== "all"}
        unavailable={(entry) => {
          const position = Number(entry.id.split(".").at(-1));
          return (
            Number.isInteger(position) &&
            (entry.id.includes("workspace")
              ? position >= workspaceCount
              : entry.id.startsWith("list.jump")
                ? position >= taskCount
                : entry.id.startsWith("palette.jump")
                  ? position >= paletteTaskCount
                  : false)
          );
        }}
        conflicts={(entry) => {
          const collisions = Object.hasOwn(overrides, entry.id)
            ? conflicts(entry, effectiveBindings(entry, overrides), overrides)
            : [];
          return collisions.length
            ? `Conflicts with ${collisions.map((other) => `${other.label} (${describeContext(other)})`).join(", ")}. Ambiguous personal assignments will not run.`
            : "";
        }}
        editor={editor}
      />
    </div>
  );
}
