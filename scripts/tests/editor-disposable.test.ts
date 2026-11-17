import { readFileSync } from "node:fs";
import { createContext, runInContext } from "node:vm";
import { describe, expect, it } from "vitest";

const source = readFileSync(
  new URL("../../src-tauri/src/commands/code_server/disposable.js", import.meta.url),
  "utf8"
);

const removeExplicitResourceManagement = `
  delete globalThis.DisposableStack;
  delete globalThis.AsyncDisposableStack;
  delete globalThis.SuppressedError;
  if ("dispose" in Symbol || "asyncDispose" in Symbol) {
    const nativeSymbol = Symbol;
    const symbolWithoutDisposal = function Symbol(description) {
      return nativeSymbol(description);
    };
    for (const key of Reflect.ownKeys(nativeSymbol)) {
      if (["dispose", "asyncDispose", "prototype", "length", "name"].includes(key)) continue;
      Object.defineProperty(symbolWithoutDisposal, key, Object.getOwnPropertyDescriptor(nativeSymbol, key));
    }
    globalThis.Symbol = symbolWithoutDisposal;
  }
`;

interface Stack {
  readonly disposed: boolean;
  use<T>(value: T): T;
  adopt<T>(value: T, onDispose: (value: T) => unknown): T;
  defer(onDispose: () => unknown): void;
  move(): this;
}
interface SyncStack extends Stack {
  dispose(): void;
}
interface AsyncStack extends Stack {
  disposeAsync(): Promise<void>;
}
interface Realm {
  DisposableStack: new () => SyncStack;
  AsyncDisposableStack: new () => AsyncStack;
  SuppressedError: new (error: unknown, suppressed: unknown, message?: string) => SuppressedFailure;
  Symbol: { dispose: symbol; asyncDispose: symbol; for(key: string): symbol };
  ReferenceError: ErrorConstructor;
  TypeError: ErrorConstructor;
}
interface SuppressedFailure extends Error {
  error: unknown;
  suppressed: unknown;
}

function realmWithoutExplicitResourceManagement() {
  const context = createContext({});
  runInContext(removeExplicitResourceManagement, context);
  runInContext(source, context);
  return runInContext("globalThis", context) as Realm;
}

function thrownBy(action: () => unknown) {
  try {
    action();
  } catch (error) {
    return error;
  }
  throw new Error("Expected the action to throw");
}

async function rejectionOf(promise: Promise<unknown>) {
  try {
    await promise;
  } catch (error) {
    return error;
  }
  throw new Error("Expected the promise to reject");
}

