import { KeyboardEvent, useEffect, useRef, useState } from "react";
import { Button } from "../ui/Button";
import { ChevronDownIcon } from "../ui/Icons";
import { MenuOption, OptionMenu } from "./OptionMenu";
import {
  filterCommands,
  insertMention,
  localCommand,
  menuTrigger,
  promptMemory,
} from "./composerMenu";
import {
  chatFileSearch,
  ChatControls,
  ChatSetting,
  ChatStatus,
  CommandOption,
  effortsFor,
  FileMatch,
  isLive,
} from "../../services/chat";

type Picker = "model" | "effort" | "mode";

export interface ComposerActions {
  send: (text: string) => Promise<void>;
  interrupt: () => Promise<void>;
  configure: (setting: ChatSetting) => Promise<void>;
  compact: () => Promise<void>;
  startFresh: () => Promise<void>;
}

interface ComposerProps {
  memoryKey: string;
  agentLabel: string;
  status: ChatStatus;
  controls: ChatControls;
  folders: string[];
  actions: ComposerActions;
  onError: (message: string | null) => void;
}

const PICKER_SHORTCUTS: Record<string, Picker> = { m: "mode", i: "model", e: "effort" };
const PICKER_LABEL: Record<Picker, string> = {
  model: "Model",
  effort: "Reasoning effort",
  mode: "Permissions",
};

const labelFor = (options: { id: string; label: string }[], id: string | null) =>
  options.find((option) => option.id === id)?.label ?? id;

