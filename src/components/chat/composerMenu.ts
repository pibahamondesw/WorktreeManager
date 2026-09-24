import { ChatControls, CommandOption, effortsFor } from "../../services/chat";

export type MenuTrigger =
  | { kind: "slash"; query: string }
  | { kind: "mention"; query: string; start: number };

/** Which completion menu the caret is in: a leading `/command` or an `@path` token. */
export function menuTrigger(text: string, caret: number): MenuTrigger | null {
  const before = text.slice(0, caret);
  const slash = /^\/(\S*)$/.exec(before);
  if (slash) return { kind: "slash", query: slash[1] };
  const mention = /(^|\s)@(\S*)$/.exec(before);
  if (mention) return { kind: "mention", query: mention[2], start: caret - mention[2].length - 1 };
  return null;
}

/** Prefix matches first, then substring matches in the name, then in the description. */
export function filterCommands(commands: CommandOption[], query: string): CommandOption[] {
  const q = query.toLowerCase();
  const rank = (command: CommandOption) => {
    const name = command.name.toLowerCase();
    if (name.startsWith(q)) return 0;
    if (name.includes(q)) return 1;
    if (command.description.toLowerCase().includes(q)) return 2;
    return -1;
  };
  return commands
    .map((command, index) => ({ command, index, rank: rank(command) }))
    .filter((entry) => entry.rank >= 0)
    .sort((a, b) => a.rank - b.rank || a.index - b.index)
    .map((entry) => entry.command);
}

export function insertMention(text: string, start: number, caret: number, path: string) {
  const next = `${text.slice(0, start)}@${path} ${text.slice(caret)}`;
  return { text: next, caret: start + path.length + 2 };
}

export type LocalCommand =
  | { kind: "configure"; setting: "model" | "effort" | "mode"; value: string }
  | { kind: "picker"; picker: "model" | "effort" | "mode" }
  | { kind: "compact" }
  | { kind: "clear" };

/**
 * A submitted message that the chat handles itself instead of sending: `/model sonnet`,
 * `/effort high`, `/plan`, `/compact`, `/clear`. Unknown arguments open the picker.
 */
export function localCommand(text: string, controls: ChatControls): LocalCommand | null {
  const match = /^\/(\S+)(?:\s+(\S+))?\s*$/.exec(text.trim());
  if (!match) return null;
  const [, name, argument] = match;
  const command = controls.commands.find((c) => c.name === name);
  if (!command) return null;
  const action = command.action;
  switch (action.kind) {
    case "model":
      return argument && controls.models.some((m) => m.id === argument)
        ? { kind: "configure", setting: "model", value: argument }
        : { kind: "picker", picker: "model" };
    case "effort":
      return argument && effortsFor(controls).includes(argument)
        ? { kind: "configure", setting: "effort", value: argument }
        : { kind: "picker", picker: "effort" };
    case "mode":
      return { kind: "configure", setting: "mode", value: action.mode };
    case "compact":
      return { kind: "compact" };
    case "clear":
      return { kind: "clear" };
    case "insert":
      return null;
  }
}

const history = new Map<string, string[]>();
const drafts = new Map<string, string>();
const HISTORY_LIMIT = 50;

/** Sent prompts and unsent drafts per task agent, kept for the app session like the chat itself. */
export const promptMemory = {
  history: (key: string) => history.get(key) ?? [],
  remember(key: string, text: string) {
    const previous = (history.get(key) ?? []).filter((entry) => entry !== text);
    history.set(key, [...previous, text].slice(-HISTORY_LIMIT));
  },
  draft: (key: string) => drafts.get(key) ?? "",
  saveDraft(key: string, text: string) {
    if (text) drafts.set(key, text);
    else drafts.delete(key);
  },
};
