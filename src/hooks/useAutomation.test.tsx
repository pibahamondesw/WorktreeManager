// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { useAutomation } from "./useAutomation";
import { Operations } from "../services/operations";
import { DEFAULT_STATE } from "../types";

const { invoke, dispatch } = vi.hoisted(() => ({
  invoke: vi.fn().mockResolvedValue(undefined),
  dispatch: vi.fn().mockResolvedValue({ ok: true, data: [] }),
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke,
  Channel: class {
    onmessage = () => {};
  },
}));
vi.mock("../services/automation", () => ({ dispatchAutomation: dispatch }));
vi.mock("../services/store", () => ({ persist: vi.fn() }));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("frontend automation bridge", () => {
  it("registers only after state loads and forwards responses through the main interface", async () => {
    const operations = new Operations(
      () => DEFAULT_STATE,
      vi.fn(),
      () => "cursor"
    );
    const view = renderHook(({ ready }) => useAutomation(operations, ready), {
      initialProps: { ready: false },
    });
    expect(invoke).not.toHaveBeenCalled();
    view.rerender({ ready: true });
    const registration = invoke.mock.calls.find(
      ([command]) => command === "automation_register"
    )![1];
    const request = { id: "r1", version: 1, method: "workspace.list", params: {} };
    await act(async () => registration.channel.onmessage({ token: "token1", request }));
    expect(dispatch).toHaveBeenCalledWith(operations, request, expect.any(Function));
    expect(invoke).toHaveBeenCalledWith("automation_complete", {
      session: registration.session,
      token: "token1",
      response: { ok: true, data: [] },
    });
    view.unmount();
    expect(invoke).toHaveBeenCalledWith("automation_unregister", { session: registration.session });
  });

  it("cleans up a registration that completes after unmount", async () => {
    let register!: () => void;
    invoke.mockImplementationOnce(
      () =>
        new Promise<void>((resolve) => {
          register = resolve;
        })
    );
    const operations = new Operations(
      () => DEFAULT_STATE,
      vi.fn(),
      () => "cursor"
    );
    const view = renderHook(() => useAutomation(operations, true));
    view.unmount();
    register();
    await waitFor(() =>
      expect(
        invoke.mock.calls.filter(([command]) => command === "automation_unregister")
      ).toHaveLength(2)
    );
  });
});