export function Composer({
  memoryKey,
  agentLabel,
  status,
  controls,
  folders,
  actions,
  onError,
}: ComposerProps) {
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const [text, setText] = useState(() => promptMemory.draft(memoryKey));
  const [caret, setCaret] = useState(text.length);
  const [picker, setPicker] = useState<Picker | null>(null);
  const [dismissed, setDismissed] = useState<string | null>(null);
  const [active, setActive] = useState(0);
  const [files, setFiles] = useState<FileMatch[]>([]);
  const [recall, setRecall] = useState<number | null>(null);
  const live = isLive(status);
  const busy = status.kind === "busy";
  const ready = status.kind === "idle" || busy;
  const efforts = effortsFor(controls);
  const trigger = picker || dismissed === text ? null : menuTrigger(text, caret);
  const mentionQuery = trigger?.kind === "mention" ? trigger.query : null;
  const foldersKey = folders.join("\n");

  useEffect(() => promptMemory.saveDraft(memoryKey, text), [memoryKey, text]);

  useEffect(() => {
    if (mentionQuery === null) return;
    let stale = false;
    const timer = setTimeout(() => {
      chatFileSearch(foldersKey.split("\n"), mentionQuery)
        .then((found) => !stale && setFiles(found))
        .catch(() => !stale && setFiles([]));
    }, 80);
    return () => {
      stale = true;
      clearTimeout(timer);
    };
  }, [mentionQuery, foldersKey]);

  const slashCommands =
    trigger?.kind === "slash" ? filterCommands(controls.commands, trigger.query) : [];

  const options: MenuOption[] = picker
    ? pickerOptions(picker, controls, efforts)
    : trigger?.kind === "slash"
      ? slashCommands.map((command) => ({
          id: command.name,
          label: `/${command.name}`,
          hint: command.argumentHint,
          description: command.description,
        }))
      : trigger?.kind === "mention"
        ? files.map((file) => ({ id: file.path, label: file.name, description: file.path }))
        : [];
  const menuOpen = picker !== null || trigger !== null;
  const activeIndex = Math.min(active, Math.max(0, options.length - 1));

  const focus = (nextCaret?: number) =>
    requestAnimationFrame(() => {
      const el = textareaRef.current;
      if (!el) return;
      el.focus();
      if (nextCaret !== undefined) el.setSelectionRange(nextCaret, nextCaret);
    });

  const update = (next: string, nextCaret = next.length) => {
    setText(next);
    setCaret(nextCaret);
    setActive(0);
    setRecall(null);
    focus(nextCaret);
  };

  const run = async (action: () => Promise<void>) => {
    onError(null);
    try {
      await action();
    } catch (e) {
      onError(String(e));
    }
  };

  const configure = (setting: ChatSetting["kind"], value: string) =>
    run(() => actions.configure({ kind: setting, value } as ChatSetting));

  const openPicker = (next: Picker) => {
    setPicker(next);
    setActive(0);
    focus();
  };

  const runCommand = (command: CommandOption) => {
    const action = command.action;
    if (action.kind === "insert") return update(action.text);
    update("");
    if (action.kind === "model" || action.kind === "effort") openPicker(action.kind);
    else if (action.kind === "mode") void configure("mode", action.mode);
    else if (action.kind === "compact") void run(actions.compact);
    else void run(actions.startFresh);
  };

  const pick = (option: MenuOption) => {
    if (picker) {
      setPicker(null);
      void configure(picker, option.id);
      focus();
    } else if (trigger?.kind === "slash") {
      const command = slashCommands.find((c) => c.name === option.id);
      if (command) runCommand(command);
    } else if (trigger?.kind === "mention") {
      const next = insertMention(text, trigger.start, caret, option.id);
      update(next.text, next.caret);
    }
  };

  const submit = async () => {
    const message = text.trim();
    if (!message || !ready) return;
    const local = localCommand(message, controls);
    if (local) {
      update("");
      if (local.kind === "picker") openPicker(local.picker);
      else if (local.kind === "configure") void configure(local.setting, local.value);
      else if (local.kind === "compact") void run(actions.compact);
      else void run(actions.startFresh);
      return;
    }
    onError(null);
    try {
      await actions.send(message);
      promptMemory.remember(memoryKey, message);
      update("");
    } catch (e) {
      onError(String(e));
    }
  };

  const recallHistory = (direction: -1 | 1) => {
    const entries = promptMemory.history(memoryKey);
    if (!entries.length) return false;
    const current = recall ?? entries.length;
    const next = Math.max(0, Math.min(entries.length, current + direction));
    const value = next === entries.length ? "" : entries[next];
    setText(value);
    setCaret(value.length);
    setRecall(next === entries.length ? null : next);
    focus(value.length);
    return true;
  };

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.nativeEvent.isComposing) return;
    const shortcut = e.metaKey && e.shiftKey && PICKER_SHORTCUTS[e.key.toLowerCase()];
    if (shortcut) {
      e.preventDefault();
      if (shortcut !== "effort" || efforts.length) openPicker(shortcut);
      return;
    }
    if (menuOpen) {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const step = e.key === "ArrowDown" ? 1 : -1;
        setActive((activeIndex + step + options.length) % Math.max(1, options.length));
        return;
      }
      if ((e.key === "Enter" && !e.shiftKey) || e.key === "Tab") {
        if (options[activeIndex]) {
          e.preventDefault();
          pick(options[activeIndex]);
          return;
        }
      }
      if (picker && /^[1-9]$/.test(e.key) && options[Number(e.key) - 1]) {
        e.preventDefault();
        pick(options[Number(e.key) - 1]);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        setPicker(null);
        setDismissed(text);
        return;
      }
    }
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void submit();
    } else if (e.key === "Escape") {
      e.preventDefault();
      if (busy) void run(actions.interrupt);
      else e.currentTarget.blur();
    } else if (e.key === "ArrowUp" && e.currentTarget.selectionStart === 0 && !e.shiftKey) {
      if (recallHistory(-1)) e.preventDefault();
    } else if (e.key === "ArrowDown" && recall !== null && !e.shiftKey) {
      if (recallHistory(1)) e.preventDefault();
    }
  };

  const placeholder = busy
    ? `${agentLabel} is working — messages you send now are added to this turn (Esc to stop)`
    : `Message ${agentLabel} — / for commands, @ for files`;

  return (
    <div className="max-w-3xl mx-auto flex flex-col gap-1.5">
      <div className="relative">
        {menuOpen && (
          <OptionMenu
            label={picker ? PICKER_LABEL[picker] : trigger?.kind === "slash" ? "Commands" : "Files"}
            options={options}
            active={activeIndex}
            numbered={picker !== null}
            empty={trigger?.kind === "mention" ? "No matching files" : undefined}
            onHover={setActive}
            onPick={pick}
          />
        )}
        <div className="rounded-lg bg-bg-secondary border border-border focus-within:border-accent">
          <textarea
            ref={textareaRef}
            aria-label={`Message ${agentLabel}`}
            value={text}
            onChange={(e) => {
              setText(e.target.value);
              setCaret(e.target.selectionStart);
              setActive(0);
              setRecall(null);
              setDismissed(null);
            }}
            onSelect={(e) => setCaret(e.currentTarget.selectionStart)}
            onBlur={() => setPicker(null)}
            onKeyDown={onKeyDown}
            disabled={!live}
            rows={Math.min(10, Math.max(2, text.split("\n").length))}
            placeholder={placeholder}
            className="block w-full resize-none bg-transparent px-3 pt-2 pb-1 text-sm text-text-primary placeholder:text-text-muted focus:outline-none disabled:opacity-50"
            autoFocus
          />
          <div className="flex items-center gap-1 px-1.5 pb-1.5">
            {controls.modes.length > 0 && (
              <Chip
                label="Permissions"
                shortcut="⌘⇧M"
                value={labelFor(controls.modes, controls.mode) ?? "Mode"}
                highlighted={controls.mode === "plan"}
                onClick={() => openPicker("mode")}
              />
            )}
            {controls.models.length > 0 && (
              <Chip
                label="Model"
                shortcut="⌘⇧I"
                value={labelFor(controls.models, controls.model) ?? "Model"}
                onClick={() => openPicker("model")}
              />
            )}
            {efforts.length > 0 && (
              <Chip
                label="Reasoning effort"
                shortcut="⌘⇧E"
                value={controls.effort ?? "effort"}
                onClick={() => openPicker("effort")}
              />
            )}
            <span className="ml-auto flex items-center gap-2">
              {controls.context && <ContextRing {...controls.context} />}
              {busy && (
                <Button
                  type="button"
                  variant="secondary"
                  className="h-7 px-3 text-xs"
                  onClick={() => void run(actions.interrupt)}
                >
                  Stop
                </Button>
              )}
              {(!busy || text.trim()) && (
                <Button
                  type="button"
                  className="h-7 px-3 text-xs"
                  disabled={!text.trim() || !ready}
                  onClick={() => void submit()}
                >
                  Send
                </Button>
              )}
            </span>
          </div>
        </div>
      </div>
    </div>
  );
}

