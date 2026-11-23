// @vitest-environment jsdom
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  observe: vi.fn(),
  unobserve: vi.fn(),
  notify: null as ((entries: ResizeObserverEntry[]) => void) | null,
}));

vi.mock("react-dom", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react-dom")>();
  return { ...actual, flushSync: vi.fn(actual.flushSync) };
});

import { flushSync } from "react-dom";
import { observeResize } from "./observeResize";

beforeAll(() => {
  globalThis.ResizeObserver = class {
    constructor(callback: (entries: ResizeObserverEntry[]) => void) {
      mocks.notify = callback;
    }
    observe = mocks.observe;
    unobserve = mocks.unobserve;
    disconnect() {}
  } as never;
});

afterEach(() => vi.clearAllMocks());

function resize(...targets: Element[]) {
  mocks.notify?.(targets.map((target) => ({ target }) as ResizeObserverEntry));
}

describe("observeResize", () => {
  it("runs the callbacks of every resized element inside one flushSync", () => {
    const pane = document.createElement("div");
    const terminal = document.createElement("div");
    const order: string[] = [];
    const stopPane = observeResize(pane, () => order.push("pane"));
    const stopTerminal = observeResize(terminal, () => order.push("terminal"));

    resize(pane, terminal);

    expect(order).toEqual(["pane", "terminal"]);
    expect(flushSync).toHaveBeenCalledTimes(1);
    stopPane();
    stopTerminal();
  });

  it("observes an element once and stops after its last callback unsubscribes", () => {
    const element = document.createElement("div");
    const first = vi.fn();
    const second = vi.fn();
    const stopFirst = observeResize(element, first);
    const stopSecond = observeResize(element, second);
    expect(mocks.observe).toHaveBeenCalledTimes(1);

    stopFirst();
    expect(mocks.unobserve).not.toHaveBeenCalled();
    resize(element);
    expect(first).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledTimes(1);

    stopSecond();
    expect(mocks.unobserve).toHaveBeenCalledWith(element);
    resize(element);
    expect(second).toHaveBeenCalledTimes(1);
  });

  it("keeps a newer subscription when an old unsubscribe runs again", () => {
    const element = document.createElement("div");
    const stale = observeResize(element, vi.fn());
    stale();
    const current = vi.fn();
    const stopCurrent = observeResize(element, current);

    stale();
    resize(element);

    expect(mocks.unobserve).toHaveBeenCalledTimes(1);
    expect(current).toHaveBeenCalledTimes(1);
    stopCurrent();
  });
});
