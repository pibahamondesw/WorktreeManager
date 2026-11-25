// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, cleanup, render } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { LiveElapsed } from "./LiveElapsed";
import { formatElapsed, useForgetRunWhenStopped } from "./runClock";

beforeEach(() => vi.useFakeTimers());
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

function Run({ runKey, running }: { runKey: string; running: boolean }) {
  useForgetRunWhenStopped(runKey, running);
  return running ? <LiveElapsed runKey={runKey} /> : null;
}

it.each([
  [0, "0s"],
  [59_999, "59s"],
  [65_000, "1m 05s"],
  [3_725_000, "1h 02m"],
  [-500, "0s"],
])("formats %d ms as %s", (ms, label) => {
  expect(formatElapsed(ms)).toBe(label);
});

it("counts up every second without re-rendering", () => {
  const onRender = vi.fn();
  function Counted() {
    onRender();
    return <LiveElapsed runKey="count" />;
  }
  const { container } = render(<Counted />);
  expect(container).toHaveTextContent("0s");

  act(() => {
    vi.advanceTimersByTime(3_000);
  });

  expect(container).toHaveTextContent("3s");
  expect(onRender).toHaveBeenCalledTimes(1);
});

it("keeps a run's start when it is shown again and restarts once it stops", () => {
  const first = render(<Run runKey="run" running />);
  act(() => {
    vi.advanceTimersByTime(5_000);
  });
  first.unmount();

  const again = render(<Run runKey="run" running />);
  expect(again.container).toHaveTextContent("5s");

  again.rerender(<Run runKey="run" running={false} />);
  again.rerender(<Run runKey="run" running />);
  expect(again.container).toHaveTextContent("0s");
});

it("stops ticking once no count is shown", () => {
  const { unmount } = render(<LiveElapsed runKey="idle" />);
  unmount();

  expect(vi.getTimerCount()).toBe(0);
});
