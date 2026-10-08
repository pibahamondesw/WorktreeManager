import { useEffect, useLayoutEffect, useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  bindingById,
  bindings,
  effectiveBindings,
  formatShortcut,
  preferredShortcut,
  matches,
  overlaps,
  nativeKeySupported,
  type ShortcutContext,
  type ShortcutOverrides,
} from "./catalog";

export interface ShortcutAction {
  handler: () => void;
  enabled?: boolean;
  inTextFields?: boolean;
}
let overrides: ShortcutOverrides = {};
let capturing = false;
let listeners = 0;
const isTauri = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
const subscribers = new Set<() => void>();
const registrations = new Map<symbol, Record<string, ShortcutAction>>();
function subscribe(listener: () => void) {
  subscribers.add(listener);
  return () => {
    subscribers.delete(listener);
  };
}
export function setShortcutOverrides(value: ShortcutOverrides) {
  overrides = value;
  subscribers.forEach((listener) => listener());
}
export function useShortcutOverrides() {
  return useSyncExternalStore(subscribe, () => overrides);
}
export function shortcutLabel(id: string, values = overrides): string {
  const entry = bindingById.get(id);
  const preferred = entry ? preferredShortcut(effectiveBindings(entry, values)) : undefined;
  return preferred ? formatShortcut(preferred) : "";
}
export function useShortcutLabels() {
  const values = useShortcutOverrides();
  return (id: string) => shortcutLabel(id, values);
}
function currentContext(eventTarget: EventTarget | null = document.activeElement): ShortcutContext {
  const dialogs = Array.from(document.querySelectorAll<HTMLElement>('[role="dialog"]')).filter(
    (e) => !e.closest("[hidden]")
  );
  const dialog = dialogs.at(-1);
  if (dialog) return dialog.dataset.shortcutContext === "palette" ? "palette" : "modal";
  const target =
    eventTarget instanceof HTMLElement
      ? eventTarget.closest<HTMLElement>("[data-shortcut-context]")
      : document.activeElement?.closest<HTMLElement>("[data-shortcut-context]");
  return (target?.dataset.shortcutContext ??
    document
      .querySelector('[data-shortcut-context="task"]')
      ?.getAttribute("data-shortcut-context") ??
    "list") as ShortcutContext;
}
function textTarget(target: EventTarget | null) {
  return (
    target instanceof HTMLElement &&
    (target.matches("input, textarea, select") ||
      target.isContentEditable ||
      !!target.closest(".xterm, [contenteditable=true]"))
  );
}
export function dispatchShortcut(event: KeyboardEvent, captureOnly = false) {
  if (
    event.defaultPrevented ||
    event.isComposing ||
    ["Dead", "Process", "Unidentified"].includes(event.key)
  )
    return;
  if (capturing) return;
  const context = currentContext(event.target);
  const text = textTarget(event.target);
  const candidates = bindings.filter(
    (entry) =>
      entry.contexts.includes(context) &&
      effectiveBindings(entry, overrides).some((s) => matches(event, s))
  );
  const eligible = candidates.filter((entry) => {
    if (text && !entry.inText && !entry.bareInPalette) return false;
    if (text && !entry.bareInPalette && !event.metaKey && !event.ctrlKey && !event.altKey)
      return false;
    if (
      entry.id.startsWith("chat.") &&
      entry.editable &&
      !(
        event.target instanceof HTMLElement &&
        event.target.closest('[data-shortcut-context="chat"]')
      )
    )
      return false;
    if (
      event.target instanceof HTMLElement &&
      event.target.closest("button, a, [role=button]") &&
      ["Enter", " "].includes(event.key) &&
      !event.metaKey &&
      !event.ctrlKey &&
      !event.altKey
    )
      return false;
    return true;
  });
  const protectedEntries = eligible.filter((entry) => !entry.editable);
  const custom = protectedEntries.length
    ? []
    : eligible.filter((entry) => Object.hasOwn(overrides, entry.id) && entry.editable);
  const choices = custom.length ? custom : protectedEntries.length ? protectedEntries : eligible;
  if (custom.length > 1) {
    event.preventDefault();
    event.stopImmediatePropagation();
    return;
  }
  const entry = choices.find((candidate) =>
    [...registrations.values()].some((actions) => actions[candidate.id])
  );
  if (!entry || (entry.native && isTauri())) return;
  if (captureOnly && entry.category !== "Appearance" && !entry.id.startsWith("palette.jump."))
    return;
  if (event.repeat && !entry.repeat) return;
  const action = [...registrations.values()].map((actions) => actions[entry.id]).find(Boolean);
  if (!action || action.enabled === false) return;
  if (
    text &&
    entry.bareInPalette &&
    !event.metaKey &&
    !event.ctrlKey &&
    !event.altKey &&
    !action.inTextFields
  )
    return;
  event.preventDefault();
  event.stopImmediatePropagation();
  action.handler();
}
const captureDispatcher = (event: KeyboardEvent) => dispatchShortcut(event, true);
const bubbleDispatcher = (event: KeyboardEvent) => dispatchShortcut(event);
export function useShortcutActions(actions: Record<string, ShortcutAction>, enabled = true) {
  useLayoutEffect(() => {
    const id = Symbol();
    registrations.set(id, enabled ? actions : {});
    return () => {
      registrations.delete(id);
    };
  });
  useEffect(() => {
    if (listeners++ === 0) {
      window.addEventListener("keydown", bubbleDispatcher);
      window.addEventListener("keydown", captureDispatcher, true);
      startContextObserver();
    }
    return () => {
      if (--listeners === 0) {
        window.removeEventListener("keydown", bubbleDispatcher);
        window.removeEventListener("keydown", captureDispatcher, true);
        stopContextObserver();
      }
    };
  }, []);
}
export function nativeBindings(values: ShortcutOverrides) {
  return bindings
    .filter((entry) => entry.native)
    .flatMap((entry) =>
      effectiveBindings(entry, values)
        .filter(
          (value) => nativeKeySupported(value.key) && value.modifiers.some((m) => m !== "shift")
        )
        .map((value, index) => ({
          id: `${entry.id}.${index}`,
          action: entry.native!,
          inText: !!entry.inText,
          key: value.key,
          modifiers: value.modifiers,
          contexts: entry.contexts.filter(
            (context) =>
              !bindings.some(
                (other) =>
                  other.id !== entry.id &&
                  other.contexts.includes(context) &&
                  (Object.hasOwn(values, other.id) || !other.editable) &&
                  effectiveBindings(other, values).some((b) => overlaps(value, b))
              )
          ),
        }))
    );
}
let nativeQueue: Promise<unknown> = Promise.resolve();
function nativeChange(command: string, args: Record<string, unknown>): Promise<void> {
  const result = nativeQueue.then(async () => {
    if (isTauri()) await invoke(command, args);
  });
  nativeQueue = result.catch(() => {});
  return result;
}
let contextObserver: MutationObserver | undefined;
let nativeContext: string | undefined;
function refreshNativeContext() {
  const next = currentContext();
  const editing = textTarget(document.activeElement);
  const signature = JSON.stringify([next, editing]);
  if (signature === nativeContext) return;
  nativeContext = signature;
  void nativeChange("set_shortcut_context", { context: next, editing }).catch(() => {
    nativeContext = undefined;
  });
}
function startContextObserver() {
  contextObserver = new MutationObserver(refreshNativeContext);
  contextObserver.observe(document.body, {
    childList: true,
    subtree: true,
    attributes: true,
    attributeFilter: ["hidden", "data-shortcut-context"],
  });
  document.addEventListener("focusin", refreshNativeContext);
  document.addEventListener("focusout", refreshNativeContext);
  refreshNativeContext();
}
function stopContextObserver() {
  contextObserver?.disconnect();
  document.removeEventListener("focusin", refreshNativeContext);
  document.removeEventListener("focusout", refreshNativeContext);
  nativeContext = undefined;
}
export function syncNativeShortcuts(values: ShortcutOverrides) {
  return nativeChange("set_shortcut_menu", { bindings: nativeBindings(values) });
}
export async function setShortcutCapture(active: boolean) {
  capturing = active;
  try {
    await nativeChange("set_shortcut_capture", { active });
  } catch (error) {
    capturing = !active;
    throw error;
  }
}
export function isShortcutCaptureActive() {
  return capturing;
}
