import { KeyboardEvent, ReactNode, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useAgentCatalog } from "../../hooks/useAgentCatalog";
import {
  DefinitionIssue,
  StepsSource,
  WrittenStep,
  readAutomationSteps,
  saveAutomationSteps,
} from "../../services/automations";
import { pipelineLayout, stepKindName, stepSummary } from "../../services/automationPipeline";
import {
  DraftStep,
  EDITABLE_KINDS,
  WRITTEN_KIND_NOTES,
  EditableKind,
  TransitionDraft,
  TransitionMode,
  addStep,
  changeKind,
  draftKind,
  isWrittenKind,
  draftSteps,
  kindField,
  moveStep,
  previewSteps,
  removeStep,
  renameStep,
  setField,
  stepEdits,
  transitionDraft,
  updateStep,
  writtenTransition,
} from "../../services/automationStepEditor";
import { Button } from "../ui/Button";
import { JumpList, LANE_WIDTH, NODE_HEIGHT, NODE_WIDTH, PipelineJumps } from "./PipelineJumps";

const CONTROL =
  "w-full bg-bg-tertiary border border-border rounded-md px-2 py-1 text-xs text-text-primary outline-none focus:border-accent";

const text = (value: unknown) => (typeof value === "string" ? value : "");

interface AutomationStepsEditorProps {
  name: string;
  onClose: () => void;
  onSaved: () => void;
}

/** Edits the `steps` of `AUTOMATION.md` from its graph and saves them with the file's validation. */
export function AutomationStepsEditor({ name, onClose, onSaved }: AutomationStepsEditorProps) {
  const [source, setSource] = useState<StepsSource | null>(null);
  const [steps, setSteps] = useState<DraftStep[]>([]);
  const [selected, setSelected] = useState(0);
  const [dirty, setDirty] = useState(false);
  const [newKind, setNewKind] = useState<EditableKind>("agent");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [issues, setIssues] = useState<DefinitionIssue[]>([]);

  const load = useCallback(() => {
    setError(null);
    setIssues([]);
    readAutomationSteps(name).then(
      (found) => {
        setSource(found);
        setSteps(draftSteps(found));
        setSelected(0);
        setDirty(false);
      },
      (reason) => setError(String(reason))
    );
  }, [name]);

  useEffect(load, [load]);

  const change = (next: DraftStep[], index = selected) => {
    setSteps(next);
    setSelected(Math.max(0, Math.min(index, next.length - 1)));
    setDirty(true);
    setIssues([]);
  };

  const save = async () => {
    if (!source) return;
    setSaving(true);
    setError(null);
    try {
      const result = await saveAutomationSteps(name, source.hash, stepEdits(steps));
      if (result.status === "invalid") setIssues(result.errors);
      else onSaved();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setSaving(false);
    }
  };

  const add = () => {
    const added = addStep(steps, newKind, steps.length ? selected : null);
    change(added.steps, added.index);
  };

  const current = steps[selected];

  return (
    <div className="space-y-4 text-xs">
      <div className="flex flex-wrap items-center gap-2">
        <select
          aria-label="New step type"
          value={newKind}
          onChange={(event) => setNewKind(event.target.value as EditableKind)}
          className={`${CONTROL} w-auto cursor-pointer`}
        >
          {EDITABLE_KINDS.map((kind) => (
            <option key={kind} value={kind}>
              {kind}
            </option>
          ))}
        </select>
        <Button variant="secondary" className="h-7 px-3 text-xs" disabled={!source} onClick={add}>
          Add step
        </Button>
        <div className="ml-auto flex items-center gap-2">
          <Button variant="ghost" className="h-7 px-3 text-xs" onClick={onClose}>
            {dirty ? "Discard" : "Close"}
          </Button>
          <Button
            className="h-7 px-3 text-xs"
            loading={saving}
            disabled={!dirty || steps.length === 0}
            onClick={() => void save()}
          >
            Save
          </Button>
        </div>
      </div>
      <p className="text-text-muted">
        Saving rewrites only the steps block of AUTOMATION.md and validates it like the file.
        Comments inside the steps block are not kept; the rest of the file stays as written.
      </p>
      {error && (
        <div role="alert" className="flex items-start gap-2 text-danger">
          <p className="flex-1 break-words select-text">{error}</p>
          <Button variant="secondary" className="h-6 px-2 text-xs" onClick={load}>
            Reload from disk
          </Button>
        </div>
      )}
      {issues.length > 0 && (
        <ul aria-label="Validation errors" className="space-y-0.5 text-danger select-text">
          {issues.map((issue) => (
            <li key={`${issue.field}:${issue.message}`}>
              <span className="font-mono">{issue.field || "file"}</span>: {issue.message}
            </li>
          ))}
        </ul>
      )}
      {source && (
        <div className="flex flex-wrap items-start gap-6">
          <DraftGraph
            steps={steps}
            selected={selected}
            onSelect={setSelected}
            onMove={(index, offset) => change(moveStep(steps, index, offset), index + offset)}
          />
          {current && (
            <StepForm
              key={current.key}
              step={current.value}
              ids={steps.map((step) => text(step.value.id))}
              onChange={(value) => change(updateStep(steps, selected, value))}
              onRename={(id) => change(renameStep(steps, selected, id))}
              onMove={(offset) => {
                const target = selected + offset;
                if (target < 0 || target >= steps.length) return undefined;
                return () => change(moveStep(steps, selected, offset), target);
              }}
              onRemove={steps.length > 1 ? () => change(removeStep(steps, selected)) : undefined}
            />
          )}
        </div>
      )}
    </div>
  );
}

