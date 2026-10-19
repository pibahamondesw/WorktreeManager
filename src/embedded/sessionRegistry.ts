import { TaskSurface } from "../types";
import { TerminalStatus } from "../services/terminal";

export type SessionKind = Exclude<TaskSurface["kind"], "external">;

const SESSION_KINDS: SessionKind[] = ["editor", "opencode", "chat", "terminal"];

export type SurfaceOf<Kind extends SessionKind> = Extract<TaskSurface, { kind: Kind }>;

/** A transport session in the registry's terms; a null status means there is no session. */
export interface SessionState<Surface extends TaskSurface = TaskSurface> {
  taskId: string;
  surface: Surface;
  status: TerminalStatus | null;
}

/** One way of running a task surface: how its sessions are listed, followed, stopped and closed. */
export interface SessionTransport<Session, Surface extends TaskSurface = TaskSurface> {
  list(): Promise<Session[]>;
  onSession(listener: (session: Session) => void): Promise<() => void>;
  describe(session: Session): SessionState<Surface>;
  stop(taskId: string, surface: Surface): Promise<void>;
  close(taskId: string): Promise<void>;
}

export type SessionTransports<Sessions extends Record<SessionKind, unknown>> = {
  [Kind in SessionKind]: SessionTransport<Sessions[Kind], SurfaceOf<Kind>>;
};

type SessionEventListener<Session> = (next: Session, previous: Session | undefined) => void;

interface Entry {
  state: SessionState;
  stamp: number;
}

const sessionKey = ({ taskId, surface }: SessionState) =>
  "agent" in surface ? `${taskId}:${surface.agent}` : taskId;

/** A task is running while any of its sessions runs; otherwise it shows its first ended one. */
function statusesByTask(states: SessionState[]): Record<string, TerminalStatus> {
  const statuses: Record<string, TerminalStatus> = {};
  for (const { taskId, status } of states) {
    if (status && (status.kind === "running" || !statuses[taskId])) statuses[taskId] = status;
  }
  return statuses;
}

/**
 * The task sessions of every transport. While anything watches it, each transport is subscribed
 * before it is listed, so no event is lost; a list never overwrites an event received after it
 * was requested, and nothing from an older watch or list lands after a newer one.
 */
