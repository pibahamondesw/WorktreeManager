import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface EditorRuntime {
  installed: boolean;
  ready: boolean;
  version: string;
  detail: string;
}

export interface EditorSession {
  taskId: string;
  generation: string;
  status: "starting" | "running" | "exited" | "closed";
  pid: number | null;
  error: string | null;
}

export const editorProbe = () => invoke<EditorRuntime>("vscode_probe");
export const editorInstall = () => invoke<EditorRuntime>("vscode_install");
export interface EditorInstallProgress {
  active: boolean;
  phase: string;
  message: string;
  downloaded: number;
  total: number | null;
  startedAt: number;
  logs: string[];
}
export const editorInstallStatus = () => invoke<EditorInstallProgress>("vscode_install_status");
export const editorOpen = (taskId: string, folders: string[]) =>
  invoke<EditorSession>("vscode_open", { taskId, folders });
export const editorClose = (taskId: string) => invoke<void>("vscode_close", { taskId });
export const editorList = () => invoke<EditorSession[]>("vscode_list");
export const onEditorSession = (callback: (session: EditorSession) => void) =>
  listen<EditorSession>("editor-session", (event) => callback(event.payload));

export interface EditorBounds {
  x: number;
  y: number;
  width: number;
  height: number;
}
export interface EditorPresentation {
  revision: number;
  taskId: string | null;
  bounds: EditorBounds;
  suppressed: boolean;
  focus: boolean;
}

export class EditorPresentationController {
  private revision = Date.now() * 1000;
  private blockers = new Set<symbol>();
  private active: {
    token: symbol;
    taskId: string;
    bounds: EditorBounds;
    onError: (error: string) => void;
  } | null = null;

  private dispatch: (state: EditorPresentation) => Promise<void>;

  constructor(dispatch: (state: EditorPresentation) => Promise<void>) {
    this.dispatch = dispatch;
  }

  reset() {
    this.active = null;
    this.send(false);
  }

  attach(taskId: string, bounds: EditorBounds, onError: (error: string) => void) {
    const token = Symbol(taskId);
    this.active = { token, taskId, bounds, onError };
    this.send(true);
    return {
      update: (bounds: EditorBounds) => {
        if (this.active?.token !== token) return;
        this.active.bounds = bounds;
        this.send(false);
      },
      release: () => {
        if (this.active?.token !== token) return;
        this.active = null;
        this.send(false);
      },
    };
  }

  suppress() {
    const token = Symbol("modal");
    this.blockers.add(token);
    if (this.active) this.send(false);
    return () => {
      this.blockers.delete(token);
      if (this.active) this.send(this.blockers.size === 0);
    };
  }

  private send(focus: boolean) {
    const active = this.active;
    void this.dispatch({
      revision: ++this.revision,
      taskId: active?.taskId ?? null,
      bounds: active?.bounds ?? { x: 0, y: 0, width: 0, height: 0 },
      suppressed: this.blockers.size > 0,
      focus,
    }).catch((error: unknown) => {
      if (active && this.active?.token === active.token) active.onError(String(error));
    });
  }
}

export const editorPresentation = new EditorPresentationController((state) =>
  invoke<void>("vscode_present", { ...state })
);