describe("explicit resource management polyfill", () => {
  it("removes the APIs in the test realm before installing them", () => {
    const context = createContext({});
    runInContext(removeExplicitResourceManagement, context);
    expect(
      runInContext(
        "[typeof DisposableStack, typeof AsyncDisposableStack, typeof SuppressedError, typeof Symbol.dispose, typeof Symbol.asyncDispose]",
        context
      )
    ).toEqual(["undefined", "undefined", "undefined", "undefined", "undefined"]);
  });

  it("installs the globals with the registry symbols compilers fall back to", () => {
    const realm = realmWithoutExplicitResourceManagement();
    expect(typeof realm.DisposableStack).toBe("function");
    expect(typeof realm.AsyncDisposableStack).toBe("function");
    expect(typeof realm.SuppressedError).toBe("function");
    expect(realm.Symbol.dispose).toBe(realm.Symbol.for("Symbol.dispose"));
    expect(realm.Symbol.asyncDispose).toBe(realm.Symbol.for("Symbol.asyncDispose"));
    expect(Object.keys(realm)).not.toContain("DisposableStack");
  });

  it("keeps APIs the engine already provides", () => {
    const context = createContext({});
    runInContext(removeExplicitResourceManagement, context);
    const existing = runInContext(
      `
        Object.defineProperty(Symbol, "dispose", { value: Symbol("native dispose") });
        Object.defineProperty(Symbol, "asyncDispose", { value: Symbol("native asyncDispose") });
        globalThis.DisposableStack = function DisposableStack() {};
        globalThis.AsyncDisposableStack = function AsyncDisposableStack() {};
        globalThis.SuppressedError = function SuppressedError() {};
        [Symbol.dispose, Symbol.asyncDispose, DisposableStack, AsyncDisposableStack, SuppressedError];
      `,
      context
    ) as unknown[];
    runInContext(source, context);
    expect(
      runInContext(
        "[Symbol.dispose, Symbol.asyncDispose, DisposableStack, AsyncDisposableStack, SuppressedError]",
        context
      )
    ).toEqual(existing);
  });

  it("is idempotent when injected into the same frame twice", () => {
    const context = createContext({});
    runInContext(removeExplicitResourceManagement, context);
    runInContext(source, context);
    const installed = runInContext(
      "[DisposableStack, AsyncDisposableStack, SuppressedError, Symbol.dispose]",
      context
    ) as unknown[];
    runInContext(source, context);
    expect(
      runInContext(
        "[DisposableStack, AsyncDisposableStack, SuppressedError, Symbol.dispose]",
        context
      )
    ).toEqual(installed);
  });

  it("creates SuppressedError with non-enumerable error and suppressed", () => {
    const realm = realmWithoutExplicitResourceManagement();
    const failure = new realm.SuppressedError("error", "suppressed", "message");
    expect(failure.name).toBe("SuppressedError");
    expect(failure.message).toBe("message");
    expect(failure.error).toBe("error");
    expect(failure.suppressed).toBe("suppressed");
    expect(Object.keys(failure)).toEqual([]);
  });

  describe("DisposableStack", () => {
    it("disposes used, adopted and deferred resources in reverse order", () => {
      const realm = realmWithoutExplicitResourceManagement();
      const calls: unknown[] = [];
      const resource = {
        [realm.Symbol.dispose]() {
          calls.push(["used", this]);
        },
      };
      const stack = new realm.DisposableStack();
      expect(stack.use(resource)).toBe(resource);
      expect(stack.adopt("handle", (value) => calls.push(["adopted", value]))).toBe("handle");
      stack.defer((...args: unknown[]) => calls.push(["deferred", args]));
      stack.dispose();
      expect(calls).toEqual([
        ["deferred", []],
        ["adopted", "handle"],
        ["used", resource],
      ]);
    });

    it("captures the dispose method when the resource is used", () => {
      const realm = realmWithoutExplicitResourceManagement();
      const calls: string[] = [];
      const resource: Record<symbol, () => void> = {
        [realm.Symbol.dispose]: () => calls.push("original"),
      };
      const stack = new realm.DisposableStack();
      stack.use(resource);
      resource[realm.Symbol.dispose] = () => calls.push("replaced");
      stack.dispose();
      expect(calls).toEqual(["original"]);
    });

    it("disposes only once and exposes the same method under Symbol.dispose", () => {
      const realm = realmWithoutExplicitResourceManagement();
      let disposals = 0;
      const stack = new realm.DisposableStack();
      stack.defer(() => disposals++);
      expect(stack.disposed).toBe(false);
      const disposeBySymbol = (stack as unknown as Record<symbol, () => void>)[
        realm.Symbol.dispose
      ];
      expect(disposeBySymbol).toBe(stack.dispose);
      disposeBySymbol.call(stack);
      stack.dispose();
      expect(stack.disposed).toBe(true);
      expect(disposals).toBe(1);
      expect(Object.prototype.toString.call(stack)).toBe("[object DisposableStack]");
    });

    it("throws ReferenceError when adding to or moving a disposed stack", () => {
      const realm = realmWithoutExplicitResourceManagement();
      const stack = new realm.DisposableStack();
      stack.dispose();
      for (const action of [
        () => stack.use(null),
        () => stack.adopt(1, () => {}),
        () => stack.defer(() => {}),
        () => stack.move(),
      ]) {
        expect(thrownBy(action)).toBeInstanceOf(realm.ReferenceError);
      }
    });

    it("ignores null and undefined but rejects values without a dispose method", () => {
      const realm = realmWithoutExplicitResourceManagement();
      const stack = new realm.DisposableStack();
      expect(stack.use(null)).toBeNull();
      expect(stack.use(undefined)).toBeUndefined();
      expect(thrownBy(() => stack.use({}))).toBeInstanceOf(realm.TypeError);
      expect(thrownBy(() => stack.adopt(1, "not callable" as never))).toBeInstanceOf(
        realm.TypeError
      );
      expect(thrownBy(() => stack.defer(undefined as never))).toBeInstanceOf(realm.TypeError);
      stack.dispose();
    });

    it("keeps disposing after failures and chains them as SuppressedError", () => {
      const realm = realmWithoutExplicitResourceManagement();
      const first = new Error("first");
      const second = new Error("second");
      const third = new Error("third");
      const calls: string[] = [];
      const stack = new realm.DisposableStack();
      stack.defer(() => {
        calls.push("first");
        throw first;
      });
      stack.defer(() => calls.push("clean"));
      stack.defer(() => {
        throw second;
      });
      stack.defer(() => {
        throw third;
      });
      const failure = thrownBy(() => stack.dispose()) as SuppressedFailure;
      expect(calls).toEqual(["clean", "first"]);
      expect(failure).toBeInstanceOf(realm.SuppressedError);
      expect(failure.error).toBe(first);
      const suppressed = failure.suppressed as SuppressedFailure;
      expect(suppressed).toBeInstanceOf(realm.SuppressedError);
      expect(suppressed.error).toBe(second);
      expect(suppressed.suppressed).toBe(third);
    });

    it("rethrows a single failure unwrapped", () => {
      const realm = realmWithoutExplicitResourceManagement();
      const only = new Error("only");
      const stack = new realm.DisposableStack();
      stack.defer(() => {
        throw only;
      });
      expect(thrownBy(() => stack.dispose())).toBe(only);
    });

    it("moves resources to a new stack and marks the original disposed", () => {
      const realm = realmWithoutExplicitResourceManagement();
      const calls: string[] = [];
      const stack = new realm.DisposableStack();
      stack.defer(() => calls.push("a"));
      stack.defer(() => calls.push("b"));
      const moved = stack.move();
      expect(moved).toBeInstanceOf(realm.DisposableStack);
      expect(stack.disposed).toBe(true);
      stack.dispose();
      expect(calls).toEqual([]);
      moved.dispose();
      expect(calls).toEqual(["b", "a"]);
    });
  });

  describe("AsyncDisposableStack", () => {
    it("awaits each disposal in reverse order before starting the next", async () => {
      const realm = realmWithoutExplicitResourceManagement();
      const calls: string[] = [];
      const resource = {
        async [realm.Symbol.asyncDispose]() {
          await Promise.resolve();
          calls.push("used async");
        },
      };
      const stack = new realm.AsyncDisposableStack();
      stack.use(resource);
      stack.adopt("handle", async (value) => {
        calls.push(`adopt start ${value}`);
        await new Promise((resolve) => setTimeout(resolve, 5));
        calls.push(`adopt end ${value}`);
      });
      stack.defer(async () => {
        await Promise.resolve();
        calls.push("deferred");
      });
      await stack.disposeAsync();
      expect(calls).toEqual(["deferred", "adopt start handle", "adopt end handle", "used async"]);
    });

    it("falls back to Symbol.dispose without awaiting its result", async () => {
      const realm = realmWithoutExplicitResourceManagement();
      let resultAwaited = false;
      const resource = {
        [realm.Symbol.dispose]() {
          return {
            then() {
              resultAwaited = true;
            },
          };
        },
      };
      const stack = new realm.AsyncDisposableStack();
      stack.use(resource);
      await expect(stack.disposeAsync()).resolves.toBeUndefined();
      expect(resultAwaited).toBe(false);
    });

    it("rejects values with neither dispose method", () => {
      const realm = realmWithoutExplicitResourceManagement();
      const stack = new realm.AsyncDisposableStack();
      expect(thrownBy(() => stack.use({}))).toBeInstanceOf(realm.TypeError);
      expect(stack.use(null)).toBeNull();
    });

    it("disposes only once and chains rejections as SuppressedError", async () => {
      const realm = realmWithoutExplicitResourceManagement();
      const first = new Error("first");
      const second = new Error("second");
      let disposals = 0;
      const stack = new realm.AsyncDisposableStack();
      stack.defer(() => {
        disposals++;
        return Promise.reject(first);
      });
      stack.defer(() => {
        throw second;
      });
      const disposeBySymbol = (stack as unknown as Record<symbol, () => Promise<void>>)[
        realm.Symbol.asyncDispose
      ];
      expect(disposeBySymbol).toBe(stack.disposeAsync);
      const failure = (await rejectionOf(disposeBySymbol.call(stack))) as SuppressedFailure;
      expect(failure).toBeInstanceOf(realm.SuppressedError);
      expect(failure.error).toBe(first);
      expect(failure.suppressed).toBe(second);
      await stack.disposeAsync();
      expect(disposals).toBe(1);
      expect(Object.prototype.toString.call(stack)).toBe("[object AsyncDisposableStack]");
    });

    it("throws ReferenceError after disposal and moves resources", async () => {
      const realm = realmWithoutExplicitResourceManagement();
      const calls: string[] = [];
      const stack = new realm.AsyncDisposableStack();
      stack.defer(() => calls.push("moved"));
      const moved = stack.move();
      expect(stack.disposed).toBe(true);
      expect(thrownBy(() => stack.defer(() => {}))).toBeInstanceOf(realm.ReferenceError);
      await stack.disposeAsync();
      expect(calls).toEqual([]);
      await moved.disposeAsync();
      expect(calls).toEqual(["moved"]);
    });
  });
});