export function createSessionRegistry<Sessions extends Record<SessionKind, unknown>>(
  transports: SessionTransports<Sessions>
) {
  const entries: Record<SessionKind, Map<string, Entry>> = {
    editor: new Map(),
    opencode: new Map(),
    chat: new Map(),
    terminal: new Map(),
  };
  const reported: { [Kind in SessionKind]: Map<string, Sessions[Kind]> } = {
    editor: new Map(),
    opencode: new Map(),
    chat: new Map(),
    terminal: new Map(),
  };
  const eventListeners: { [Kind in SessionKind]: Set<SessionEventListener<Sessions[Kind]>> } = {
    editor: new Set(),
    opencode: new Set(),
    chat: new Set(),
    terminal: new Set(),
  };
  const listRequests: Record<SessionKind, number> = {
    editor: 0,
    opencode: 0,
    chat: 0,
    terminal: 0,
  };
  const changeListeners = new Set<() => void>();
  let unlisteners: (() => void)[] = [];
  let watchers = 0;
  let generation = 0;
  let clock = 0;
  let statuses: Record<string, TerminalStatus> = {};

  const changed = () => {
    statuses = statusesByTask(
      SESSION_KINDS.flatMap((kind) => [...entries[kind].values()].map((entry) => entry.state))
    );
    changeListeners.forEach((listener) => listener());
  };

  function receive<Kind extends SessionKind>(kind: Kind, session: Sessions[Kind]) {
    const state = transports[kind].describe(session);
    const key = sessionKey(state);
    const previous = reported[kind].get(key);
    reported[kind].set(key, session);
    entries[kind].set(key, { state, stamp: ++clock });
    changed();
    eventListeners[kind].forEach((listener) => listener(session, previous));
  }

  function load<Kind extends SessionKind>(kind: Kind) {
    const request = ++listRequests[kind];
    const watch = generation;
    const requestedAt = clock;
    transports[kind].list().then(
      (sessions) => {
        if (watch !== generation || request !== listRequests[kind]) return;
        for (const [key, entry] of entries[kind])
          if (entry.stamp <= requestedAt) entries[kind].delete(key);
        for (const session of sessions) {
          const state = transports[kind].describe(session);
          const key = sessionKey(state);
          if (!entries[kind].has(key)) entries[kind].set(key, { state, stamp: requestedAt });
          if (!reported[kind].has(key)) reported[kind].set(key, session);
        }
        changed();
      },
      () => undefined
    );
  }

  function follow<Kind extends SessionKind>(kind: Kind) {
    const watch = generation;
    void transports[kind]
      .onSession((session) => {
        if (watch === generation) receive(kind, session);
      })
      .then(
        (unlisten) => {
          if (watch === generation) unlisteners.push(unlisten);
          else unlisten();
        },
        () => undefined
      )
      .then(() => {
        if (watch === generation) load(kind);
      });
  }

  function stopFollowing() {
    generation++;
    unlisteners.forEach((unlisten) => unlisten());
    unlisteners = [];
    SESSION_KINDS.forEach((kind) => {
      entries[kind].clear();
      reported[kind].clear();
    });
    statuses = {};
  }

  function watch() {
    if (watchers++ === 0) SESSION_KINDS.forEach(follow);
    let watching = true;
    return () => {
      if (!watching) return;
      watching = false;
      if (--watchers === 0) stopFollowing();
    };
  }

  const transportOf = (kind: SessionKind): SessionTransport<unknown> => transports[kind];

  return {
    /** Follows every transport while subscribed; `onChange` fires when any session changes. */
    subscribe(onChange: () => void) {
      changeListeners.add(onChange);
      const unwatch = watch();
      return () => {
        changeListeners.delete(onChange);
        unwatch();
      };
    },

    /** Session status per task id, as last reported; the same object until a session changes. */
    taskStatuses: () => statuses,

    /** Lists every transport again, for sessions that start without announcing themselves. */
    refresh() {
      if (watchers > 0) SESSION_KINDS.forEach(load);
    },

    /**
     * Reports each status event of one transport with the last one reported for that session, or
     * the first listed one; later lists never stand in for an event still to be reported.
     */
    onSessionEvent<Kind extends SessionKind>(
      kind: Kind,
      listener: SessionEventListener<Sessions[Kind]>
    ) {
      eventListeners[kind].add(listener);
      const unwatch = watch();
      return () => {
        eventListeners[kind].delete(listener);
        unwatch();
      };
    },

    /**
     * The task's surfaces with a live process, asked fresh from every transport. A list that
     * fails to load counts as empty so the others still report what is live.
     */
    async listLive(taskId: string): Promise<TaskSurface[]> {
      const lists = await Promise.all(
        SESSION_KINDS.map((kind) =>
          transportOf(kind)
            .list()
            .then((sessions) => sessions.map((session) => transportOf(kind).describe(session)))
            .catch((): SessionState[] => [])
        )
      );
      return lists
        .flat()
        .filter((state) => state.taskId === taskId && state.status?.kind === "running")
        .map((state) => state.surface);
    },

    /** Ends the surface's process through its normal close path; conversations stay resumable. */
    stop(taskId: string, surface: TaskSurface): Promise<void> {
      if (surface.kind === "external") return Promise.resolve();
      return transportOf(surface.kind).stop(taskId, surface);
    },

    /** Tears down every session of the task in every transport; fails if any transport fails. */
    async closeAll(taskId: string): Promise<void> {
      await Promise.all(SESSION_KINDS.map((kind) => transportOf(kind).close(taskId)));
    },
  };
}

export type SessionRegistry = ReturnType<typeof createSessionRegistry>;
