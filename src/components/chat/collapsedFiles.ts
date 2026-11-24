import { FileChange } from "../../services/chat";

const LOCK_FILES = new Set([
  "bun.lock",
  "bun.lockb",
  "Cargo.lock",
  "composer.lock",
  "deno.lock",
  "flake.lock",
  "Gemfile.lock",
  "go.sum",
  "mix.lock",
  "npm-shrinkwrap.json",
  "package-lock.json",
  "packages.lock.json",
  "Pipfile.lock",
  "pnpm-lock.yaml",
  "Podfile.lock",
  "poetry.lock",
  "pubspec.lock",
  "uv.lock",
  "yarn.lock",
]);

const LARGE_DIFF_LINES = 1500;

export function collapsedReason(file: FileChange): string | null {
  if (LOCK_FILES.has(file.path.slice(file.path.lastIndexOf("/") + 1))) return "Lock file";
  if (file.generated) return "Generated file";
  if (file.additions + file.deletions > LARGE_DIFF_LINES) return "Large diff";
  return null;
}
