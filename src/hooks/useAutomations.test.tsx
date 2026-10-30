// @vitest-environment jsdom
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { AutomationSummary, loadAutomationSummaries } from "../services/automations";
import { useAutomations } from "./useAutomations";

vi.mock("../services/automations", () => ({
  loadAutomationSummaries: vi.fn(),
  listAutomationRuns: vi.fn(),
}));

afterEach(cleanup);

const named = (name: string) => ({ entry: { name } }) as AutomationSummary;

it("reloads on window focus and ignores responses of superseded loads", async () => {
  let resolveSlow: (value: AutomationSummary[]) => void = () => undefined;
  vi.mocked(loadAutomationSummaries)
    .mockResolvedValueOnce([named("first")])
    .mockReturnValueOnce(new Promise((resolve) => (resolveSlow = resolve)))
    .mockResolvedValueOnce([named("latest")])
    .mockRejectedValueOnce("Could not read /automations");
  const { result } = renderHook(() => useAutomations());
  await waitFor(() => expect(result.current.automations).toEqual([named("first")]));

  act(() => void window.dispatchEvent(new Event("focus")));
  act(() => void window.dispatchEvent(new Event("focus")));
  await waitFor(() => expect(result.current.automations).toEqual([named("latest")]));
  await act(async () => resolveSlow([named("stale")]));
  expect(result.current.automations).toEqual([named("latest")]);

  act(() => void window.dispatchEvent(new Event("focus")));
  await waitFor(() => expect(result.current.error).toBe("Could not read /automations"));
  expect(result.current.automations).toEqual([named("latest")]);
});
