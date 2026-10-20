import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { spawnSync } from "node:child_process";
import {
  copyFileSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

let directory: string;

function executable(name: string, source: string) {
  writeFileSync(join(directory, "bin", name), `#!${process.execPath}\n${source}`, {
    mode: 0o700,
  });
}

beforeEach(() => {
  directory = mkdtempSync(join(tmpdir(), "wtm-rust-coverage-"));
  mkdirSync(join(directory, "bin"));
  mkdirSync(join(directory, "scripts"));
  mkdirSync(join(directory, "src-tauri"));
  copyFileSync(resolve("scripts/rust-coverage.sh"), join(directory, "scripts/rust-coverage.sh"));
  copyFileSync(
    resolve("scripts/rust-coverage-report.py"),
    join(directory, "scripts/rust-coverage-report.py")
  );
  executable(
    "git",
    `const fs = require("node:fs");
const args = process.argv.slice(2);
fs.appendFileSync(process.env.COVERAGE_LOG, JSON.stringify({tool:"git", args}) + "\\n");
if (args[0] === "merge-base") console.log("fixture-ancestor");
else if (args[0] === "ls-files" && process.env.TEST_UNTRACKED_FILE) process.stdout.write(process.env.TEST_UNTRACKED_FILE + "\\0");
else if (args.includes("--no-index")) { process.stdout.write("new source patch\\n"); process.exit(1); }
else if (args[0] === "diff") process.stdout.write("fixture patch\\n");`
  );
  executable("uname", 'console.log(process.env.TEST_PLATFORM || "Darwin");');
  executable(
    "rustup",
    `const fs = require("node:fs");
const path = require("node:path");
const args = process.argv.slice(2);
fs.appendFileSync(process.env.COVERAGE_LOG, JSON.stringify({
  tool: "rustup", args,
  target: process.env.CARGO_TARGET_DIR,
  flags: process.env.RUSTFLAGS,
}) + "\\n");
if (args.includes("--version")) {
  console.log(process.env.TEST_VERSION || "cargo-llvm-cov 0.9.1");
} else if (args.includes("rustc")) {
  console.log("rustc 1.93.1");
} else if (args.includes("show-env")) {
  console.log("export LLVM_PROFILE_FILE='" + process.env.CARGO_TARGET_DIR + "/test-%p.profraw'");
} else if (args[3] === "test") {
  if (process.env.BUILD_EXIT) process.exit(Number(process.env.BUILD_EXIT));
  console.log("non-JSON compiler output");
  for (const [name, kind, test] of [
    ["app_lib", ["rlib", "cdylib", "staticlib"], false],
    ["app_lib", ["rlib", "cdylib", "staticlib"], true],
    ["menu", ["test"], false],
  ]) {
    if (process.env.MISSING_MENU && name === "menu") continue;
    console.log(JSON.stringify({
      reason: "compiler-artifact", target: { name, kind }, profile: { test },
      executable: path.join(process.cwd(), "bin", test ? "unit tests" : name),
      fresh: process.env.FRESH === "1",
    }));
  }
  console.log(JSON.stringify({reason: "build-finished", success: true}));
} else if (args.includes("report")) {
  fs.writeFileSync(args[args.indexOf("--output-path") + 1], args.includes("--json") && !args.includes("--summary-only") ? '{"data":[{"functions":[]}]}' : "report");
}
`
  );
  for (const name of ["unit tests", "menu"]) {
    executable(
      name,
      `const fs = require("node:fs");
const tool = ${JSON.stringify(name)};
const args = process.argv.slice(2);
fs.appendFileSync(process.env.COVERAGE_LOG, JSON.stringify({
  tool, args, cwd: process.cwd(), profile: process.env.LLVM_PROFILE_FILE,
}) + "\\n");
if (tool === "unit tests" && args.length === 0) process.exit(99);
process.exit(Number(process.env[tool === "menu" ? "MENU_EXIT" : "UNIT_EXIT"] || "0"));
`
    );
  }
  executable(
    "diff-cover",
    `const fs = require("node:fs");
fs.appendFileSync(process.env.COVERAGE_LOG, JSON.stringify({tool: "diff-cover", args: process.argv.slice(2)}) + "\\n");
process.exit(Number(process.env.DIFF_EXIT || "0"));
`
  );
});

afterEach(() => rmSync(directory, { recursive: true, force: true }));

function run(env: NodeJS.ProcessEnv = {}) {
  return spawnSync("/bin/sh", [join(directory, "scripts/rust-coverage.sh")], {
    env: {
      ...process.env,
      PATH: `${join(directory, "bin")}:${process.env.PATH}`,
      CARGO_TARGET_DIR: join(directory, "target"),
      CARGO_LLVM_COV_TARGET_DIR: "",
      RUSTFLAGS: "",
      COVERAGE_BASE: "fixture-base",
      COVERAGE_LOG: join(directory, "calls.jsonl"),
      ...env,
    },
    encoding: "utf8",
    timeout: 15_000,
  });
}

type RecordedCommand = {
  tool: string;
  args: string[];
  target?: string;
  flags?: string;
  cwd?: string;
  profile?: string;
};

function calls(): RecordedCommand[] {
  return readFileSync(join(directory, "calls.jsonl"), "utf8")
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line) as RecordedCommand);
}

