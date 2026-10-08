export type Modifier = "meta" | "ctrl" | "alt" | "shift";
export type Shortcut = { key: string; modifiers: Modifier[] };
export type ShortcutOverrides = Record<string, Shortcut[]>;
export type ShortcutContext = "list" | "task" | "chat" | "palette" | "modal";
export interface Binding {
  id: string;
  command: string;
  label: string;
  category: string;
  contexts: ShortcutContext[];
  defaults: Shortcut[];
  editable: boolean;
  numberedGroup?: string;
  inText?: boolean;
  bareInPalette?: boolean;
  repeat?: boolean;
  native?: "back" | "search" | "history-back" | "history-forward";
}

const app: ShortcutContext[] = ["list", "task", "chat"];
const everywhere: ShortcutContext[] = [...app, "palette", "modal"];
export const shortcut = (key: string, ...modifiers: Modifier[]): Shortcut => ({ key, modifiers });
const primary = (key: string, ...rest: Modifier[]) => [
  shortcut(key, "meta", ...rest),
  shortcut(key, "ctrl", ...rest),
];
function binding(
  id: string,
  label: string,
  category: string,
  contexts: ShortcutContext[],
  defaults: Shortcut[],
  options: Partial<Binding> = {}
): Binding {
  return { id, command: id, label, category, contexts, defaults, editable: true, ...options };
}
const reference = (
  id: string,
  label: string,
  contexts: ShortcutContext[],
  defaults: Shortcut[],
  options: Partial<Binding> = {}
) => binding(id, label, "Navigation", contexts, defaults, { editable: false, ...options });
export const bindings: Binding[] = [
  binding("app.workspace.add", "Add workspace", "Application", app, [shortcut("p")]),
  binding("app.reload", "Reload application", "Application", app, primary("r", "shift")),
  binding("app.sidebar", "Toggle sidebar", "Application", app, [shortcut("["), shortcut("{")]),
  binding("app.settings", "Open settings", "Application", app, primary("s")),
  binding("palette.settings", "Open settings", "Palette", ["palette"], primary("s"), {
    command: "app.settings",
    inText: true,
  }),
  binding("app.search", "Search tasks", "Application", [...app, "palette"], primary("k"), {
    native: "search",
    inText: true,
  }),
  binding(
    "app.search.workspace",
    "Search current workspace",
    "Application",
    [...app, "palette"],
    primary("f"),
    { inText: true }
  ),
  binding("app.history.back", "Previous location", "Application", app, primary("ArrowLeft"), {
    native: "history-back",
  }),
  binding("app.history.forward", "Next location", "Application", app, primary("ArrowRight"), {
    native: "history-forward",
  }),
  binding("native.back", "Back to tasks", "Task", everywhere, primary("["), {
    native: "back",
    inText: true,
  }),
  binding(
    "native.search",
    "Search tasks (including editor)",
    "Application",
    everywhere,
    primary("p", "alt"),
    { native: "search", inText: true }
  ),
  binding("list.new", "New task", "Tasks", ["list"], [shortcut("n")]),
  binding("palette.new", "New task", "Palette", ["palette"], [shortcut("n")], {
    command: "list.new",
    bareInPalette: true,
  }),
  binding("list.refresh", "Refresh tasks", "Tasks", ["list"], primary("r")),
  binding("list.next", "Next task", "Tasks", ["list"], [shortcut("j")], { repeat: true }),
  binding("list.previous", "Previous task", "Tasks", ["list"], [shortcut("k")], { repeat: true }),
  binding("list.linear", "Open Linear issue", "Tasks", ["list"], [shortcut("l")]),
  binding("list.note", "Open task note", "Tasks", ["list"], [shortcut("o")]),
  binding("list.delete", "Delete task", "Tasks", ["list"], primary("d")),
  binding("list.branch", "Copy branch name", "Tasks", ["list"], primary("b")),
  binding("list.path", "Copy worktree paths", "Tasks", ["list"], primary("c", "shift")),
  binding(
    "palette.linear",
    "Open selected Linear issue",
    "Palette",
    ["palette"],
    [shortcut("l", "meta")],
    { command: "list.linear", inText: true }
  ),
  binding(
    "palette.reveal",
    "Reveal selected task",
    "Palette",
    ["palette"],
    [shortcut("Enter", "meta")],
    { inText: true }
  ),
  binding("zoom.in", "Zoom in", "Appearance", everywhere, [...primary("+"), ...primary("=")], {
    inText: true,
    repeat: true,
  }),
  binding("zoom.out", "Zoom out", "Appearance", everywhere, primary("-"), {
    inText: true,
    repeat: true,
  }),
  ...(
    [
      ["mode", "m"],
      ["model", "i"],
      ["effort", "e"],
    ] as const
  ).map(([name, key]) =>
    binding(`chat.${name}`, `Choose ${name}`, "Chat", ["chat"], [shortcut(key, "meta", "shift")], {
      inText: true,
    })
  ),
  ...Array.from({ length: 10 }, (_, i) => [
    binding(`workspace.${i}`, `Switch to workspace ${i}`, "Workspaces", app, primary(String(i)), {
      editable: false,
      numberedGroup: "workspace",
    }),
    binding(
      `palette.workspace.${i}`,
      `Switch to workspace ${i}`,
      "Palette",
      ["palette"],
      [shortcut(String(i), "meta", "shift")],
      {
        command: `workspace.${i}`,
        inText: true,
        editable: false,
        numberedGroup: "palette.workspace",
      }
    ),
    binding(`list.jump.${i}`, `Jump to task ${i}`, "Tasks", ["list"], [shortcut(String(i))], {
      editable: false,
      numberedGroup: "list.jump",
    }),
    binding(
      `palette.jump.${i}`,
      `Jump to task result ${i}`,
      "Palette",
      ["palette"],
      [shortcut(String(i), "meta")],
      { inText: true, editable: false, numberedGroup: "palette.jump" }
    ),
  ]).flat(),
  reference("list.down", "Next task", ["list"], [shortcut("ArrowDown")], { repeat: true }),
  reference("list.up", "Previous task", ["list"], [shortcut("ArrowUp")], { repeat: true }),
  reference("list.open", "Open selected task", ["list"], [shortcut("Enter")]),
  reference("list.clear", "Clear selection", ["list"], [shortcut("Escape")]),
  reference("terminal.restart", "Restart ended terminal", ["task"], [shortcut("Enter")]),
  reference("task.close", "Back to tasks", ["task", "chat"], [shortcut("Escape")]),
  reference(
    "navigation.tab",
    "Move focus",
    everywhere,
    [shortcut("Tab"), shortcut("Tab", "shift")],
    { inText: true }
  ),
  reference(
    "navigation.close",
    "Close dialog or menu",
    ["modal", "palette"],
    [shortcut("Escape")],
    { inText: true }
  ),
  reference(
    "palette.down",
    "Next result",
    ["palette"],
    [shortcut("ArrowDown"), shortcut("n", "ctrl")],
    { inText: true }
  ),
  reference(
    "palette.up",
    "Previous result",
    ["palette"],
    [shortcut("ArrowUp"), shortcut("p", "ctrl")],
    { inText: true }
  ),
  reference("palette.open", "Open selected result", ["palette"], [shortcut("Enter")], {
    inText: true,
  }),
  reference("chat.submit", "Send message / confirm option", ["chat"], [shortcut("Enter")], {
    inText: true,
  }),
  reference("chat.newline", "Insert newline", ["chat"], [shortcut("Enter", "shift")], {
    inText: true,
  }),
  reference(
    "chat.cancel",
    "Close picker / stop response / leave composer",
    ["chat"],
    [shortcut("Escape")],
    { inText: true }
  ),
  reference(
    "chat.history",
    "Prompt history / navigate options",
    ["chat"],
    [shortcut("ArrowUp"), shortcut("ArrowDown")],
    { inText: true }
  ),
  reference(
    "chat.options",
    "Select numbered option / approval",
    ["chat"],
    Array.from({ length: 9 }, (_, i) => shortcut(String(i + 1)))
  ),
  reference(
    "modal.navigation",
    "Navigate options",
    ["modal"],
    [
      shortcut("ArrowUp"),
      shortcut("ArrowDown"),
      shortcut("Home"),
      shortcut("End"),
      shortcut("n", "ctrl"),
      shortcut("p", "ctrl"),
    ],
    { inText: true }
  ),
  reference("system.quit", "Quit application", everywhere, [shortcut("q", "meta")], {
    inText: true,
  }),
  reference(
    "system.hide",
    "Hide application",
    everywhere,
    [shortcut("h", "meta"), shortcut("h", "meta", "alt")],
    { inText: true }
  ),
  reference("system.minimize", "Minimize window", everywhere, [shortcut("m", "meta")], {
    inText: true,
  }),
  reference(
    "system.fullscreen",
    "Toggle full screen",
    everywhere,
    [shortcut("f", "meta", "ctrl")],
    { inText: true }
  ),
  reference(
    "system.edit",
    "Copy, cut, paste, select all, undo and redo",
    everywhere,
    [
      shortcut("c", "meta"),
      shortcut("x", "meta"),
      shortcut("v", "meta"),
      shortcut("a", "meta"),
      shortcut("z", "meta"),
      shortcut("z", "meta", "shift"),
    ],
    { inText: true }
  ),
  reference(
    "modal.numbered",
    "Select numbered issue or option",
    ["modal"],
    Array.from({ length: 10 }, (_, i) => shortcut(String(i)))
  ),
  reference(
    "modal.confirm",
    "Confirm focused control",
    ["modal"],
    [shortcut("Enter"), shortcut(" ")],
    { inText: true }
  ),
];
export const bindingById = new Map(bindings.map((entry) => [entry.id, entry]));
function isBareNumber(value: Shortcut): boolean {
  return /^[0-9]$/.test(value.key) && !value.modifiers.some((modifier) => modifier !== "shift");
}
export function effectiveBindings(entry: Binding, overrides: ShortcutOverrides): Shortcut[] {
  if (!entry.editable) return entry.defaults;
  return (overrides[entry.id] ?? entry.defaults).filter((value) => !isBareNumber(value));
}
const modifierOrder: Modifier[] = ["ctrl", "alt", "shift", "meta"];
export function canonical(value: Shortcut): Shortcut {
  return {
    key: value.key.length === 1 ? value.key.toLowerCase() : value.key,
    modifiers: modifierOrder.filter((m) => value.modifiers.includes(m)),
  };
}
export function signature(value: Shortcut): string {
  return JSON.stringify(canonical(value));
}
const isSymbol = (key: string) => key.length === 1 && /^[^\p{L}\p{N}\s]$/u.test(key);
export function overlaps(a: Shortcut, b: Shortcut): boolean {
  if (canonical(a).key !== canonical(b).key) return false;
  return modifierOrder.every(
    (m) =>
      (m === "shift" &&
        isSymbol(a.key) &&
        (!a.modifiers.includes(m) || !b.modifiers.includes(m))) ||
      a.modifiers.includes(m) === b.modifiers.includes(m)
  );
}
export function eventShortcut(
  event: Pick<KeyboardEvent, "key" | "code" | "metaKey" | "ctrlKey" | "altKey" | "shiftKey">
): Shortcut {
  return canonical({
    key: event.code === "NumpadSubtract" ? "-" : event.key,
    modifiers: modifierOrder.filter((m) => event[`${m}Key`]),
  });
}
export function matches(event: KeyboardEvent, value: Shortcut): boolean {
  const actual = eventShortcut(event);
  if (actual.key !== canonical(value).key) return false;
  return modifierOrder.every(
    (m) =>
      (m === "shift" && isSymbol(value.key) && !value.modifiers.includes(m)) ||
      actual.modifiers.includes(m) === value.modifiers.includes(m)
  );
}
export function formatShortcut(value: Shortcut): string {
  const symbols = { ctrl: "⌃", alt: "⌥", shift: "⇧", meta: "⌘" };
  const keys: Record<string, string> = {
    Enter: "↵",
    Escape: "Esc",
    ArrowUp: "↑",
    ArrowDown: "↓",
    ArrowLeft: "←",
    ArrowRight: "→",
    " ": "Space",
  };
  return (
    canonical(value)
      .modifiers.map((m) => symbols[m])
      .join("") + (keys[value.key] ?? value.key.toUpperCase())
  );
}
export function preferredShortcut(values: Shortcut[]): Shortcut | undefined {
  const candidates = values.filter((value) => value.modifiers.includes("meta"));
  return [...(candidates.length ? candidates : values)].sort(
    (a, b) => a.modifiers.length - b.modifiers.length
  )[0];
}
export function shortcutDisplayValues(entry: Binding, overrides: ShortcutOverrides): Shortcut[] {
  const values = effectiveBindings(entry, overrides);
  if (!entry.editable || Object.hasOwn(overrides, entry.id)) return values;
  const preferred = preferredShortcut(values);
  return preferred ? [preferred] : [];
}
export function compactShortcutLabels(values: Shortcut[]): string[] {
  if (
    values.length > 1 &&
    values.every(
      (value) =>
        /^[0-9]$/.test(value.key) &&
        signature({ ...value, key: "0" }) === signature({ ...values[0], key: "0" })
    )
  ) {
    const digits = values.map((value) => Number(value.key)).sort((a, b) => a - b);
    if (digits.every((digit, i) => digit === digits[0] + i))
      return [formatShortcut({ ...values[0], key: `${digits[0]}–${digits.at(-1)}` })];
  }
  return values.map(formatShortcut);
}
function isShortcut(value: unknown): value is Shortcut {
  if (!value || typeof value !== "object") return false;
  const v = value as Record<string, unknown>;
  return (
    typeof v.key === "string" &&
    validKey(v.key) &&
    Array.isArray(v.modifiers) &&
    v.modifiers.every(
      (m: unknown) => typeof m === "string" && modifierOrder.includes(m as Modifier)
    )
  );
}
export function normalizeOverrides(raw: unknown): ShortcutOverrides {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return {};
  return Object.fromEntries(
    Object.entries(raw).flatMap(([id, values]) => {
      if (!Array.isArray(values) || !values.every(isShortcut)) return [];
      return [
        [id, [...new Map(values.map((v: Shortcut) => [signature(v), canonical(v)])).values()]],
      ];
    })
  );
}
export function validKey(key: string): boolean {
  return (
    key.length === 1 ||
    /^(Arrow(Up|Down|Left|Right)|Enter|Escape|Tab|Backspace|Delete|Home|End|PageUp|PageDown|F([1-9]|1[0-9]|2[0-4]))$/.test(
      key
    )
  );
}
export function conflicts(
  entry: Binding,
  values: Shortcut[],
  overrides: ShortcutOverrides
): Binding[] {
  return bindings.filter(
    (other) =>
      other.id !== entry.id &&
      entry.contexts.some((c) => other.contexts.includes(c)) &&
      values.some((a) => effectiveBindings(other, overrides).some((b) => overlaps(a, b)))
  );
}
export function nativeKeySupported(key: string): boolean {
  return key.length > 1 ? validKey(key) : /^[a-z0-9]$/i.test(key) || " `\\[],./;'=+-".includes(key);
}