function pickerOptions(picker: Picker, controls: ChatControls, efforts: string[]): MenuOption[] {
  if (picker === "model")
    return controls.models.map((model) => ({
      id: model.id,
      label: model.label,
      description: model.description,
      selected: model.id === controls.model,
    }));
  if (picker === "effort")
    return efforts.map((effort) => ({
      id: effort,
      label: effort,
      selected: effort === controls.effort,
    }));
  return controls.modes.map((mode) => ({
    id: mode.id,
    label: mode.label,
    description: mode.description,
    selected: mode.id === controls.mode,
  }));
}

function Chip({
  label,
  value,
  shortcut,
  highlighted,
  onClick,
}: {
  label: string;
  value: string;
  shortcut: string;
  highlighted?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      aria-label={`${label}: ${value}`}
      title={`${label} (${shortcut})`}
      onMouseDown={(e) => e.preventDefault()}
      onClick={onClick}
      className={`h-6 flex items-center gap-1 px-2 rounded-md text-[0.6875rem] cursor-pointer transition-colors ${
        highlighted
          ? "bg-accent/15 text-accent-hover"
          : "text-text-secondary hover:text-text-primary hover:bg-bg-hover"
      }`}
    >
      <span className="max-w-[160px] truncate">{value}</span>
      <ChevronDownIcon className="opacity-50" />
    </button>
  );
}

function ContextRing({ used, max }: { used: number; max: number }) {
  const ratio = max > 0 ? Math.min(1, used / max) : 0;
  const radius = 6;
  const circumference = 2 * Math.PI * radius;
  const tone = ratio > 0.85 ? "text-danger" : ratio > 0.6 ? "text-warning" : "text-text-muted";
  const percent = Math.round(ratio * 100);
  return (
    <span
      role="img"
      aria-label={`Context ${percent}% used`}
      title={`Context: ${used.toLocaleString()} of ${max.toLocaleString()} tokens (${percent}%). /compact frees space.`}
      className={`flex items-center gap-1 text-[0.6875rem] ${tone}`}
    >
      <svg width="16" height="16" viewBox="0 0 16 16" className="-rotate-90">
        <circle
          cx="8"
          cy="8"
          r={radius}
          fill="none"
          stroke="currentColor"
          strokeOpacity="0.25"
          strokeWidth="2"
        />
        <circle
          cx="8"
          cy="8"
          r={radius}
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeDasharray={circumference}
          strokeDashoffset={circumference * (1 - ratio)}
        />
      </svg>
      {percent}%
    </span>
  );
}
