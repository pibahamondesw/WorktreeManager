import { afterEach, beforeEach, expect, it } from "vitest";
import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

let directory: string;

beforeEach(() => {
  directory = mkdtempSync(join(tmpdir(), "wtm-coverage-report-"));
});

afterEach(() => rmSync(directory, { recursive: true, force: true }));

function normalize(source: string, regions: number[][], lcov: string) {
  const file = join(directory, "command.rs");
  writeFileSync(file, source);
  writeFileSync(
    join(directory, "input.json"),
    JSON.stringify({
      data: [{ functions: [{ name: "generated", filenames: [file], regions }] }],
      lcov: lcov.replaceAll("SOURCE", file),
    })
  );
  const result = spawnSync(
    "python3",
    [
      "-B",
      "-c",
      `import importlib.util, json, sys
spec = importlib.util.spec_from_file_location("report", sys.argv[1])
report = importlib.util.module_from_spec(spec)
spec.loader.exec_module(report)
data = json.load(open(sys.argv[2]))
print(report.normalize_lcov(data["lcov"], report.generated_command_regions(data)), end="")`,
      resolve("scripts/rust-coverage-report.py"),
      join(directory, "input.json"),
    ],
    { encoding: "utf8" }
  );
  expect({ status: result.status, stderr: result.stderr }).toEqual({ status: 0, stderr: "" });
  return result.stdout.replaceAll(file, "SOURCE");
}

it("removes only the compiler region on a standalone Tauri attribute and retains uncovered code", () => {
  const result = normalize(
    "#[tauri::command]\nfn run() {\n    restore_files();\n}\n",
    [[1, 17, 1, 18, 0, 0, 0, 0]],
    "SF:SOURCE\nDA:1,0\nDA:2,1\nDA:3,0\nLF:3\nLH:1\nend_of_record\n"
  );
  expect(result).toBe("SF:SOURCE\nDA:2,1\nDA:3,0\nLF:2\nLH:1\nend_of_record\n");
});

it.each([
  [[1, 1, 3, 2, 0, 0, 0, 0]],
  [[1, 1, 1, 18, 0, 0, 0, 0]],
  [[1, 17, 1, 18, 1, 0, 0, 0]],
  [[1, 17, 1, 18, 0, 0, 0, 1]],
  [[1, 1, 1, 2, 0, 0, 0, 0]],
])("keeps non-generated, covered and expansion regions (%j)", (regions) => {
  const lcov = "SF:SOURCE\nDA:1,0\nLF:1\nLH:0\nend_of_record\n";
  expect(normalize("#[tauri::command]\nfn run() {}\n", [regions], lcov)).toBe(lcov);
});

it.each(["#[tauri::command] fn run() {}\n", "fn run() {}\n", "#[derive(Clone)]\n"])(
  "does not drop executable lines or unrelated attributes (%s)",
  (source) => {
    const lcov = "SF:SOURCE\nDA:1,0\nLF:1\nLH:0\nend_of_record\n";
    expect(normalize(source, [[1, 17, 1, 18, 0, 0, 0, 0]], lcov)).toBe(lcov);
  }
);

it("retains raw records and nonzero line counts", () => {
  const lcov = "SF:SOURCE\nDA:1,2\nLF:1\nLH:1\nend_of_record\n";
  expect(normalize("#[tauri::command]\n", [[1, 17, 1, 18, 0, 0, 0, 0]], lcov)).toBe(lcov);
});

it("removes the generated function records while preserving uncovered command functions", () => {
  const result = normalize(
    "#[tauri::command]\nfn run() {}\n",
    [[1, 17, 1, 18, 0, 0, 0, 0]],
    "SF:SOURCE\nFN:1,generated\nFN:2,run\nFNDA:0,generated\nFNDA:0,run\nFNF:2\nFNH:0\nDA:1,0\nDA:2,0\nLF:2\nLH:0\nend_of_record\n"
  );
  expect(result).toBe(
    "SF:SOURCE\nFN:2,run\nFNDA:0,run\nFNF:1\nFNH:0\nDA:2,0\nLF:1\nLH:0\nend_of_record\n"
  );
});

it("retains functions that also have executable body regions", () => {
  const lcov =
    "SF:SOURCE\nFN:1,generated\nFNDA:0,generated\nFNF:1\nFNH:0\nDA:1,0\nDA:2,0\nLF:2\nLH:0\nend_of_record\n";
  expect(
    normalize(
      "#[tauri::command]\nfn run() {}\n",
      [
        [1, 17, 1, 18, 0, 0, 0, 0],
        [2, 1, 2, 12, 0, 0, 0, 0],
      ],
      lcov
    )
  ).toBe(lcov);
});
