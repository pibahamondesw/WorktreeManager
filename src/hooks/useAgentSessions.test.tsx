// @vitest-environment jsdom
import { act, cleanup, renderHook } from "@testing-library/react";
import { StrictMode } from "react";
import { afterEach, expect, it, vi } from "vitest";
import { TerminalStatus } from "../services/terminal";
import { useAgentSessions } from "./useAgentSessions";

vi.mock("../embedded/sessionTransports", () => ({ sessionRegistry: {} }));

afterEach(cleanup);

function fakeRegistry() {
  let statuses: Record<string, TerminalStatus> = {};
  const listeners = new Set<() => void>();
  return {
    subscribe: (listener: () => void) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    taskStatuses: () => statuses,
    refresh: vi.fn(() => listeners.size),
    publish(next: Record<string, TerminalStatus>) {
      statuses = next;
      listeners.forEach((listener) => listener());
    },
  };
}

it("follows the registry's statuses and lists again when the task view opens or closes", () => {
  const registry = fakeRegistry();
  const { result, rerender } = renderHook(({ taskOpen }) => useAgentSessions(taskOpen, registry), {
    initialProps: { taskOpen: false },
    wrapper: StrictMode,
  });
  act(() => registry.publish({ t1: { kind: "running" } }));
  expect(result.current).toEqual({ t1: { kind: "running" } });
  const refreshesBeforeOpen = registry.refresh.mock.calls.length;
  rerender({ taskOpen: true });
  expect(registry.refresh.mock.calls.length).toBeGreaterThan(refreshesBeforeOpen);
  expect(registry.refresh.mock.results.every(({ value }) => value === 1)).toBe(true);
});
