// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import {
  EditorPresentationController,
  followAppThemeInEditor,
  type EditorPresentation,
} from "./codeEditor";
import { applyTheme } from "../themes";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

const bounds = { x: 40, y: 80, width: 800, height: 600 };

describe("editor presentation", () => {
  it("ignores late cleanup and resizing from a previous task", () => {
    const dispatch = vi
      .fn<(state: EditorPresentation) => Promise<void>>()
      .mockResolvedValue(undefined);
    const controller = new EditorPresentationController(dispatch);
    const first = controller.attach("a", "editor", bounds, vi.fn());
    controller.attach("b", "editor", bounds, vi.fn());
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
    const dispatch = vi
      .fn<(state: EditorPresentation) => Promise<void>>()
      .mockResolvedValue(undefined);
    const controller = new EditorPresentationController(dispatch);
    controller.attach("a", "editor", bounds, vi.fn());
    const closeFirst = controller.suppress();
    const closeSecond = controller.suppress();
    controller.attach("b", "editor", bounds, vi.fn());
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

  it("presents the surface the task pane selected", () => {
    const dispatch = vi
      .fn<(state: EditorPresentation) => Promise<void>>()
      .mockResolvedValue(undefined);
    const controller = new EditorPresentationController(dispatch);
    controller.attach("a", "opencode", bounds, vi.fn());
    expect(dispatch.mock.calls.at(-1)?.[0]).toMatchObject({ taskId: "a", surface: "opencode" });
    controller.attach("a", "editor", bounds, vi.fn());
    expect(dispatch.mock.calls.at(-1)?.[0]).toMatchObject({ taskId: "a", surface: "editor" });
    controller.reset();
    expect(dispatch.mock.calls.at(-1)?.[0]).toMatchObject({ taskId: null, surface: null });
  });

  it("returning to the list only clears presentation", () => {
    const dispatch = vi
      .fn<(state: EditorPresentation) => Promise<void>>()
      .mockResolvedValue(undefined);
    const controller = new EditorPresentationController(dispatch);
    controller.attach("a", "editor", bounds, vi.fn()).release();
    expect(dispatch.mock.calls.at(-1)?.[0]).toMatchObject({ taskId: null });
    controller.attach("a", "editor", bounds, vi.fn());
    expect(dispatch.mock.calls.at(-1)?.[0]).toMatchObject({ taskId: "a" });
    controller.reset();
    expect(dispatch.mock.calls.at(-1)?.[0]).toMatchObject({ taskId: null });
  });
});

describe("editor theme", () => {
  const themeSent = () =>
    vi.mocked(invoke).mock.calls.filter(([command]) => command === "vscode_set_theme");

  it("sends the current app theme and every later change until stopped", () => {
    vi.mocked(invoke).mockReset().mockResolvedValue(undefined);
    applyTheme("ocean");
    const stop = followAppThemeInEditor();
    applyTheme("solarized-light");
    stop();
    applyTheme("forest");
    expect(
      themeSent().map(
        ([, args]) =>
          (args as { defaults: Record<string, unknown> }).defaults["workbench.colorTheme"]
      )
    ).toEqual(["Dark Modern", "Light Modern"]);
  });

  it("keeps the app working when the editor cannot take the theme", async () => {
    vi.mocked(invoke).mockReset().mockRejectedValue(new Error("unavailable"));
    const stop = followAppThemeInEditor();
    await Promise.resolve();
    stop();
    expect(themeSent()).toHaveLength(1);
  });
});
