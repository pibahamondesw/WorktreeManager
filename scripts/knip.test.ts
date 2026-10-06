import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { spawnSync } from "node:child_process";
import {
  copyFileSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";

let directory: string;
const knip = resolve("node_modules/knip/bin/knip.js");

function write(path: string, content: string) {
  const target = join(directory, path);
  mkdirSync(dirname(target), { recursive: true });
  writeFileSync(target, content);
}

function scan(...args: string[]) {
  return spawnSync(process.execPath, [knip, "--no-progress", ...args], {
    cwd: directory,
    encoding: "utf8",
    timeout: 15_000,
  });
}

beforeEach(() => {
  directory = mkdtempSync(join(tmpdir(), "wtm-knip-"));
  copyFileSync(resolve("knip.json"), join(directory, "knip.json"));
  symlinkSync(resolve("node_modules"), join(directory, "node_modules"), "dir");
  write(
    "package.json",
    JSON.stringify({
      private: true,
      type: "module",
      scripts: { release: "node scripts/bump-version.mjs" },
      dependencies: { "@tauri-apps/api": "*" },
      devDependencies: { vite: "*", vitest: "*", tailwindcss: "*" },
    })
  );
  write("index.html", '<script type="module" src="/src/main.tsx"></script>');
  write("vite.config.ts", "export default {};");
  write(
    "src/main.tsx",
    'import "./index.css"; import { invoke } from "@tauri-apps/api/core"; void invoke("native_command");'
  );
  write("src/index.css", '@import "tailwindcss";');
  write("src/testHelper.ts", 'export const testOnly = "value";');
  write(
    "src/example.test.ts",
    'import { expect, it } from "vitest"; import { testOnly } from "./testHelper"; it("works", () => expect(testOnly).toBe("value"));'
  );
  write("scripts/bump-version.mjs", 'console.log("release entry");');
  write(".semgrep/tests/example.ts", "export const securityFixture = true;");
});

afterEach(() => rmSync(directory, { recursive: true, force: true }));

describe("Knip configuration", () => {
  it("recognizes Vite, Vitest, CSS dependencies, release scripts and native calls", () => {
    const result = scan();
    expect(result.stderr).toBe("");
    expect(result.stdout).toBe("");
    expect(result.status).toBe(0);
  });

  it("reports unused files, exports and dependencies without blocking the CI pilot", () => {
    const originalPackage = readFileSync(join(directory, "package.json"), "utf8");
    write("src/unused.ts", "export const unusedFile = true;");
    write(
      "src/testHelper.ts",
      'export const testOnly = "value"; export const unusedExport = true; export type UnusedType = string;'
    );
    write("scripts/unused.mjs", "export const unusedScript = true;");
    write(
      "package.json",
      JSON.stringify({
        private: true,
        type: "module",
        scripts: { release: "node scripts/bump-version.mjs" },
        dependencies: { "@tauri-apps/api": "*", uuid: "*" },
        devDependencies: { vite: "*", vitest: "*", tailwindcss: "*" },
      })
    );

    const strict = scan();
    expect(strict.status).toBe(1);
    expect(strict.stdout).toContain("src/unused.ts");
    expect(strict.stdout).toContain("scripts/unused.mjs");
    expect(strict.stdout).toContain("unusedExport");
    expect(strict.stdout).toContain("UnusedType");
    expect(strict.stdout).toContain("uuid");

    const report = scan("--no-exit-code");
    expect(report.status).toBe(0);
    expect(report.stdout).toBe(strict.stdout);

    rmSync(join(directory, "src/unused.ts"));
    rmSync(join(directory, "scripts/unused.mjs"));
    write("src/testHelper.ts", 'export const testOnly = "value";');
    write("package.json", originalPackage);
    const corrected = scan();
    expect(corrected.stdout).toBe("");
    expect(corrected.stderr).toBe("");
    expect(corrected.status).toBe(0);
  });

  it("fails on invalid configuration even in report mode", () => {
    write("knip.json", "{");
    const result = scan("--no-exit-code");
    expect(result.status).toBe(2);
    expect(result.stderr).not.toBe("");
  });
});
