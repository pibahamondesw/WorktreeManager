const fs = require("node:fs");
const path = require("node:path");
const childProcess = require("node:child_process");
const { syncBuiltinESMExports } = require("node:module");

const controlRoot = process.env.WTM_EDITOR_CONTROL;

function forgetExitedGroup(directory, pid) {
  const retry = () => setTimeout(() => forgetExitedGroup(directory, pid), 1000).unref();
  try {
    process.kill(-pid, 0);
    retry();
    return;
  } catch (error) {
    if (error.code !== "ESRCH") return;
  }
  const lock = path.join(directory, "lock");
  try {
    fs.mkdirSync(lock);
  } catch (error) {
    if (error.code === "EEXIST") retry();
    return;
  }
  try {
    if (!fs.existsSync(path.join(directory, "active.json"))) return;
    const groups = path.join(directory, "groups");
    const remaining = fs
      .readFileSync(groups, "utf8")
      .split("\n")
      .filter((line) => line !== String(pid));
    fs.writeFileSync(groups, remaining.join("\n"));
  } catch {
    retry();
  } finally {
    try {
      fs.rmdirSync(lock);
    } catch {}
  }
}

function track(env, spawn) {
  const generation = env?.WTM_EDITOR_SESSION;
  if (!controlRoot) return spawn();
  if (!/^[a-f0-9]{48}$/.test(generation ?? "")) {
    throw new Error("Missing WTM editor session");
  }
  const directory = path.join(controlRoot, generation);
  const lock = path.join(directory, "lock");
  const deadline = Date.now() + 3000;
  while (true) {
    try {
      fs.mkdirSync(lock);
      break;
    } catch (error) {
      if (error.code !== "EEXIST" || Date.now() >= deadline) throw error;
      Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 10);
    }
  }
  try {
    const session = JSON.parse(fs.readFileSync(path.join(directory, "active.json"), "utf8"));
    if (session.taskId !== env.WTM_TASK_ID) throw new Error("WTM editor session mismatch");
    const child = spawn();
    if (!Number.isInteger(child.pid) || child.pid <= 1)
      throw new Error("Editor process did not start");
    try {
      fs.appendFileSync(path.join(directory, "groups"), `${child.pid}\n`);
    } catch (error) {
      try {
        process.kill(-child.pid, "SIGKILL");
      } catch {}
      throw error;
    }
    const exited = () => forgetExitedGroup(directory, child.pid);
    if (typeof child.onExit === "function") child.onExit(exited);
    else child.once("exit", exited);
    return child;
  } finally {
    fs.rmdirSync(lock);
  }
}

if (controlRoot) {
  const fork = childProcess.fork;
  childProcess.fork = function (modulePath, args, options) {
    if (!Array.isArray(args) || !args.includes("--type=extensionHost")) {
      return fork.call(this, modulePath, args, options);
    }
    return track(options?.env, () =>
      fork.call(this, modulePath, args, { ...options, detached: true })
    );
  };
  syncBuiltinESMExports();
}

globalThis.__wtmSpawnPty = (spawn, file, args, options) =>
  track(options?.env, () => spawn(file, args, options));
