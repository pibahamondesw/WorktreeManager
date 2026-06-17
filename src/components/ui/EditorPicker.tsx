import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { CheckIcon, ChevronDownIcon } from "./Icons";
import { EditorApp, EDITOR_APPS } from "../../types";

interface EditorPickerProps {
  value: EditorApp;
  onChange: (editor: EditorApp) => void;
}

export function EditorPicker({ value, onChange }: EditorPickerProps) {
  const [open, setOpen] = useState(false);
  const [installedMap, setInstalledMap] = useState<Record<string, boolean>>({});
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    EDITOR_APPS.forEach(({ id }) => {
      invoke<boolean>("check_app_installed", { editor: id }).then((installed) =>
        setInstalledMap((prev) => ({ ...prev, [id]: installed }))
      );
    });
  }, []);

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open]);

  const current = EDITOR_APPS.find((e) => e.id === value)!;

  return (
    <div className="relative" ref={ref}>
      <button
        onClick={() => setOpen(!open)}
        className="flex items-center gap-1.5 h-8 px-2.5 rounded-lg text-xs text-text-secondary hover:text-text-primary hover:bg-bg-hover border border-transparent hover:border-border transition-colors cursor-pointer"
        title="Select default editor"
      >
        <EditorIcon editor={value} size={14} />
        <span className="max-w-[100px] truncate">{current.label}</span>
        <ChevronDownIcon className="opacity-50" />
      </button>

      {open && (
        <div className="absolute right-0 top-9 z-50 w-60 rounded-lg border border-border bg-bg-secondary shadow-xl py-1">
          {EDITOR_APPS.map((app) => {
            const installed = installedMap[app.id];
            const isSelected = app.id === value;
            return (
              <button
                key={app.id}
                onClick={() => {
                  if (installed === false) return;
                  onChange(app.id);
                  setOpen(false);
                }}
                className={`w-full flex items-center gap-2.5 px-3 py-2 text-xs transition-colors cursor-pointer ${
                  installed === false
                    ? "opacity-40 cursor-not-allowed"
                    : isSelected
                      ? "text-text-primary bg-bg-tertiary"
                      : "text-text-secondary hover:text-text-primary hover:bg-bg-hover"
                }`}
              >
                <EditorIcon editor={app.id} size={16} />
                <span className="flex-1 text-left">{app.label}</span>
                {installed === false && (
                  <span className="text-[10px] text-warning">not installed</span>
                )}
                {isSelected && <CheckIcon className="text-accent flex-shrink-0" />}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}

function EditorIcon({ editor, size }: { editor: EditorApp; size: number }) {
  const s = size;
  switch (editor) {
    case "cursor":
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="currentColor" className="flex-shrink-0">
          <path d="M2 2l12 6-12 6V9l7-1-7-1V2z" />
        </svg>
      );
    case "vscode":
      return (
        <svg width={s} height={s} viewBox="0 0 16 16" fill="currentColor" className="flex-shrink-0">
          <path d="M11.5 1L6 6.5 3.5 4.5 1 6.5v3L3.5 11.5 6 9.5 11.5 15 15 13.5v-11L11.5 1zM3.5 9.5l-1-1.5 1-1.5L6 9.5l-2.5 0zm8-5v7L7 8l4.5-3.5z" />
        </svg>
      );
    case "cursor-claude":
      return (
        <span className="flex items-center gap-0.5 flex-shrink-0" aria-hidden>
          <svg width={s} height={s} viewBox="0 0 16 16" fill="currentColor">
            <path d="M2 2l12 6-12 6V9l7-1-7-1V2z" />
          </svg>
          <ClaudeMiniIcon s={Math.max(10, s - 4)} />
        </span>
      );
    case "vscode-claude":
      return (
        <span className="flex items-center gap-0.5 flex-shrink-0" aria-hidden>
          <svg width={s} height={s} viewBox="0 0 16 16" fill="currentColor">
            <path d="M11.5 1L6 6.5 3.5 4.5 1 6.5v3L3.5 11.5 6 9.5 11.5 15 15 13.5v-11L11.5 1zM3.5 9.5l-1-1.5 1-1.5L6 9.5l-2.5 0zm8-5v7L7 8l4.5-3.5z" />
          </svg>
          <ClaudeMiniIcon s={Math.max(10, s - 4)} />
        </span>
      );
    case "claude-code":
      return (
        <svg
          width={s}
          height={s}
          viewBox="0 0 16 16"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.2"
          className="flex-shrink-0"
        >
          <rect x="1.5" y="3" width="13" height="10" rx="1.5" />
          <path d="M4 8.5l2 2 3.5-4" />
        </svg>
      );
    case "opencode":
      return (
        <svg
          width={s}
          height={s}
          viewBox="0 0 16 16"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.2"
          className="flex-shrink-0"
        >
          <rect x="1.5" y="3" width="13" height="10" rx="1.5" />
          <path d="M5 7l-1.5 1.5L5 10M11 7l1.5 1.5L11 10M8 6.5v4" />
        </svg>
      );
  }
}

function ClaudeMiniIcon({ s }: { s: number }) {
  return (
    <svg
      width={s}
      height={s}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.2"
      className="flex-shrink-0 opacity-80"
    >
      <rect x="1.5" y="3" width="13" height="10" rx="1.5" />
      <path d="M4 8.5l2 2 3.5-4" />
    </svg>
  );
}
