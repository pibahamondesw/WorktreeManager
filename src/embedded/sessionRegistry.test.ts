import { describe, expect, it, vi } from "vitest";
import { TerminalStatus } from "../services/terminal";
import {
  createSessionRegistry,
  SessionKind,
  SessionState,
  SessionTransport,
  SurfaceOf,
} from "./sessionRegistry";

type FakeSession<Kind extends SessionKind> = SessionState<SurfaceOf<Kind>>;

function deferred<Value>() {
  let resolve!: (value: Value) => void;
  const promise = new Promise<Value>((settle) => (resolve = settle));
  return { promise, resolve };
}

function fakeTransport<Kind extends SessionKind>(listed: FakeSession<Kind>[] = []) {
  const listeners = new Set<(session: FakeSession<Kind>) => void>();
  const unlisten = vi.fn();
  const transport = {
    list: vi.fn(() => Promise.resolve(listed)),
    onSession: vi.fn((listener: (session: FakeSession<Kind>) => void) => {
      listeners.add(listener);
      return Promise.resolve(() => {
        unlisten();
        listeners.delete(listener);
      });
    }),
    describe: (session: FakeSession<Kind>) => session,
    stop: vi.fn(() => Promise.resolve()),
    close: vi.fn(() => Promise.resolve()),
  } satisfies SessionTransport<FakeSession<Kind>, SurfaceOf<Kind>>;
  return {
    transport,
    unlisten,
    emit: (session: FakeSession<Kind>) => listeners.forEach((listener) => listener(session)),
  };
}

function setup() {
  const fakes = {
    editor: fakeTransport<"editor">(),
    opencode: fakeTransport<"opencode">(),
    chat: fakeTransport<"chat">(),
    terminal: fakeTransport<"terminal">(),
  };
  const registry = createSessionRegistry({
    editor: fakes.editor.transport,
    opencode: fakes.opencode.transport,
    chat: fakes.chat.transport,
    terminal: fakes.terminal.transport,
  });
  return { fakes, registry };
}

const running: TerminalStatus = { kind: "running" };
const exited: TerminalStatus = { kind: "exited", code: 0 };
const terminal = (taskId: string, status: TerminalStatus | null): FakeSession<"terminal"> => ({
  taskId,
  surface: { kind: "terminal", agent: "codex" },
  status,
});
const chat = (taskId: string, status: TerminalStatus | null): FakeSession<"chat"> => ({
  taskId,
  surface: { kind: "chat", agent: "claude" },
  status,
});
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("following sessions", () => {
  it("subscribes before listing, and a list never overwrites an event received meanwhile", async () => {
    const { fakes, registry } = setup();
    const list = deferred<FakeSession<"terminal">[]>();
    fakes.terminal.transport.list.mockReturnValueOnce(list.promise);
    registry.subscribe(() => undefined);
    await flush();
    expect(fakes.terminal.transport.onSession.mock.invocationCallOrder[0]).toBeLessThan(
      fakes.terminal.transport.list.mock.invocationCallOrder[0]
    );
    fakes.terminal.emit(terminal("t1", exited));
    list.resolve([terminal("t1", running), terminal("t2", running)]);
    await flush();
    expect(registry.taskStatuses()).toEqual({ t1: exited, t2: running });
  });

  it("drops a list that lands after a newer one", async () => {
    const { fakes, registry } = setup();
    registry.subscribe(() => undefined);
    await flush();
    const older = deferred<FakeSession<"chat">[]>();
    fakes.chat.transport.list.mockReturnValueOnce(older.promise);
    registry.refresh();
    fakes.chat.transport.list.mockResolvedValueOnce([chat("t1", exited)]);
    registry.refresh();
    await flush();
    older.resolve([chat("t1", running)]);
    await flush();
    expect(registry.taskStatuses()).toEqual({ t1: exited });
  });

  it("ignores events and lists that land after the last watcher leaves", async () => {
    const { fakes, registry } = setup();
    const onChange = vi.fn();
    const unsubscribe = registry.subscribe(onChange);
    unsubscribe();
    fakes.chat.emit(chat("t1", running));
    await flush();
    expect(fakes.chat.unlisten).toHaveBeenCalled();
    expect(fakes.chat.transport.list).not.toHaveBeenCalled();
    expect(onChange).not.toHaveBeenCalled();
    expect(registry.taskStatuses()).toEqual({});
  });

  it("drops a list that lands after the last watcher leaves", async () => {
    const { fakes, registry } = setup();
    const list = deferred<FakeSession<"chat">[]>();
    fakes.chat.transport.list.mockReturnValueOnce(list.promise);
    const unsubscribe = registry.subscribe(() => undefined);
    await flush();
    unsubscribe();
    list.resolve([chat("t1", running)]);
    await flush();
    expect(registry.taskStatuses()).toEqual({});
  });

  it("keeps listing when a transport cannot be subscribed to", async () => {
    const { fakes, registry } = setup();
    fakes.terminal.transport.onSession.mockRejectedValueOnce(new Error("events unavailable"));
    fakes.terminal.transport.list.mockResolvedValueOnce([terminal("t1", running)]);
    registry.subscribe(() => undefined);
    await flush();
    expect(registry.taskStatuses()).toEqual({ t1: running });
  });
});

