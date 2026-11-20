/**
 * True in the Windows build. The app was written for macOS; the few places that differ on
 * Windows (install hints, VS Code opening as a desktop app) branch on this.
 */
export const IS_WINDOWS = typeof navigator !== "undefined" && /Windows/i.test(navigator.userAgent);

/** True for `/unix/paths`, `C:\windows\paths` (either slash) and `\\server\share` paths. */
export function isAbsolutePath(path: string): boolean {
  return /^\/|^[A-Za-z]:[\\/]|^\\\\/.test(path);
}

/**
 * Joins `parts` onto `base` with the separator `base` already uses: `\` for a Windows path such
 * as `C:\Users\me`, `/` otherwise. Trailing separators on `base` are dropped.
 */
export function joinPath(base: string, ...parts: string[]): string {
  const sep = base.includes("\\") && !base.includes("/") ? "\\" : "/";
  const trimmed = base.replace(/[\\/]+$/, "");
  return [trimmed, ...parts].join(sep);
}

/**
 * A path in one comparable form: `/` separators, no trailing separator, and — for Windows drive
 * or UNC paths, whose file systems ignore case — lowercased. POSIX paths are left as they are.
 */
function comparablePath(path: string): string {
  const windows = /^[A-Za-z]:[\\/]|^\\\\/.test(path);
  const slashed = (windows ? path.replace(/\\/g, "/") : path).replace(/\/+$/, "");
  return windows ? slashed.toLowerCase() : slashed;
}

/** Whether `path` is `root` or inside it, whichever separators either uses. */
export function isWithinPath(path: string, root: string): boolean {
  const base = comparablePath(root);
  const target = comparablePath(path);
  return base !== "" && (target === base || target.startsWith(`${base}/`));
}
