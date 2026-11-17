(() => {
  const global = globalThis;
  const defineHidden = (target, key, value) =>
    Object.defineProperty(target, key, { value, writable: true, configurable: true });

  for (const name of ["dispose", "asyncDispose"]) {
    if (typeof Symbol[name] !== "symbol") {
      Object.defineProperty(Symbol, name, { value: Symbol.for(`Symbol.${name}`) });
    }
  }

  if (typeof global.SuppressedError !== "function") {
    class SuppressedError extends Error {
      constructor(error, suppressed, message) {
        super(message);
        defineHidden(this, "error", error);
        defineHidden(this, "suppressed", suppressed);
      }
    }
    defineHidden(SuppressedError.prototype, "name", "SuppressedError");
    defineHidden(global, "SuppressedError", SuppressedError);
  }

  const requireCallable = (callback) => {
    if (typeof callback !== "function") throw new TypeError("Disposer is not a function");
    return callback;
  };

  const chainError = (state, error) => {
    state.error = state.failed ? new global.SuppressedError(error, state.error) : error;
    state.failed = true;
  };

  const installStack = (Stack, disposeSymbol, disposeMethod) => {
    defineHidden(Stack.prototype, disposeSymbol, Stack.prototype[disposeMethod]);
    Object.defineProperty(Stack.prototype, Symbol.toStringTag, {
      value: Stack.name,
      configurable: true,
    });
    defineHidden(global, Stack.name, Stack);
  };

  if (typeof global.DisposableStack !== "function") {
    class DisposableStack {
      #disposers = [];
      #disposed = false;

      get disposed() {
        return this.#disposed;
      }

      #requireOpen() {
        if (this.#disposed) throw new ReferenceError("DisposableStack already disposed");
      }

      use(value) {
        this.#requireOpen();
        if (value == null) return value;
        const dispose = requireCallable(value[Symbol.dispose]);
        this.#disposers.push(() => dispose.call(value));
        return value;
      }

      adopt(value, onDispose) {
        this.#requireOpen();
        requireCallable(onDispose);
        this.#disposers.push(() => onDispose(value));
        return value;
      }

      defer(onDispose) {
        this.#requireOpen();
        requireCallable(onDispose);
        this.#disposers.push(() => onDispose());
      }

      move() {
        this.#requireOpen();
        const moved = new DisposableStack();
        moved.#disposers = this.#disposers;
        this.#disposers = [];
        this.#disposed = true;
        return moved;
      }

      dispose() {
        if (this.#disposed) return;
        this.#disposed = true;
        const disposers = this.#disposers;
        this.#disposers = [];
        const state = { failed: false, error: undefined };
        while (disposers.length) {
          try {
            disposers.pop()();
          } catch (error) {
            chainError(state, error);
          }
        }
        if (state.failed) throw state.error;
      }
    }
    installStack(DisposableStack, Symbol.dispose, "dispose");
  }

  if (typeof global.AsyncDisposableStack !== "function") {
    class AsyncDisposableStack {
      #disposers = [];
      #disposed = false;

      get disposed() {
        return this.#disposed;
      }

      #requireOpen() {
        if (this.#disposed) throw new ReferenceError("AsyncDisposableStack already disposed");
      }

      use(value) {
        this.#requireOpen();
        if (value == null) return value;
        const disposeAsync = value[Symbol.asyncDispose];
        if (disposeAsync !== undefined && disposeAsync !== null) {
          requireCallable(disposeAsync);
          this.#disposers.push(() => disposeAsync.call(value));
        } else {
          const dispose = requireCallable(value[Symbol.dispose]);
          this.#disposers.push(() => {
            dispose.call(value);
          });
        }
        return value;
      }

      adopt(value, onDispose) {
        this.#requireOpen();
        requireCallable(onDispose);
        this.#disposers.push(() => onDispose(value));
        return value;
      }

      defer(onDispose) {
        this.#requireOpen();
        requireCallable(onDispose);
        this.#disposers.push(() => onDispose());
      }

      move() {
        this.#requireOpen();
        const moved = new AsyncDisposableStack();
        moved.#disposers = this.#disposers;
        this.#disposers = [];
        this.#disposed = true;
        return moved;
      }

      async disposeAsync() {
        if (this.#disposed) return;
        this.#disposed = true;
        const disposers = this.#disposers;
        this.#disposers = [];
        const state = { failed: false, error: undefined };
        while (disposers.length) {
          try {
            await disposers.pop()();
          } catch (error) {
            chainError(state, error);
          }
        }
        if (state.failed) throw state.error;
      }
    }
    installStack(AsyncDisposableStack, Symbol.asyncDispose, "disposeAsync");
  }
})();
