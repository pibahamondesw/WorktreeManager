#!/usr/bin/env node
/**
 * Bump the app version in the three places that must stay in lockstep, then
 * (unless --no-tag) create the matching `vX.Y.Z` git tag. Pushing that tag is
 * what triggers the release workflow (.github/workflows/release.yml).
 *
 * Usage:
 *   npm run release 0.2.0            # bump + commit + tag
 *   npm run release 0.2.0 --no-tag   # just rewrite the version files
 *
 * The three sources:
 *   - package.json                 ("version")
 *   - src-tauri/tauri.conf.json    ("version")  <- authoritative for the bundle
 *   - src-tauri/Cargo.toml         (version = "...")
 */
import { readFileSync, writeFileSync } from "node:fs";
import { execSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

const args = process.argv.slice(2);
const noTag = args.includes("--no-tag");
const version = args.find((a) => !a.startsWith("--"));

if (!version || !/^\d+\.\d+\.\d+(-[\w.]+)?$/.test(version)) {
  console.error(
    "Usage: npm run release <version> [--no-tag]\n" +
      "  <version> must be semver, e.g. 0.2.0 or 0.2.0-rc.1"
  );
  process.exit(1);
}

// package.json — "version": "x.y.z"
const pkgPath = join(root, "package.json");
const pkg = readFileSync(pkgPath, "utf8");
writeFileSync(
  pkgPath,
  pkg.replace(/("version":\s*")[^"]+(")/, `$1${version}$2`)
);

// src-tauri/tauri.conf.json — "version": "x.y.z"
const confPath = join(root, "src-tauri", "tauri.conf.json");
const conf = readFileSync(confPath, "utf8");
writeFileSync(
  confPath,
  conf.replace(/("version":\s*")[^"]+(")/, `$1${version}$2`)
);

// src-tauri/Cargo.toml — version = "x.y.z" (first occurrence, the [package] one)
const cargoPath = join(root, "src-tauri", "Cargo.toml");
const cargo = readFileSync(cargoPath, "utf8");
writeFileSync(
  cargoPath,
  cargo.replace(/^(version\s*=\s*")[^"]+(")/m, `$1${version}$2`)
);

console.log(`Bumped version to ${version} in package.json, tauri.conf.json, Cargo.toml`);

try {
  execSync("npm install --package-lock-only", { cwd: root, stdio: "ignore" });
} catch {
  // Non-fatal: CI's lockfile check will catch a stale package-lock.json.
}

if (noTag) {
  console.log("Skipped git commit/tag (--no-tag). Remember to update Cargo.lock via a build.");
  process.exit(0);
}

// Keep Cargo.lock in sync so the tagged commit is buildable as-is.
try {
  execSync("cargo update -p worktree-manager --precise " + version, {
    cwd: join(root, "src-tauri"),
    stdio: "ignore",
  });
} catch {
  // Non-fatal: the release build will regenerate Cargo.lock anyway.
}

execSync(
  `git add package.json package-lock.json src-tauri/tauri.conf.json src-tauri/Cargo.toml src-tauri/Cargo.lock`,
  { cwd: root, stdio: "inherit" }
);
execSync(`git commit -m "Release v${version}"`, { cwd: root, stdio: "inherit" });
execSync(`git tag v${version}`, { cwd: root, stdio: "inherit" });

console.log(
  `\nCreated commit + tag v${version}.\n` +
    `Push it to trigger the release:\n\n` +
    `  git push && git push origin v${version}\n`
);
