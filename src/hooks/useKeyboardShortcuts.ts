import { useEffect, useRef } from "react";

interface ShortcutDef {
  handler: () => void;
  enabled?: boolean;
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
      if (
        e.target instanceof HTMLInputElement ||
        e.target instanceof HTMLTextAreaElement
      ) {
        return;
      }

      if (optionsRef.current?.enabled === false) return;

      const isMeta = e.metaKey || e.ctrlKey;

      for (const s of parsedRef.current) {
        if (s.def.enabled === false) continue;
        if (e.key !== s.key) continue;
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
