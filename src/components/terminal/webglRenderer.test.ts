import { afterEach, describe, expect, it, vi } from "vitest";
import type { Terminal } from "@xterm/xterm";

const webgl = vi.hoisted(() => ({
  dispose: vi.fn(),
  loseContext: (): void => undefined,
}));

vi.mock("@xterm/addon-webgl", () => ({
  WebglAddon: vi.fn(function () {
    return {
      dispose: webgl.dispose,
      onContextLoss: (listener: () => void) => {
        webgl.loseContext = listener;
      },
    };
  }),
}));

import { loadWebglRenderer } from "./webglRenderer";

const terminal = () =>
  ({ loadAddon: vi.fn() }) as unknown as Terminal & { loadAddon: ReturnType<typeof vi.fn> };

afterEach(() => {
  vi.clearAllMocks();
  vi.restoreAllMocks();
});

describe("loadWebglRenderer", () => {
  it("renders the terminal with WebGL", () => {
    const term = terminal();
    loadWebglRenderer(term);
    expect(term.loadAddon).toHaveBeenCalledOnce();
    expect(webgl.dispose).not.toHaveBeenCalled();
  });

  it("returns to the DOM renderer when the WebGL context is lost", () => {
    loadWebglRenderer(terminal());
    webgl.loseContext();
    expect(webgl.dispose).toHaveBeenCalledOnce();
  });

  it("keeps the DOM renderer when WebGL cannot load", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    const term = terminal();
    term.loadAddon.mockImplementation(() => {
      throw new Error("WebGL2 not supported");
    });
    loadWebglRenderer(term);
    expect(warn).toHaveBeenCalledOnce();
  });
});
