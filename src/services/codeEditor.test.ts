import { describe, expect, it, vi } from "vitest";
import { EditorPresentationController } from "./codeEditor";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

const bounds = { x: 40, y: 80, width: 800, height: 600 };

describe("editor presentation", () => {
  it("ignores late cleanup and resizing from a previous task", () => {
    const dispatch = vi.fn().mockResolvedValue(undefined);
    const controller = new EditorPresentationController(dispatch);
    const first = controller.attach("a", bounds, vi.fn());
    controller.attach("b", bounds, vi.fn());
    first.release();
    first.update({ ...bounds, width: 1 });
    expect(dispatch).toHaveBeenCalledTimes(2);
    expect(dispatch.mock.calls.at(-1)?.[0]).toMatchObject({
      taskId: "b",
      bounds,
      suppressed: false,
    });
    expect(dispatch.mock.calls[1][0].revision).toBeGreaterThan(dispatch.mock.calls[0][0].revision);
  });

  it("keeps the editor hidden until every modal has closed, including task switches", () => {
    const dispatch = vi.fn().mockResolvedValue(undefined);
    const controller = new EditorPresentationController(dispatch);
    controller.attach("a", bounds, vi.fn());
    const closeFirst = controller.suppress();
    const closeSecond = controller.suppress();
    controller.attach("b", bounds, vi.fn());
    closeFirst();
    expect(dispatch.mock.calls.at(-1)?.[0]).toMatchObject({
      taskId: "b",
      suppressed: true,
      focus: false,
    });
    closeSecond();
    expect(dispatch.mock.calls.at(-1)?.[0]).toMatchObject({
      taskId: "b",
      suppressed: false,
      focus: true,
    });
  });

  it("returning to the list only clears presentation", () => {
    const dispatch = vi.fn().mockResolvedValue(undefined);
    const controller = new EditorPresentationController(dispatch);
    controller.attach("a", bounds, vi.fn()).release();
    expect(dispatch.mock.calls.at(-1)?.[0]).toMatchObject({ taskId: null });
    controller.attach("a", bounds, vi.fn());
    expect(dispatch.mock.calls.at(-1)?.[0]).toMatchObject({ taskId: "a" });
    controller.reset();
    expect(dispatch.mock.calls.at(-1)?.[0]).toMatchObject({ taskId: null });
  });
});
