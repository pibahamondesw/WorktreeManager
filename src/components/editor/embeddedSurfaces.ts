import type { ComponentType } from "react";
import {
  editorClose,
  editorInstallStatus,
  editorOpen,
  editorProbe,
  EditorSession,
  EmbeddedSurfaceKind,
  onEditorSession,
} from "../../services/codeEditor";
import { onOpenCodeSession, openCodeClose, openCodeOpen } from "../../services/opencode";
import { EditorInstallation } from "./EditorInstallation";

interface EmbeddedTask {
  taskId: string;
  workspaceId: string;
  folders: string[];
}

/** A dependency the surface needs before it can open, with the UI that sets it up. */
interface InstallGate {
  prompt: string;
  needed: () => Promise<boolean>;
  Setup: ComponentType<{ onReady: () => void }>;
}

export interface EmbeddedSurface {
  kind: EmbeddedSurfaceKind;
  name: string;
  testId: string;
  restartLabel: string;
  open: (task: EmbeddedTask) => Promise<EditorSession>;
  close: (taskId: string) => Promise<void>;
  onSession: (callback: (session: EditorSession) => void) => Promise<() => void>;
  install?: InstallGate;
}

const vsCodeNeedsInstall = async () => {
  if ((await editorInstallStatus()).active) return true;
  return !(await editorProbe()).ready;
};

export const embeddedSurfaces = {
  editor: {
    kind: "editor",
    name: "VSCode",
    testId: "editor-pane",
    restartLabel: "Restart editor",
    open: ({ taskId, workspaceId, folders }) => editorOpen(taskId, workspaceId, folders),
    close: editorClose,
    onSession: onEditorSession,
    install: {
      prompt: "Set up VSCode.",
      needed: vsCodeNeedsInstall,
      Setup: EditorInstallation,
    },
  },
  opencode: {
    kind: "opencode",
    name: "OpenCode",
    testId: "opencode-pane",
    restartLabel: "Restart OpenCode",
    open: ({ taskId, folders }) => openCodeOpen(taskId, folders),
    close: openCodeClose,
    onSession: onOpenCodeSession,
  },
} satisfies { [Kind in EmbeddedSurfaceKind]: EmbeddedSurface & { kind: Kind } };
