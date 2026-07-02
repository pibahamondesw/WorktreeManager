import { useEffect, useRef } from "react";
import { check } from "@tauri-apps/plugin-updater";
import { ask } from "@tauri-apps/plugin-dialog";
import { relaunch } from "@tauri-apps/plugin-process";

/**
 * Checks GitHub Releases for a newer version on startup. If one is available,
 * prompts the user and — on confirmation — downloads, installs, and relaunches.
 *
 * Fails silently on any error (offline, or running in dev where no updater
 * endpoint is reachable) so it never blocks app usage.
 */
export function useUpdater() {
  const checked = useRef(false);

  useEffect(() => {
    // Guard against React 19 StrictMode running effects twice in dev.
    if (checked.current) return;
    checked.current = true;

    (async () => {
      try {
        const update = await check();
        if (!update) return;

        const shouldInstall = await ask(
          `WorktreeManager ${update.version} is available ` +
            `(you have ${update.currentVersion}).\n\nDownload and install it now?`,
          {
            title: "Update available",
            kind: "info",
            okLabel: "Update",
            cancelLabel: "Later",
          }
        );
        if (!shouldInstall) return;

        await update.downloadAndInstall();
        await relaunch();
      } catch (err) {
        console.error("Update check failed:", err);
      }
    })();
  }, []);
}
