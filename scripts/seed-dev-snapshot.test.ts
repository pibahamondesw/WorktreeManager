import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { spawnSync } from "node:child_process";

let directory: string;
const seeder = resolve("scripts/seed-dev-snapshot.sh");
const sourceStore = () => join(directory, "support/com.worktreemanager.dev/store.json");
const snapshotStore = () => join(directory, "support/com.worktreemanager.dev.snapshot/store.json");

beforeEach(() => {
  directory = mkdtempSync(join(tmpdir(), "wtm-snapshot-"));
  writeFileSync(
    join(directory, "security"),
    `#!/bin/sh
case "$1" in
  find-generic-password) [ -n "\${SECRET:-}" ] && printf '%s\\n' "$SECRET" || exit 44 ;;
  -i) cat >> "$SECURITY_LOG" ;;
esac
`,
    { mode: 0o700 }
  );
});
afterEach(() => rmSync(directory, { recursive: true, force: true }));

function seed(env: NodeJS.ProcessEnv = {}) {
  return spawnSync("/bin/sh", [seeder, "sh", "-c", 'echo "launched $1"', "_", "arg with spaces"], {
    env: {
      ...process.env,
      PATH: `${directory}:${process.env.PATH}`,
      WTM_APP_SUPPORT_DIR: join(directory, "support"),
      SECURITY_LOG: join(directory, "security.log"),
      ...env,
    },
    encoding: "utf8",
  });
}

function writeSourceStore(content: string) {
  mkdirSync(join(directory, "support/com.worktreemanager.dev"), { recursive: true });
  writeFileSync(sourceStore(), content);
}

describe("development snapshot seeder", () => {
  it("copies state and credentials into the snapshot profile before launching", () => {
    writeSourceStore('{"workspaces":[]}');
    const result = seed({ SECRET: '{"ws1":"lin_api_key"}' });

    expect(result.status).toBe(0);
    expect(result.stdout).toBe("launched arg with spaces\n");
    expect(readFileSync(snapshotStore(), "utf8")).toBe('{"workspaces":[]}');
    const hex = Buffer.from('{"ws1":"lin_api_key"}').toString("hex");
    expect(readFileSync(join(directory, "security.log"), "utf8")).toBe(
      `add-generic-password -U -s com.worktreemanager.dev.snapshot -a linear-api-keys -X ${hex}\n`
    );
  });

  it("refreshes an existing snapshot", () => {
    writeSourceStore("old");
    seed();
    writeSourceStore("new");
    seed();
    expect(readFileSync(snapshotStore(), "utf8")).toBe("new");
  });

  it("launches without credentials when the installed app has none", () => {
    writeSourceStore("{}");
    const result = seed();
    expect(result.status).toBe(0);
    expect(result.stderr).toContain("starts without them");
  });

  it("does not launch without installed-app state", () => {
    const result = seed();
    expect(result.status).toBe(1);
    expect(result.stdout).not.toContain("launched");
  });
});
