// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import "@testing-library/jest-dom/vitest";
import type { EditorSession } from "../../services/codeEditor";

const mocks = vi.hoisted(() => ({
  open: vi.fn(),
  close: vi.fn(),
  probe: vi.fn(),
  install: vi.fn(),
  installStatus: vi.fn(),
  attach: vi.fn(),
  release: vi.fn(),
  update: vi.fn(),
  onSession: vi.fn(),
}));
vi.mock("../../services/codeEditor", () => ({
  editorOpen: mocks.open,
  editorClose: mocks.close,
  editorProbe: mocks.probe,
  editorInstall: mocks.install,
  editorInstallStatus: mocks.installStatus,
  onEditorSession: mocks.onSession,
  editorPresentation: { attach: mocks.attach },
}));
import { EditorPane } from "./EditorPane";

const session: EditorSession = {
  taskId: "a",
  generation: "one",
  status: "running",
  pid: 10,
  error: null,
};
let notify: (session: EditorSession) => void;
beforeEach(() => {
  vi.clearAllMocks();
  globalThis.ResizeObserver = class {
    observe() {}
    disconnect() {}
  } as never;
  mocks.attach.mockReturnValue({ release: mocks.release, update: mocks.update });
  mocks.open.mockResolvedValue(session);
  mocks.close.mockResolvedValue(undefined);
  mocks.probe.mockResolvedValue({ ready: true });
  mocks.install.mockResolvedValue({ ready: true });
  mocks.installStatus.mockResolvedValue({ phase: "", active: false, logs: [] });
  mocks.onSession.mockImplementation(async (callback) => {
    notify = callback;
    return () => undefined;
  });
});
afterEach(cleanup);

describe("EditorPane lifecycle", () => {
  it("shows ongoing setup instead of opening a runtime still being validated", async () => {
    mocks.installStatus.mockResolvedValue({
      active: true,
      phase: "validating",
      message: "Checking that the editor starts correctly…",
      logs: [],
    });
    render(<EditorPane taskId="a" folders={["/a"]} onStatusChange={vi.fn()} />);
    await waitFor(() =>
      expect(screen.getByRole("status")).toHaveTextContent(
        "Checking that the editor starts correctly"
      )
    );
    expect(mocks.open).not.toHaveBeenCalled();
    expect(mocks.install).not.toHaveBeenCalled();
  });
  it("opens all task folders and detaches without stopping the session", async () => {
    const status = vi.fn();
    const view = render(<EditorPane taskId="a" folders={["/a", "/b"]} onStatusChange={status} />);
    await waitFor(() => expect(status).toHaveBeenCalledWith({ kind: "running" }));
    expect(mocks.open).toHaveBeenCalledWith("a", ["/a", "/b"]);
    view.unmount();
    expect(mocks.release).toHaveBeenCalledOnce();
    expect(mocks.close).not.toHaveBeenCalled();
  });

  it("ignores another session's events and offers restart for the current process exit", async () => {
    render(<EditorPane taskId="a" folders={["/a"]} onStatusChange={vi.fn()} />);
    await waitFor(() => expect(mocks.open).toHaveBeenCalledOnce());
    act(() => notify({ ...session, generation: "older", status: "exited", error: "old error" }));
    act(() => notify({ ...session, taskId: "b", status: "exited", error: "other task" }));
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    act(() => notify({ ...session, status: "exited", error: "process ended" }));
    fireEvent.click(screen.getByRole("button", { name: "Restart editor" }));
    await waitFor(() => expect(mocks.open).toHaveBeenCalledTimes(2));
    expect(mocks.close).toHaveBeenCalledWith("a");
  });

  it("does not open a task after its component unmounts during the dependency check", async () => {
    let finish!: (value: { ready: boolean }) => void;
    mocks.probe.mockReturnValue(
      new Promise((resolve) => {
        finish = resolve;
      })
    );
    const view = render(<EditorPane taskId="a" folders={["/a"]} onStatusChange={vi.fn()} />);
    await waitFor(() => expect(mocks.probe).toHaveBeenCalled());
    view.unmount();
    await act(async () => finish({ ready: true }));
    expect(mocks.open).not.toHaveBeenCalled();
  });

  it("installs the dependency before creating an editor session", async () => {
    mocks.probe.mockResolvedValueOnce({ ready: false });
    render(<EditorPane taskId="a" folders={["/a"]} onStatusChange={vi.fn()} />);
    fireEvent.click(await screen.findByRole("button", { name: "Install editor" }));
    await waitFor(() => expect(mocks.open).toHaveBeenCalledOnce());
    expect(mocks.install).toHaveBeenCalledOnce();
  });
});
