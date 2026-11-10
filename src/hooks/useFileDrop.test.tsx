// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { act, cleanup, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { useRef } from "react";
import { useFileDrop } from "./useFileDrop";

type Payload =
  | { type: "enter"; paths: string[]; position: { x: number; y: number } }
  | { type: "over" | "drop"; paths?: string[]; position: { x: number; y: number } }
  | { type: "leave" };

const mocks = vi.hoisted(() => ({
  handlers: new Set<(event: { payload: Payload }) => void>(),
}));

vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: (handler: (event: { payload: Payload }) => void) => {
      mocks.handlers.add(handler);
      return Promise.resolve(() => mocks.handlers.delete(handler));
    },
  }),
}));

afterEach(cleanup);

const emit = (payload: Payload) =>
  act(() => mocks.handlers.forEach((handler) => handler({ payload })));

function Target({
  onDrop,
  enabled = true,
}: {
  onDrop: (paths: string[]) => void;
  enabled?: boolean;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const hovering = useFileDrop(ref, onDrop, enabled);
  return <div ref={ref}>{hovering ? "hovering" : "idle"}</div>;
}

function renderTarget(enabled = true) {
  const onDrop = vi.fn();
  const view = render(<Target onDrop={onDrop} enabled={enabled} />);
  const target = screen.getByText("idle");
  document.elementFromPoint = (x: number, y: number) =>
    x >= 10 && x <= 110 && y >= 10 && y <= 60 ? target : document.body;
  return { onDrop, view };
}

it("tracks file hovers over the element and delivers drops inside it", async () => {
  const { onDrop } = renderTarget();
  await emit({ type: "enter", paths: ["/a.pdf"], position: { x: 0, y: 0 } });
  expect(screen.getByText("idle")).toBeInTheDocument();
  await emit({ type: "over", position: { x: 50, y: 30 } });
  expect(screen.getByText("hovering")).toBeInTheDocument();
  await emit({ type: "leave" });
  expect(screen.getByText("idle")).toBeInTheDocument();

  await emit({ type: "enter", paths: ["/a.pdf"], position: { x: 50, y: 30 } });
  await emit({ type: "drop", paths: ["/a.pdf"], position: { x: 500, y: 30 } });
  expect(onDrop).not.toHaveBeenCalled();
  await emit({ type: "enter", paths: ["/a.pdf"], position: { x: 50, y: 30 } });
  await emit({ type: "drop", paths: ["/a.pdf"], position: { x: 50, y: 30 } });
  expect(onDrop).toHaveBeenCalledWith(["/a.pdf"]);
  expect(screen.getByText("idle")).toBeInTheDocument();
});

it("ignores drags without files and stops listening when disabled or unmounted", async () => {
  const { view } = renderTarget();
  await emit({ type: "enter", paths: [], position: { x: 50, y: 30 } });
  expect(screen.getByText("idle")).toBeInTheDocument();

  view.rerender(<Target onDrop={vi.fn()} enabled={false} />);
  await act(() => Promise.resolve());
  expect(mocks.handlers.size).toBe(0);
  view.rerender(<Target onDrop={vi.fn()} />);
  await act(() => Promise.resolve());
  expect(mocks.handlers.size).toBe(1);
  view.unmount();
  expect(mocks.handlers.size).toBe(0);
});
