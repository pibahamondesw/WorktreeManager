import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const GIT_STATUS_CHANGED_EVENT = "git-status-changed";

export function watchWorktreeGitStatus(worktreePaths: string[]) {
  void invoke("git_status_watch", { worktreePaths }).catch(() => undefined);
}

export function onWorktreeGitStatusChanged(handler: (worktreePaths: Set<string>) => void) {
  const unlisten = listen<string[]>(GIT_STATUS_CHANGED_EVENT, ({ payload }) =>
    handler(new Set(payload))
  ).catch(() => undefined);
  return () => {
    void unlisten.then((stop) => stop?.());
  };
}