interface DraftGraphProps {
  steps: DraftStep[];
  selected: number;
  onSelect: (index: number) => void;
  onMove: (index: number, offset: number) => void;
}

/** Arrows select a step; Alt+arrows move it. */
function DraftGraph({ steps, selected, onSelect, onMove }: DraftGraphProps) {
  const preview = useMemo(() => previewSteps(steps), [steps]);
  const layout = useMemo(() => pipelineLayout(preview), [preview]);
  const listRef = useRef<HTMLOListElement>(null);

  const focus = (index: number) =>
    requestAnimationFrame(() =>
      listRef.current?.querySelectorAll<HTMLElement>("button")[index]?.focus()
    );

  const navigate = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    const offset = event.key === "ArrowDown" ? 1 : event.key === "ArrowUp" ? -1 : 0;
    const target = index + offset;
    if (offset === 0 || target < 0 || target >= steps.length) return;
    event.preventDefault();
    if (event.altKey) onMove(index, offset);
    else onSelect(target);
    focus(target);
  };

  return (
    <div className="space-y-4">
      <div className="relative" style={{ width: NODE_WIDTH + layout.lanes * LANE_WIDTH + 12 }}>
        <ol ref={listRef} aria-label="Draft steps" className="space-y-6">
          {preview.map((step, index) => (
            <li key={steps[index].key}>
              <button
                type="button"
                aria-pressed={index === selected}
                tabIndex={index === selected ? 0 : -1}
                onClick={() => onSelect(index)}
                onKeyDown={(event) => navigate(event, index)}
                style={{ width: NODE_WIDTH, height: NODE_HEIGHT }}
                className={`flex flex-col justify-center rounded-lg border px-3 text-left cursor-pointer transition-colors outline-none focus-visible:ring-2 focus-visible:ring-accent ${
                  index === selected ? "bg-bg-hover border-accent" : "bg-bg-tertiary border-border"
                }`}
              >
                <span className="flex items-baseline gap-2">
                  <span className="font-medium text-text-primary truncate">
                    {step.label || "(no id)"}
                  </span>
                  <span className="text-text-muted">{stepKindName(step)}</span>
                </span>
                <span className="block text-text-muted font-mono truncate">
                  {stepSummary(step)}
                </span>
              </button>
            </li>
          ))}
        </ol>
        <PipelineJumps edges={layout.edges} steps={preview} lanes={layout.lanes} traversed={null} />
      </div>
      <JumpList edges={layout.edges} steps={preview} traversed={null} />
    </div>
  );
}

interface StepFormProps {
  step: WrittenStep;
  ids: string[];
  onChange: (step: WrittenStep) => void;
  onRename: (id: string) => void;
  /** Undefined where the step cannot move that way. */
  onMove: (offset: -1 | 1) => (() => void) | undefined;
  onRemove?: () => void;
}