describe("task statuses", () => {
  it.each([
    { kind: "running", hasError: true },
    { kind: "exited", code: 1 },
  ] as const)(
    "keeps errors above live sessions and clears them on recovery: %o",
    async (failed) => {
      const { fakes, registry } = setup();
      const unsubscribe = registry.subscribe(() => undefined);
      await flush();
      fakes.chat.emit(chat("t1", failed));
      fakes.terminal.emit(terminal("t1", running));
      expect(registry.taskStatuses()).toEqual({ t1: failed });
      fakes.chat.emit(chat("t1", running));
      expect(registry.taskStatuses()).toEqual({ t1: running });
      fakes.chat.emit(chat("t1", failed));
      expect(registry.taskStatuses()).toEqual({ t1: failed });
      fakes.chat.emit(chat("t1", { kind: "exited", code: null }));
      expect(registry.taskStatuses()).toEqual({ t1: running });
      unsubscribe();
    }
  );

  it.each([true, false])(
    "counts a task as running while any session runs (reverse: %s)",
    async (reverse) => {
      const { fakes, registry } = setup();
      const sessions = [terminal("t1", exited), terminal("t1", null)];
      fakes.terminal.transport.list.mockResolvedValueOnce(
        reverse ? [...sessions].reverse() : sessions
      );
      fakes.chat.transport.list.mockResolvedValueOnce([chat("t1", running)]);
      registry.subscribe(() => undefined);
      await flush();
      expect(registry.taskStatuses()).toEqual({ t1: running });
      fakes.chat.emit(chat("t1", exited));
      expect(registry.taskStatuses()).toEqual({ t1: exited });
    }
  );

  it("stays the same object until a session changes", async () => {
    const { fakes, registry } = setup();
    registry.subscribe(() => undefined);
    await flush();
    const before = registry.taskStatuses();
    expect(registry.taskStatuses()).toBe(before);
    fakes.editor.emit({ taskId: "t1", surface: { kind: "editor" }, status: running });
    expect(registry.taskStatuses()).not.toBe(before);
  });

  it("keeps another task's sessions when one task's sessions change", async () => {
    const { fakes, registry } = setup();
    fakes.chat.transport.list.mockResolvedValueOnce([chat("t2", running)]);
    registry.subscribe(() => undefined);
    await flush();
    fakes.chat.emit(chat("t1", exited));
    expect(registry.taskStatuses()).toEqual({ t1: exited, t2: running });
  });
});

describe("session events", () => {
  it("reports each event with the last one reported, not with a later list", async () => {
    const { fakes, registry } = setup();
    fakes.chat.transport.list.mockResolvedValueOnce([chat("t1", null)]);
    const listener = vi.fn();
    registry.onSessionEvent("chat", listener);
    await flush();
    fakes.chat.emit(chat("t1", running));
    fakes.chat.transport.list.mockResolvedValueOnce([chat("t1", exited)]);
    registry.refresh();
    await flush();
    fakes.chat.emit(chat("t1", exited));
    expect(listener.mock.calls).toEqual([
      [chat("t1", running), chat("t1", null)],
      [chat("t1", exited), chat("t1", running)],
    ]);
  });
});

describe("stopping and closing", () => {
  it("stops a surface through its own transport only", async () => {
    const { fakes, registry } = setup();
    await registry.stop("t1", { kind: "chat", agent: "codex" });
    await registry.stop("t1", { kind: "external" });
    expect(fakes.chat.transport.stop).toHaveBeenCalledWith("t1", { kind: "chat", agent: "codex" });
    for (const kind of ["editor", "opencode", "terminal"] as const)
      expect(fakes[kind].transport.stop).not.toHaveBeenCalled();
  });

  it("closes the task in every transport and fails when any of them fails", async () => {
    const { fakes, registry } = setup();
    fakes.editor.transport.close.mockRejectedValueOnce(new Error("still closing"));
    await expect(registry.closeAll("t1")).rejects.toThrow("still closing");
    for (const kind of ["editor", "opencode", "chat", "terminal"] as const)
      expect(fakes[kind].transport.close.mock.calls).toEqual([["t1"]]);
    await expect(registry.closeAll("t1")).resolves.toBeUndefined();
  });
});