export function updateBinding(
  overrides: ShortcutOverrides,
  id: string,
  values: Shortcut[] | null,
  reassign = false
): ShortcutOverrides {
  const entry = bindingById.get(id);
  if (!entry?.editable) throw new Error("This binding is reserved for navigation.");
  const normalized = normalizeOverrides({ [id]: values ?? entry.defaults })[id];
  if (!normalized) throw new Error("Use modifiers and a single key.");
  if (normalized.some(isBareNumber))
    throw new Error("Numbers without Cmd, Ctrl or Option are reserved for typing and navigation.");
  if (
    entry.inText &&
    !entry.bareInPalette &&
    normalized.some((v) => !v.modifiers.some((m) => m !== "shift"))
  )
    throw new Error("This action needs Cmd, Ctrl or Option so it does not interrupt typing.");
  if (entry.native && normalized.some((v) => !nativeKeySupported(v.key)))
    throw new Error("This key cannot be used by the native menu. Choose another combination.");
  const collisions = conflicts(entry, normalized, overrides);
  if (collisions.some((c) => !c.editable))
    throw new Error(
      `Reserved for ${collisions
        .filter((c) => !c.editable)
        .map((c) => c.label)
        .join(", ")}.`
    );
  if (collisions.length && !reassign)
    throw new Error("This shortcut is already assigned. Confirm reassignment first.");
  const next = { ...overrides };
  for (const other of collisions)
    next[other.id] = effectiveBindings(other, next).filter(
      (b) => !normalized.some((a) => overlaps(a, b))
    );
  next[id] = normalized;
  if (
    normalized.length === entry.defaults.length &&
    normalized.every((a) => entry.defaults.some((b) => signature(a) === signature(b)))
  )
    delete next[id];
  return next;
}
