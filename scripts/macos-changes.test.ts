import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, unlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";

let directory: string;

function git(...args: string[]) {
  const result = spawnSync("git", args, { cwd: directory, encoding: "utf8" });
  expect({ status: result.status, stderr: result.stderr }).toMatchObject({ status: 0 });
  return result.stdout.trim();
}

function write(path: string, content: string) {
  mkdirSync(dirname(join(directory, path)), { recursive: true });
  writeFileSync(join(directory, path), content);
}

function commit(files: Record<string, string>) {
  for (const [path, content] of Object.entries(files)) write(path, content);
  git("add", "-A");
  git("commit", "-q", "-m", "fixture");
  return git("rev-parse", "HEAD");
}

function detect(base: string, cwd = directory) {
  const result = spawnSync("python3", ["-B", resolve("scripts/macos-changes.py"), base], {
    cwd,
    encoding: "utf8",
  });
  expect({ status: result.status, stderr: result.stderr }).toMatchObject({ status: 0 });
  return result.stdout.split("\n").filter(Boolean);
}

let base: string;

beforeEach(() => {
  directory = mkdtempSync(join(tmpdir(), "wtm-macos-changes-"));
  git("init", "-q", "-b", "main");
  git("config", "user.email", "ci@example.com");
  git("config", "user.name", "CI");
  git("config", "commit.gpgsign", "false");
  base = commit({
    "README.md": "readme\n",
    "src/App.tsx": "export {};\n",
    "src-tauri/src/lib.rs": "mod commands;\nmod menu;\n",
    "src-tauri/src/menu.rs":
      '#[cfg(target_os = "macos")]\n#[path = "menu/editor.rs"]\nmod editor;\n',
    "src-tauri/src/commands/mod.rs": "pub mod git;\npub mod native;\npub mod stubs;\n",
    "src-tauri/src/commands/git.rs": "pub fn status() {}\n",
    "src-tauri/src/commands/native.rs":
      '#[cfg(target_os = "macos")]\n#[allow(dead_code)]\npub(crate) mod bridge;\n#[cfg(all(test, target_os = "macos"))]\nmod tests;\n',
    "src-tauri/src/commands/stubs.rs": '#[cfg(not(target_os = "macos"))]\nmod fallback;\n',
    "src-tauri/src/commands/removed.rs": 'pub const NAME: &str = "NSString";\n',
  });
});

afterEach(() => rmSync(directory, { recursive: true, force: true }));

describe("macOS change detection", () => {
  it("ignores cross-platform Rust, frontend and documentation changes", () => {
    commit({
      "README.md": "changed\n",
      "src/App.tsx": "export const value = 1;\n",
      "src-tauri/src/commands/git.rs": "pub fn status() { let _ = 1; }\n",
      "src-tauri/src/commands/stubs/fallback.rs": "pub fn run() {}\n",
      ".github/workflows/release.yml": "name: Release\n",
    });
    expect(detect(base)).toEqual([]);
  });

  it.each([
    ['#[cfg(target_os = "macos")]\nfn native() {}\n'],
    ['fn native() -> bool { cfg!(target_os="macos") }\n'],
    ['#[cfg_attr(target_os = "macos", allow(deprecated))]\nfn native() {}\n'],
    ['#[cfg(not(target_os = "macos"))]\nfn fallback() {}\n'],
    ["use objc2::runtime::AnyObject;\n"],
    ["fn call() { unsafe { msg_send![center, removeAllDeliveredNotifications] } }\n"],
    ["fn class() -> &'static AnyClass { todo!() }\n"],
    ['const CLASS: &str = "NSUserNotificationCenter";\n'],
  ])("flags Rust files that contain macOS code (%j)", (source) => {
    commit({ "src-tauri/src/commands/git.rs": source });
    expect(detect(base)).toEqual(["src-tauri/src/commands/git.rs"]);
  });

  it("flags removed macOS code and deleted macOS files", () => {
    write("src-tauri/src/commands/git.rs", "pub fn status() {}\n");
    commit({ "src-tauri/src/commands/git.rs": '#[cfg(target_os = "macos")]\nfn native() {}\n' });
    const withMacosCode = git("rev-parse", "HEAD");
    commit({ "src-tauri/src/commands/git.rs": "pub fn status() {}\n" });
    expect(detect(withMacosCode)).toEqual(["src-tauri/src/commands/git.rs"]);

    unlinkSync(join(directory, "src-tauri/src/commands/removed.rs"));
    commit({});
    expect(detect(base)).toEqual(["src-tauri/src/commands/removed.rs"]);
  });

  it("flags modules declared under a macOS cfg, including their submodules and path attributes", () => {
    commit({
      "src-tauri/src/commands/native/bridge.rs": "pub fn run() {}\n",
      "src-tauri/src/commands/native/bridge/delegate.rs": "pub fn run() {}\n",
      "src-tauri/src/commands/native/tests/mod.rs": "fn helper() {}\n",
      "src-tauri/src/menu/editor.rs": "pub fn install() {}\n",
    });
    expect(detect(base)).toEqual([
      "src-tauri/src/commands/native/bridge.rs",
      "src-tauri/src/commands/native/bridge/delegate.rs",
      "src-tauri/src/commands/native/tests/mod.rs",
      "src-tauri/src/menu/editor.rs",
    ]);
  });

  it.each([
    "src-tauri/src/commands/login_shell.rs",
    "src-tauri/src/commands/agent_notifications.rs",
    "src-tauri/src/commands/embedded/registry.rs",
    "src-tauri/src/menu/shortcuts.rs",
    "src-tauri/src/lifecycle.rs",
    "src-tauri/tests/menu.rs",
  ])("flags the native area %s", (path) => {
    commit({ [path]: "pub fn cross_platform() {}\n" });
    expect(detect(base)).toEqual([path]);
  });

  it.each([
    "src-tauri/Cargo.toml",
    "src-tauri/Cargo.lock",
    "src-tauri/tauri.conf.json",
    "src-tauri/tauri.snapshot.conf.json",
    ".github/workflows/ci.yml",
  ])("flags the build input %s", (path) => {
    commit({ [path]: "changed\n" });
    expect(detect(base)).toEqual([path]);
  });

  it("includes uncommitted and untracked files from any working directory", () => {
    write("src-tauri/src/commands/git.rs", '#[cfg(target_os = "macos")]\nfn native() {}\n');
    write("src-tauri/src/commands/untracked.rs", "use objc2::rc::Retained;\n");
    write("src-tauri/src/commands/plain.rs", "pub fn run() {}\n");
    expect(detect(base, join(directory, "src-tauri"))).toEqual([
      "src-tauri/src/commands/git.rs",
      "src-tauri/src/commands/untracked.rs",
    ]);
  });

  it("compares against the merge base so base-branch changes do not count", () => {
    git("switch", "-q", "-c", "feature");
    commit({ "src-tauri/src/commands/git.rs": "pub fn status() { let _ = 2; }\n" });
    git("switch", "-q", "main");
    commit({ "src-tauri/Cargo.toml": "changed\n" });
    const mainTip = git("rev-parse", "HEAD");
    git("switch", "-q", "feature");
    expect(detect(mainTip)).toEqual([]);
  });
});
