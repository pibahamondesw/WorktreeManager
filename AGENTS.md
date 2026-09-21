# Working on WorktreeManager

WorktreeManager is a macOS app for managing Git worktrees and Linear issues. It groups repositories into workspaces, creates tasks spanning one or more repositories, and opens them in external editors or embedded editor and AI agent sessions. Built with Tauri 2, Rust, React and TypeScript.

Keep changes small and follow nearby patterns. Prioritize user data, consistent task lifecycles, then UI convenience.

## Where things belong

- A workspace groups peer repositories; a task has one worktree per participating repository. Preserve multi-repo behavior even when working on a single-repo flow.
- UI components and hooks coordinate presentation; shared mutations go through [Operations](src/services/operations.ts) so the UI and CLI keep the same validation, serialization and persistence behavior. Native Git, filesystem and process work belongs in [Rust commands](src-tauri/src/commands/).
- Keep task presentation, generic terminal transport and agent launch/resume logic separate. Agent-specific behavior belongs in [agents](src-tauri/src/commands/agents/).

## Invariants to preserve

- Worktree operations can partially succeed. Persist created tasks before optional setup or opening editors; expose partial results for recovery. Failed disk deletion must retain the app record. Auxiliary cleanup must not turn a successful operation into a failure.
- Keep persisted-shape changes backward compatible and retry-safe. Update the [normalizers](src/utils.ts) alongside types: they whitelist fields and can silently drop additions. Linear credentials belong in Keychain; preserve the [store migration](src/services/store.ts) that verifies secrets before removing legacy copies. Never expose credentials through logs or CLI responses.
- Notes and vault files are user-owned after creation: never overwrite existing content or discard it on task deletion. Archive the task folder with its attachments. Keep [note resolution](src/services/notes.ts) compatible with the [vault kit](vault-kit/README.md).
- [Claude config cleanup](src-tauri/src/commands/claude_config.rs) only removes owned worktree entries whose directories are gone. Preserve unrelated settings, skip malformed config and write atomically; never broaden cleanup to arbitrary missing projects.
- Switching tasks detaches or hides embedded sessions; it does not terminate them. Close/delete/quit owns teardown. Preserve per-task editor isolation and stale-event guards; only the main webview receives [Tauri capabilities](src-tauri/capabilities/default.json).

## Platform and UI conventions

- GUI launches have a minimal environment. Use macOS `open -a` for GUI editors and the shared [shell environment helpers](src-tauri/src/commands/shell_env.rs) for CLI launches and probes. Quote dynamic shell arguments.
- Use nested raw GraphQL queries for bulk Linear reads; SDK relation traversal creates N+1 requests.
- Reuse UI primitives and theme tokens; keep chrome minimal and keyboard navigation usable. Errors, paths and branch names must remain selectable. Use `data-drag-region` with [useWindowDrag](src/hooks/useWindowDrag.ts), not Tauri's built-in drag handler or CSS app regions: native drags lose mouseup events and break subsequent clicks. Keep interactive controls out of drag handling and app shortcuts out of text/terminal input.

## Development and verification

- Default to `pnpm run tauri:dev:local`; shared development uses the installed app's state and credentials. See [development setup](README.md#development).
- Verify behavior changes with focused tests, including relevant failure paths. Run only affected files/modules and direct neighbours, never the full suite. Frontend example: `pnpm exec vitest run src/services/operations.test.ts`. Rust example: `CARGO_TARGET_DIR="$HOME/.cache/worktreemanager-target" cargo test --manifest-path src-tauri/Cargo.toml --lib commands::notes::tests`.
- Run relevant type, lint and formatting checks from [package scripts](package.json). Native drag, focus and embedded webview changes also need a macOS smoke check; report when it was not possible.
- Leave commits, pull requests and releases to the user unless explicitly requested. The release script commits and tags as well as bumping versions.
