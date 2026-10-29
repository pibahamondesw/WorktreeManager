// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { useCliBridge } from "./useCliBridge";
import { Operations } from "../services/operations";
import type { invoke as tauriInvoke } from "@tauri-apps/api/core";
import type { CliBridgeRequest } from "../services/cliBridge";
import { DEFAULT_STATE } from "../types";

const { invoke, dispatch, channels } = vi.hoisted(() => ({
  invoke: vi.fn<typeof tauriInvoke>().mockResolvedValue(undefined),
  channels: [] as { onmessage: (event: { token: string; request: CliBridgeRequest }) => void }[],
  dispatch: vi.fn().mockResolvedValue({ ok: true, data: [] }),
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke,
  Channel: class {
    onmessage: (event: { token: string; request: CliBridgeRequest }) => void = () => {};
    constructor() {
      channels.push(this);
    }
  },
}));
vi.mock("../services/cliBridge", () => ({ dispatchCliBridge: dispatch }));
vi.mock("../services/store", () => ({ persist: vi.fn() }));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  channels.length = 0;
});

describe("frontend cli bridge", () => {
  it("registers only after state loads and forwards responses through the main interface", async () => {
    const operations = new Operations(
      () => DEFAULT_STATE,
      vi.fn(),
      () => "cursor"
    );
    const view = renderHook(({ ready }) => useCliBridge(operations, ready), {
      initialProps: { ready: false },
    });
    expect(invoke).not.toHaveBeenCalled();
    view.rerender({ ready: true });
    const registration = invoke.mock.calls.find(
      ([command]) => command === "cli_bridge_register"
    )![1]!;
    if (!("session" in registration)) throw new Error("CLI bridge registration is missing");
    const request = { id: "r1", version: 1, method: "workspace.list", params: {} };
    await act(async () => channels[0].onmessage({ token: "token1", request }));
    expect(dispatch).toHaveBeenCalledWith(operations, request, expect.any(Function), undefined);
    expect(invoke).toHaveBeenCalledWith("cli_bridge_complete", {
      session: registration.session,
      token: "token1",
      response: { ok: true, data: [] },
    });
    view.unmount();
    expect(invoke).toHaveBeenCalledWith("cli_bridge_unregister", { session: registration.session });
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
    const view = renderHook(() => useCliBridge(operations, true));
    view.unmount();
    register();
    await waitFor(() =>
      expect(
        invoke.mock.calls.filter(([command]) => command === "cli_bridge_unregister")
      ).toHaveLength(2)
    );
  });
});
