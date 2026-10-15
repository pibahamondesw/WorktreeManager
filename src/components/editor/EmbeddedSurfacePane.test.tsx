// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import "@testing-library/jest-dom/vitest";
import type { EditorSession } from "../../services/codeEditor";
import type { EmbeddedSurface } from "./embeddedSurfaces";

const mocks = vi.hoisted(() => ({
  editorOpen: vi.fn(),
  editorProbe: vi.fn(),
  editorInstall: vi.fn(),
  editorInstallStatus: vi.fn(),
  openCodeOpen: vi.fn(),
  attach: vi.fn(),
  release: vi.fn(),
  update: vi.fn(),
}));
vi.mock("../../services/codeEditor", () => ({
  editorOpen: mocks.editorOpen,
  editorClose: vi.fn(),
  editorProbe: mocks.editorProbe,
  editorInstall: mocks.editorInstall,
  editorInstallStatus: mocks.editorInstallStatus,
  onEditorSession: vi.fn(),
  editorPresentation: { attach: mocks.attach },
}));
vi.mock("../../services/opencode", () => ({
  openCodeOpen: mocks.openCodeOpen,
  openCodeClose: vi.fn(),
  onOpenCodeSession: vi.fn(),
}));
import { EmbeddedSurfacePane } from "./EmbeddedSurfacePane";
import { embeddedSurfaces } from "./embeddedSurfaces";

const session: EditorSession = {
  taskId: "a",
  generation: "one",
  status: "running",
  pid: 10,
  error: null,
};
let notify: (session: EditorSession) => void;
let surface: EmbeddedSurface & {
  open: ReturnType<typeof vi.fn>;
  close: ReturnType<typeof vi.fn>;
};

const renderPane = (props: Partial<Parameters<typeof EmbeddedSurfacePane>[0]> = {}) =>
  render(
    <EmbeddedSurfacePane
      surface={surface}
      workspaceId="workspace"
      taskId="a"
      folders={["/a"]}
      onStatusChange={vi.fn()}
      {...props}
    />
  );

beforeEach(() => {
  vi.clearAllMocks();
  globalThis.ResizeObserver = class {
    observe() {}
    disconnect() {}
  } as never;
  mocks.attach.mockReturnValue({ release: mocks.release, update: mocks.update });
  mocks.editorProbe.mockResolvedValue({ ready: true });
  mocks.editorInstall.mockResolvedValue({ ready: true });
  mocks.editorInstallStatus.mockResolvedValue({ phase: "", active: false, logs: [] });
  surface = {
    name: "Stub",
    testId: "stub-pane",
    restartLabel: "Restart stub",
    open: vi.fn().mockResolvedValue(session),
    close: vi.fn().mockResolvedValue(undefined),
    onSession: vi.fn(async (callback: (session: EditorSession) => void) => {
      notify = callback;
      return () => undefined;
    }),
  };
});
afterEach(cleanup);