describe("Rust coverage pipeline", () => {
  it.each([{ FRESH: "0" }, { FRESH: "1" }])(
    "builds once and runs the selected compiled tests, including cached artifacts (%j)",
    (env) => {
      const result = run(env);
      expect(result.status).toBe(0);
      const commands = calls();
      const builds = commands.filter((call) => call.tool === "rustup" && call.args[3] === "test");
      expect(builds).toHaveLength(1);
      expect(builds[0].args).toEqual([
        "run",
        "1.93.1",
        "cargo",
        "test",
        "--locked",
        "--manifest-path",
        "src-tauri/Cargo.toml",
        "--lib",
        "--test",
        "menu",
        "--no-run",
        "--message-format=json-render-diagnostics",
      ]);
      expect(commands.find((call) => call.tool === "unit tests")?.args).toEqual([
        "cli::tests::",
        "commands::claude_config::tests::",
        "commands::export::tests::",
        "commands::github::tests::",
        "commands::code_server::view::tests::",
        "commands::code_server::tests::",
        "commands::embedded::",
        "commands::editor::tests::",
        "commands::opencode::",
        "commands::chat::",
        "commands::agents::",
        "commands::agent_alerts::tests::",
        "commands::agent_usage::tests::",
        "commands::claude_usage::tests::",
        "commands::codex_usage::tests::",
        "commands::login_shell::tests::",
        "commands::doctor::tests::",
        "commands::doppler::tests::",
        "commands::node_deps::tests::",
        "commands::python_deps::tests::",
        "commands::vault_agents::tests::",
        "commands::terminal::tests::",
        "commands::scripts::tests::",
        "menu::",
      ]);
      expect(commands.find((call) => call.tool === "menu")?.args).toEqual([]);
      const tests = commands.filter((call) => call.tool === "unit tests" || call.tool === "menu");
      expect(tests).toHaveLength(2);
      expect(tests.every((call) => call.cwd === realpathSync(join(directory, "src-tauri")))).toBe(
        true
      );
      expect(tests.every((call) => call.profile?.endsWith("/test-%p.profraw"))).toBe(true);
      expect(commands.at(-1)?.args).toContain("--fail-under=100");
      expect(commands.at(-1)?.args).toContain("--compare-branch=fixture-base");
      expect(commands.at(-1)?.args).toContain("--diff-file=coverage/rust/macos/changes.diff");
      expect(readFileSync(join(directory, "coverage/rust/macos/changes.diff"), "utf8")).toBe(
        "fixture patch\n"
      );
      expect(readFileSync(join(directory, "coverage/rust/macos/diff-lcov.info"), "utf8")).toBe(
        "report"
      );
      expect(
        commands.find((call) => call.tool === "git" && call.args[0] === "merge-base")?.args
      ).toEqual(["merge-base", "fixture-base", "HEAD"]);
      expect(commands.find((call) => call.tool === "git" && call.args[0] === "diff")?.args).toEqual(
        ["diff", "fixture-ancestor", "--", "src-tauri/src"]
      );
      expect(readFileSync(join(directory, "coverage/rust/macos/lcov.info"), "utf8")).toBe("report");
      expect(readFileSync(join(directory, "coverage/rust/macos/summary.json"), "utf8")).toBe(
        "report"
      );
    }
  );

  it("cleans only the instrumented workspace and preserves configured compiler flags", () => {
    const flags = "-C debuginfo=line-tables-only --remap-path-prefix=tests/../src=src";
    const target = join(directory, "custom coverage target");
    expect(run({ RUSTFLAGS: flags, CARGO_LLVM_COV_TARGET_DIR: target }).status).toBe(0);
    const commands = calls().filter((call) => call.tool === "rustup");
    expect(commands.every((call) => call.target === target && call.flags === flags)).toBe(true);
    expect(commands.find((call) => call.args.includes("clean"))?.args).toContain("--workspace");
  });

  it.each([{ UNIT_EXIT: "7" }, { MENU_EXIT: "8" }])(
    "reports coverage and runs both targets even when a test target fails (%j)",
    (env) => {
      expect(run(env).status).toBe(Number(env.UNIT_EXIT || env.MENU_EXIT));
      expect(
        calls().filter((call) => call.tool === "unit tests" || call.tool === "menu")
      ).toHaveLength(2);
      expect(calls().at(-1)?.tool).toBe("diff-cover");
    }
  );

  it("fails the diff coverage gate", () => {
    expect(run({ DIFF_EXIT: "1" }).status).toBe(1);
  });

  it("includes untracked Rust source files in the current diff without staging them", () => {
    const name = "src-tauri/src/new file.rs";
    expect(run({ TEST_UNTRACKED_FILE: name }).status).toBe(0);
    expect(
      calls().find((call) => call.tool === "git" && call.args.includes("--no-index"))?.args
    ).toEqual(["diff", "--no-index", "--", "/dev/null", name]);
    expect(readFileSync(join(directory, "coverage/rust/macos/changes.diff"), "utf8")).toBe(
      "fixture patch\nnew source patch\n"
    );
  });

  it("rejects missing compiled targets before running partial coverage", () => {
    const result = run({ MISSING_MENU: "1" });
    expect(result.status).toBe(1);
    expect(result.stderr).toContain(
      "Expected the app_lib unit tests and native menu test executable"
    );
    expect(calls().some((call) => call.tool === "unit tests" || call.tool === "menu")).toBe(false);
  });

  it("does not run tests after a compilation failure", () => {
    expect(run({ BUILD_EXIT: "9" }).status).toBe(9);
    expect(calls().some((call) => call.tool === "unit tests" || call.tool === "menu")).toBe(false);
  });

  it("rejects an unexpected coverage tool version", () => {
    const result = run({ TEST_VERSION: "cargo-llvm-cov 0.8.0" });
    expect(result.status).toBe(1);
    expect(result.stderr).toContain("Expected cargo-llvm-cov 0.9.1");
    expect(calls()).toHaveLength(1);
  });

  it("requires macOS", () => {
    const result = run({ TEST_PLATFORM: "Linux" });
    expect(result.status).toBe(1);
    expect(result.stderr).toContain("requires macOS");
  });
});
