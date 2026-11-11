import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { spawnSync } from "node:child_process";

let directory: string;
const installer = resolve("scripts/apt-install.sh");
const mirrorList = "URIs: mirror+file:/etc/apt/apt-mirrors.txt\nSuites: noble noble-updates\n";

beforeEach(() => {
  directory = mkdtempSync(join(tmpdir(), "wtm-apt-"));
  writeFileSync(join(directory, "sudo"), `#!/bin/sh\nexec "$@"\n`, { mode: 0o700 });
  writeFileSync(
    join(directory, "apt-get"),
    `#!/bin/sh
printf '%s\\n' "$*" >> "$APT_LOG"
if [ "$1" = update ]; then
  count=$(grep -c '^update' "$APT_LOG")
  [ "$count" -gt "\${UPDATE_FAILURES:-0}" ]
fi
`,
    { mode: 0o700 }
  );
  writeFileSync(join(directory, "ubuntu.sources"), mirrorList);
});
afterEach(() => rmSync(directory, { recursive: true, force: true }));

function install(failures: number) {
  const result = spawnSync("/bin/sh", [installer, "zsh", "patchelf"], {
    env: {
      ...process.env,
      PATH: `${directory}:${process.env.PATH}`,
      APT_LOG: join(directory, "log"),
      APT_SOURCES: join(directory, "ubuntu.sources"),
      UPDATE_FAILURES: String(failures),
    },
    encoding: "utf8",
  });
  return {
    ...result,
    log: readFileSync(join(directory, "log"), "utf8").trim().split("\n"),
    sources: readFileSync(join(directory, "ubuntu.sources"), "utf8"),
  };
}

const INSTALL = "install -y -o Acquire::Retries=3 zsh patchelf";

describe("apt installer", () => {
  it("keeps the runner's mirrors when they answer", () => {
    const result = install(0);
    expect(result.status).toBe(0);
    expect(result.log).toEqual(["update --error-on=any", INSTALL]);
    expect(result.sources).toBe(mirrorList);
  });

  it("falls back to archive.ubuntu.com when the mirror list fails", () => {
    const result = install(1);
    expect(result.status).toBe(0);
    expect(result.log).toEqual(["update --error-on=any", "update --error-on=any", INSTALL]);
    expect(result.sources).toBe(
      "URIs: http://archive.ubuntu.com/ubuntu/\nSuites: noble noble-updates\n"
    );
  });

  it("fails without installing when archive.ubuntu.com fails too", () => {
    const result = install(2);
    expect(result.status).toBe(1);
    expect(result.stderr).toContain("even from archive.ubuntu.com");
    expect(result.log).not.toContain(INSTALL);
  });

  it("fails without retrying when the sources do not use a mirror list", () => {
    writeFileSync(join(directory, "ubuntu.sources"), "URIs: http://archive.ubuntu.com/ubuntu/\n");
    const result = install(1);
    expect(result.status).toBe(1);
    expect(result.log).toEqual(["update --error-on=any"]);
  });
});
