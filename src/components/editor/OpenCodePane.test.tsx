// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import "@testing-library/jest-dom/vitest";
import type { onOpenCodeSession, OpenCodeSession } from "../../services/opencode";

const mocks = vi.hoisted(() => ({
  open: vi.fn(),
  close: vi.fn(),
  attach: vi.fn(),
  release: vi.fn(),
  update: vi.fn(),
  onSession: vi.fn<typeof onOpenCodeSession>(),
}));
vi.mock("../../services/opencode", () => ({
  openCodeOpen: mocks.open,
  openCodeClose: mocks.close,
  onOpenCodeSession: mocks.onSession,
}));
vi.mock("../../services/codeEditor", () => ({
  editorPresentation: { attach: mocks.attach },
}));
import { OpenCodePane } from "./OpenCodePane";

const session: OpenCodeSession = {
  taskId: "a",
  generation: "one",
  status: "running",
  pid: 10,
  error: null,
};
let notify: (session: OpenCodeSession) => void;
beforeEach(() => {
  vi.clearAllMocks();
  globalThis.ResizeObserver = class {
    observe() {}
    disconnect() {}
  } as never;
  mocks.attach.mockReturnValue({ release: mocks.release, update: mocks.update });
  mocks.open.mockResolvedValue(session);
  mocks.close.mockResolvedValue(undefined);
  mocks.onSession.mockImplementation(async (callback) => {
    notify = callback;
    return () => undefined;
  });
});
afterEach(cleanup);

describe("OpenCodePane lifecycle", () => {
  it("opens every task folder and detaches without stopping the server", async () => {
    const status = vi.fn();
    const view = render(<OpenCodePane taskId="a" folders={["/a", "/b"]} onStatusChange={status} />);
    await waitFor(() => expect(status).toHaveBeenCalledWith({ kind: "running" }));
    expect(mocks.open).toHaveBeenCalledWith("a", ["/a", "/b"]);
    expect(mocks.attach).toHaveBeenCalledWith("a", expect.anything(), expect.any(Function));
    view.unmount();
    expect(mocks.release).toHaveBeenCalledOnce();
    expect(mocks.close).not.toHaveBeenCalled();
  });

  it("shows a closing message while OpenCode is being closed", async () => {
    const props = { taskId: "a", folders: ["/a"], onStatusChange: vi.fn() };
    const view = render(<OpenCodePane {...props} />);
    expect(screen.getByText("Opening OpenCode…")).toBeInTheDocument();
    view.rerender(<OpenCodePane {...props} closing />);
    expect(screen.getByText("Closing OpenCode…")).toBeInTheDocument();
    await waitFor(() => expect(mocks.open).toHaveBeenCalledOnce());
  });

  it("ignores other sessions and restarts after the current server ends", async () => {
    render(<OpenCodePane taskId="a" folders={["/a"]} onStatusChange={vi.fn()} />);
    await waitFor(() => expect(mocks.open).toHaveBeenCalledOnce());
    act(() => notify({ ...session, generation: "older", status: "exited", error: "old" }));
    act(() => notify({ ...session, taskId: "b", status: "exited", error: "other task" }));
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    act(() => notify({ ...session, status: "exited", error: "OpenCode ended" }));
    expect(screen.getByRole("alert")).toHaveTextContent("OpenCode ended");
    fireEvent.click(screen.getByRole("button", { name: "Restart OpenCode" }));
    await waitFor(() => expect(mocks.open).toHaveBeenCalledTimes(2));
    expect(mocks.close).toHaveBeenCalledWith("a");
  });

  it("shows a startup failure with a restart", async () => {
    mocks.open.mockRejectedValueOnce("opencode CLI not found on PATH (see Doctor)");
    render(<OpenCodePane taskId="a" folders={["/a"]} onStatusChange={vi.fn()} />);
    expect(await screen.findByRole("alert")).toHaveTextContent("opencode CLI not found");
    expect(screen.getByRole("button", { name: "Restart OpenCode" })).toBeInTheDocument();
  });
});