function StepForm({ step, ids, onChange, onRename, onMove, onRemove }: StepFormProps) {
  const [moveUp, moveDown] = [onMove(-1), onMove(1)];
  const kind = draftKind(step);
  const [id, setId] = useState(text(step.id));
  const others = ids.filter((other) => other !== text(step.id));
  const set = (key: string, value: unknown) => onChange(setField(step, key, value));
  const setInKind = (key: string, value: unknown) => onChange(setField(step, key, value, true));
  const commitId = () => {
    if (id.trim() !== text(step.id)) onRename(id.trim());
  };

  return (
    <section aria-label="Step settings" className="min-w-64 flex-1 max-w-md space-y-3">
      <div className="flex gap-2">
        <Button
          variant="secondary"
          className="h-6 px-2 text-xs"
          disabled={!moveUp}
          onClick={moveUp}
        >
          Move up
        </Button>
        <Button
          variant="secondary"
          className="h-6 px-2 text-xs"
          disabled={!moveDown}
          onClick={moveDown}
        >
          Move down
        </Button>
        {onRemove && (
          <Button variant="danger" className="h-6 px-2 text-xs ml-auto" onClick={onRemove}>
            Remove
          </Button>
        )}
      </div>
      <Field label="Id">
        <input
          value={id}
          onChange={(event) => setId(event.target.value)}
          onBlur={commitId}
          onKeyDown={(event) => event.key === "Enter" && commitId()}
          className={`${CONTROL} font-mono`}
        />
      </Field>
      <Field label="Label">
        <input
          value={text(step.label)}
          placeholder={text(step.id)}
          onChange={(event) => set("label", event.target.value)}
          className={CONTROL}
        />
      </Field>
      <Field label="Type">
        <select
          value={kind ?? ""}
          onChange={(event) => onChange(changeKind(step, event.target.value as EditableKind))}
          className={`${CONTROL} cursor-pointer`}
        >
          {kind === null && <option value="">Several or none: fix in AUTOMATION.md</option>}
          {EDITABLE_KINDS.map((name) => (
            <option key={name} value={name}>
              {name}
            </option>
          ))}
          {isWrittenKind(kind) && (
            <option value={kind} disabled>
              {kind}
            </option>
          )}
        </select>
      </Field>
      {kind === "agent" && <AgentFields step={step} onChange={setInKind} />}
      {kind === "command" && (
        <CommandField command={step.command} onChange={(value) => set("command", value)} />
      )}
      {kind === "wait" && <WaitFields step={step} onChange={(value) => set("wait", value)} />}
      {kind === "approval" && (
        <Field label="Title">
          <input
            value={text(kindField(step, "title"))}
            onChange={(event) => setInKind("title", event.target.value)}
            className={CONTROL}
          />
        </Field>
      )}
      {isWrittenKind(kind) && <p className="text-text-muted">{WRITTEN_KIND_NOTES[kind]}</p>}
      <Field label="Next">
        <select
          value={text(step.next)}
          onChange={(event) => set("next", event.target.value)}
          className={`${CONTROL} cursor-pointer`}
        >
          <option value="">Following step</option>
          <StepOptions ids={others} value={text(step.next)} />
        </select>
      </Field>
      <TransitionField
        label="On fail"
        defaultLabel="Fail the item"
        value={step.on_fail}
        steps={ids}
        onChange={(value) => set("on_fail", value)}
      />
      {(kind === "agent" || kind === "approval") && (
        <TransitionField
          label="On reject"
          defaultLabel="Same as on fail"
          value={kindField(step, "on_reject")}
          steps={ids}
          onChange={(value) => setInKind("on_reject", value)}
        />
      )}
    </section>
  );
}

function CommandField({
  command,
  onChange,
}: {
  command: unknown;
  onChange: (argv: string[]) => void;
}) {
  const [lines, setLines] = useState(Array.isArray(command) ? command.map(String).join("\n") : "");
  return (
    <Field label="Command (one argument per line)">
      <textarea
        rows={3}
        value={lines}
        onChange={(event) => {
          setLines(event.target.value);
          onChange(event.target.value.split("\n").filter(Boolean));
        }}
        className={`${CONTROL} font-mono`}
      />
    </Field>
  );
}

/** A `<select>` of step ids that still shows a value no step has. */
function StepOptions({ ids, value }: { ids: string[]; value: string }) {
  return (
    <>
      {ids.map((id) => (
        <option key={id} value={id}>
          {id}
        </option>
      ))}
      {value && !ids.includes(value) && <option value={value}>{value} (missing)</option>}
    </>
  );
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="flex flex-col gap-1">
      <span className="text-text-muted">{label}</span>
      {children}
    </label>
  );
}