describe("EmbeddedSurfacePane lifecycle", () => {
  it("opens all task folders and detaches without stopping the session", async () => {
    const status = vi.fn();
    const view = renderPane({ folders: ["/a", "/b"], onStatusChange: status });
    await waitFor(() => expect(status).toHaveBeenCalledWith({ kind: "running" }));
    expect(surface.open).toHaveBeenCalledWith({
      taskId: "a",
      workspaceId: "workspace",
      folders: ["/a", "/b"],
    });
    expect(mocks.attach).toHaveBeenCalledWith("a", expect.anything(), expect.any(Function));
    view.unmount();
    expect(mocks.release).toHaveBeenCalledOnce();
    expect(surface.close).not.toHaveBeenCalled();
  });

  it("shows a closing message while the surface is being closed", async () => {
    const view = renderPane();
    expect(screen.getByText("Opening Stub…")).toBeInTheDocument();
    view.rerender(
      <EmbeddedSurfacePane
        surface={surface}
        workspaceId="workspace"
        taskId="a"
        folders={["/a"]}
        onStatusChange={vi.fn()}
        closing
      />
    );
    expect(screen.getByText("Closing Stub…")).toBeInTheDocument();
    await waitFor(() => expect(surface.open).toHaveBeenCalledOnce());
  });

  it("ignores other sessions' events and restarts after the current one ends", async () => {
    renderPane();
    await waitFor(() => expect(surface.open).toHaveBeenCalledOnce());
    act(() => notify({ ...session, generation: "older", status: "exited", error: "old" }));
    act(() => notify({ ...session, taskId: "b", status: "exited", error: "other task" }));
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    act(() => notify({ ...session, status: "exited", error: "process ended" }));
    expect(screen.getByRole("alert")).toHaveTextContent("process ended");
    fireEvent.click(screen.getByRole("button", { name: "Restart stub" }));
    await waitFor(() => expect(surface.open).toHaveBeenCalledTimes(2));
    expect(surface.close).toHaveBeenCalledWith("a");
  });

  it("shows a startup failure with a restart", async () => {
    surface.open.mockRejectedValueOnce("CLI not found on PATH");
    renderPane();
    expect(await screen.findByRole("alert")).toHaveTextContent("CLI not found");
    expect(screen.getByRole("button", { name: "Restart stub" })).toBeInTheDocument();
  });

  it("sets up a missing dependency before opening", async () => {
    let installed = false;
    surface.install = {
      prompt: "Set up the stub.",
      needed: () => Promise.resolve(!installed),
      Setup: ({ onReady }) => (
        <button
          onClick={() => {
            installed = true;
            onReady();
          }}
        >
          Install stub
        </button>
      ),
    };
    renderPane();
    expect(await screen.findByText("Set up the stub.")).toBeInTheDocument();
    expect(surface.open).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Install stub" }));
    await waitFor(() => expect(surface.open).toHaveBeenCalledOnce());
  });

  it("does not open a task after unmounting during the dependency check", async () => {
    let finish!: (needed: boolean) => void;
    surface.install = {
      prompt: "Set up the stub.",
      needed: () => new Promise((resolve) => (finish = resolve)),
      Setup: () => null,
    };
    const view = renderPane();
    await waitFor(() => expect(finish).toBeDefined());
    view.unmount();
    await act(async () => finish(false));
    expect(surface.open).not.toHaveBeenCalled();
  });
});

describe("embedded surfaces", () => {
  const task = { taskId: "a", workspaceId: "workspace", folders: ["/a", "/b"] };

  it("opens VS Code on the task's workspace and OpenCode on its folders", async () => {
    await embeddedSurfaces.editor.open(task);
    await embeddedSurfaces.opencode.open(task);
    expect(mocks.editorOpen).toHaveBeenCalledWith("a", "workspace", ["/a", "/b"]);
    expect(mocks.openCodeOpen).toHaveBeenCalledWith("a", ["/a", "/b"]);
    expect("install" in embeddedSurfaces.opencode).toBe(false);
  });

  it("gates VS Code on a finished, ready installation", async () => {
    const needed = embeddedSurfaces.editor.install.needed;
    expect(await needed()).toBe(false);
    mocks.editorProbe.mockResolvedValueOnce({ ready: false });
    expect(await needed()).toBe(true);
    mocks.editorInstallStatus.mockResolvedValueOnce({ active: true, logs: [] });
    mocks.editorProbe.mockClear();
    expect(await needed()).toBe(true);
    expect(mocks.editorProbe).not.toHaveBeenCalled();
  });

  it("installs VS Code embedded before creating an editor session", async () => {
    mocks.editorProbe.mockResolvedValueOnce({ ready: false });
    mocks.editorOpen.mockResolvedValue(session);
    render(
      <EmbeddedSurfacePane
        surface={embeddedSurfaces.editor}
        workspaceId="workspace"
        taskId="a"
        folders={["/a"]}
        onStatusChange={vi.fn()}
      />
    );
    fireEvent.click(await screen.findByRole("button", { name: "Install editor" }));
    await waitFor(() => expect(mocks.editorOpen).toHaveBeenCalledOnce());
    expect(mocks.editorInstall).toHaveBeenCalledOnce();
  });
});
