import { useEffect, useRef } from "react";

interface ShortcutDef {
  handler: () => void;
  enabled?: boolean;
  /**
   * Fire even when the keystroke originates in a text field. Only for meta combos that
   * must work while typing (opening the search palette); bare keys stay blocked so
   * typing never triggers them.
   */
  inTextFields?: boolean;
}

interface ParsedShortcut {
  key: string;
  meta: boolean;
  shift: boolean;
  def: ShortcutDef;
}

export function parseKey(combo: string): { key: string; meta: boolean; shift: boolean } {
  const parts = combo.split("+");
  const key = parts.pop()!;
  const meta = parts.includes("meta");
  const shift = parts.includes("shift");
  return { key, meta, shift };
}

// Shift turns single-character keys uppercase, so match those case-insensitively.
export function keyMatches(eventKey: string, shortcutKey: string): boolean {
  return shortcutKey.length === 1
    ? eventKey.toLowerCase() === shortcutKey.toLowerCase()
    : eventKey === shortcutKey;
}

function isTextEntryTarget(target: EventTarget | null): boolean {
  return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement;
}

export function useKeyboardShortcuts(
  shortcuts: Record<string, ShortcutDef>,
  options?: { enabled?: boolean }
): void {
  const parsedRef = useRef<ParsedShortcut[]>([]);
  const optionsRef = useRef(options);

  useEffect(() => {
    optionsRef.current = options;
    parsedRef.current = Object.entries(shortcuts).map(([combo, def]) => ({
      ...parseKey(combo),
      def,
    }));
  });

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (optionsRef.current?.enabled === false) return;

      const isMeta = e.metaKey || e.ctrlKey;
      const inTextField = isTextEntryTarget(e.target);

      for (const s of parsedRef.current) {
        if (s.def.enabled === false) continue;
        if (inTextField && !(s.meta && s.def.inTextFields)) continue;
        if (!keyMatches(e.key, s.key)) continue;
        if (s.meta !== isMeta) continue;
        if (s.shift !== e.shiftKey) continue;

        e.preventDefault();
        s.def.handler();
        return;
      }
    };

    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);
}