function AgentFields({
  step,
  onChange,
}: {
  step: WrittenStep;
  onChange: (key: string, value: unknown) => void;
}) {
  const catalog = useAgentCatalog();
  const agent = text(kindField(step, "id"));
  const known = catalog.some((descriptor) => descriptor.id === agent);
  const input = (key: string, label: string, placeholder = "Automation default") => (
    <Field label={label}>
      <input
        value={text(kindField(step, key))}
        placeholder={placeholder}
        onChange={(event) => onChange(key, event.target.value)}
        className={CONTROL}
      />
    </Field>
  );
  return (
    <>
      <Field label="Agent">
        <select
          value={agent}
          onChange={(event) => onChange("id", event.target.value)}
          className={`${CONTROL} cursor-pointer`}
        >
          <option value="">Automation default</option>
          {catalog.map((descriptor) => (
            <option key={descriptor.id} value={descriptor.id}>
              {descriptor.label}
            </option>
          ))}
          {agent && !known && <option value={agent}>{agent}</option>}
        </select>
      </Field>
      {input("model", "Model")}
      {input("effort", "Effort")}
      {input("skill", "Skill", "None")}
      <Field label="Prompt">
        <textarea
          rows={4}
          value={text(kindField(step, "prompt"))}
          placeholder="Appended after the instructions"
          onChange={(event) => onChange("prompt", event.target.value)}
          className={CONTROL}
        />
      </Field>
    </>
  );
}

type WaitMode = "signal" | "time";

function WaitFields({ step, onChange }: { step: WrittenStep; onChange: (value: unknown) => void }) {
  const wait = step.wait;
  const config = typeof wait === "object" && wait !== null ? (wait as Record<string, unknown>) : {};
  if ("event" in config) {
    return (
      <p className="text-text-muted">The watcher and its params are edited in AUTOMATION.md.</p>
    );
  }
  const mode: WaitMode = "time" in config ? "time" : "signal";
  return (
    <>
      <Field label="Wait for">
        <select
          value={mode}
          onChange={(event) => onChange({ [event.target.value]: "" })}
          className={`${CONTROL} cursor-pointer`}
        >
          <option value="signal">Signal</option>
          <option value="time">Time</option>
        </select>
      </Field>
      <Field label={mode === "time" ? "Duration or instant" : "Signal name"}>
        <input
          value={text(config[mode])}
          placeholder={mode === "time" ? "45m or 2026-12-01T09:00:00Z" : "ready"}
          onChange={(event) => onChange({ ...config, [mode]: event.target.value })}
          className={CONTROL}
        />
      </Field>
    </>
  );
}

const MODE_LABELS: Record<Exclude<TransitionMode, "default">, string> = {
  fail: "Fail the item",
  block: "Block the item",
  defer: "Defer the item",
  continue: "Continue to next",
  goto: "Go to step",
};

interface TransitionFieldProps {
  label: string;
  defaultLabel: string;
  value: unknown;
  steps: string[];
  onChange: (value: unknown) => void;
}

function TransitionField({ label, defaultLabel, value, steps, onChange }: TransitionFieldProps) {
  const draft = transitionDraft(value);
  const update = (patch: Partial<TransitionDraft>) =>
    onChange(writtenTransition({ ...draft, ...patch }));
  return (
    <fieldset className="space-y-1">
      <legend className="text-text-muted mb-1">{label}</legend>
      <div className="flex gap-2">
        <select
          aria-label={label}
          value={draft.mode}
          onChange={(event) =>
            update({
              mode: event.target.value as TransitionMode,
              step: draft.step || steps[0] || "",
            })
          }
          className={`${CONTROL} cursor-pointer`}
        >
          <option value="default">Default: {defaultLabel.toLowerCase()}</option>
          {Object.entries(MODE_LABELS).map(([mode, name]) => (
            <option key={mode} value={mode}>
              {name}
            </option>
          ))}
        </select>
        {draft.mode === "goto" && (
          <>
            <select
              aria-label={`${label} step`}
              value={draft.step}
              onChange={(event) => update({ step: event.target.value })}
              className={`${CONTROL} cursor-pointer font-mono`}
            >
              <StepOptions ids={steps} value={draft.step} />
            </select>
            <input
              aria-label={`${label} max`}
              value={draft.max}
              placeholder="max"
              title="Most times an item takes this jump: a count or the budget repair_rounds"
              onChange={(event) => update({ max: event.target.value })}
              className={`${CONTROL} w-24`}
            />
          </>
        )}
      </div>
    </fieldset>
  );
}
