import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { spawnSync } from "node:child_process";

let directory: string;
const runner = resolve("scripts/run-dev.sh");

beforeEach(() => {
  directory = mkdtempSync(join(tmpdir(), "wtm-runner-"));
  writeFileSync(
    join(directory, "codesign"),
    `#!/bin/sh
printf '%s\\n' "$@" >> "$RUNNER_LOG"
case "$1" in
  --force) exit "\${SIGN_EXIT:-0}" ;;
  --verify) exit "\${VERIFY_EXIT:-0}" ;;
esac
`,
    { mode: 0o700 }
  );
  writeFileSync(
    join(directory, "app with spaces"),
    `#!/bin/sh
printf 'launched\\n%s\\n' "$@" >> "$RUNNER_LOG"
exit 7
`,
    { mode: 0o700 }
  );
});
afterEach(() => rmSync(directory, { recursive: true, force: true }));

function run(env: NodeJS.ProcessEnv = {}) {
  const result = spawnSync(
    "/bin/sh",
    [runner, join(directory, "app with spaces"), "arg with spaces"],
    {
      env: {
        ...process.env,
        PATH: `${directory}:${process.env.PATH}`,
        RUNNER_LOG: join(directory, "log"),
        WTM_DEV_SIGNING_IDENTITY: "WorktreeManager Development",
        ...env,
      },
      encoding: "utf8",
    }
  );
  return result;
}

describe("development signing runner", () => {
  it("signs and verifies before launch, preserving arguments and exit status", () => {
    expect(run().status).toBe(7);
    const log = readFileSync(join(directory, "log"), "utf8");
    expect(log).toContain(
      "--sign\nWorktreeManager Development\n--identifier\ncom.worktreemanager.dev"
    );
    expect(log.indexOf("--verify")).toBeLessThan(log.indexOf("launched"));
    expect(log).toContain("launched\narg with spaces\n");
  });

  it.each([{ SIGN_EXIT: "1" }, { VERIFY_EXIT: "1" }])(
    "does not launch after a signing or verification failure",
    (env) => {
      expect(run(env).status).toBe(1);
      expect(readFileSync(join(directory, "log"), "utf8")).not.toContain("launched");
    }
  );

  it("rejects ad-hoc signing for shared development", () => {
    const result = run({ WTM_DEV_SIGNING_IDENTITY: "-" });
    expect(result.status).toBe(1);
    expect(result.stderr).toContain("requires a certificate");
  });

  it("preserves ordinary cargo execution when signing is not enabled", () => {
    expect(run({ WTM_DEV_SIGNING_IDENTITY: "" }).status).toBe(7);
    expect(readFileSync(join(directory, "log"), "utf8")).toBe("launched\narg with spaces\n");
  });
});
