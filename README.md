# WorktreeManager

![WorktreeManager logo](assets/logo.png)

A macOS app for managing Git worktrees and Linear issues, built with Tauri, React, and TypeScript.

- Group local repositories into workspaces; create a task with one worktree per repository.
- Create branches from Linear issues and track Git status and linked PRs.
- Open tasks in Cursor, VS Code, Zed, OpenCode, or embedded Claude Code and Codex terminals.
- Use an independent embedded VS Code editor per task (macOS 14+).
- Keep optional Obsidian task notes, archived when tasks are removed.
- Manage workspaces and tasks from the UI or the local `wtm` CLI.

## Installation

```bash
brew install pibahamondesw/tap/worktreemanager
wtm
```

You can also launch from Spotlight. The app offers updates automatically; `brew upgrade` works too. If Homebrew asks you to trust the tap, run `brew trust pibahamondesw/tap`.

The app is not notarized. Homebrew handles download quarantine; for a direct download, right-click the app and choose **Open** on first launch.

On first launch, configure Linear with a [personal API key](https://linear.app/settings/api), then add a workspace with your local repositories. Each repository has a configurable worktree directory, defaulting to `~/Documents/.worktreemanager/worktrees/<repo-name-slug>`.

## Local CLI

Homebrew installs `wtm` alongside the app. Run `wtm` to open it and `wtm --help` for commands, JSON inputs, and examples. Commands operate on the running app after it finishes loading. Manage Linear credentials in the UI; the CLI never accepts or returns them.

Task setup copies configured editor/environment files from the source repository and runs its dependency setup; it can require network access. It does not transfer another task's code or sessions. If creation or deletion fails, inspect the reported partial results before retrying.

Python setup runs `uv sync --locked` when the repository root contains both `pyproject.toml` and `uv.lock`. Otherwise, a standalone `requirements.txt` uses `python3 -m venv .venv` and installs into that environment with pip. Setup preserves existing `.venv` paths and skips other Python managers or ambiguous configurations. It requires uv or Python 3 to be installed; failures produce a warning without removing the task. This applies to newly created tasks; existing worktrees need their repository's setup run manually.

## Editors and notes

For embedded Codex, install Codex CLI (`brew install --cask codex`), select **Codex**, and open a task. Sign in from the terminal if needed. It resumes the latest conversation in that worktree, or starts a new one when none exists. Claude and Codex keep separate sessions when you switch agents.

For embedded VS Code, select **VS Code embedded** and click **Install editor** when opening a task. Editors and agent terminals keep running while you switch tasks. Close them explicitly, delete the task, or quit the app to end their sessions.

Enable **Obsidian vault** from the sidebar to create task notes in `~/Documents/worktreemanager-vault`. Removing a task archives its notes; disabling the vault leaves files untouched. For agent setup and vault customization, see the [vault guide](vault-kit/README.md).

## Navigation

Use `↑` / `↓` or `j` / `k` to select tasks, `Enter` to open one, and `⌘[` to return. `⌘K` opens the command palette; `⌘F` searches the current workspace. In the embedded editor, use `⌘⌥P` for search. `L` opens the selected issue and `O` its notes.

## Development

Requires Rust, Node.js 22+, the pnpm version pinned in [package.json](package.json), and Xcode Command Line Tools (`xcode-select --install`).

```bash
git clone https://github.com/pibahamondesw/WorktreeManager.git
cd WorktreeManager
pnpm install
pnpm run tauri:dev:local
```

Development starts Vite on `localhost:5173` and opens the native app with hot reload. See [AGENTS.md](AGENTS.md) for contribution conventions; track features and planned work in Linear.

| Command                       | Data and credentials                                                          | Signing                                           |
| ----------------------------- | ----------------------------------------------------------------------------- | ------------------------------------------------- |
| `pnpm run tauri:dev:local`    | Isolated; configure Linear again                                              | Default development signature                     |
| `pnpm run tauri:dev:shared`   | Shares the installed app's state                                              | Stable certificate; setup below                   |
| `pnpm run tauri:dev:snapshot` | Fresh copy of the installed app's state and Linear credentials on each launch | Stable certificate; setup below                   |
| `pnpm run tauri dev`          | Shares the installed app's state                                              | Default, unless `WTM_DEV_SIGNING_IDENTITY` is set |

Close the installed app before running shared development: only one instance can own that state. Snapshot development runs alongside it; its changes never reach the installed app.

To use the development CLI:

```bash
alias wtm="$HOME/.cache/worktreemanager-target/debug/worktree-manager --cli"
wtm --local workspace list
```

Omit `--local` for the installed/shared profile. Bare `wtm` with this alias launches the development binary and requires its Vite server to be running.

### Shared development signing

A stable signing identity avoids changing signatures on every Rust rebuild. Create it once:

1. In **Keychain Access → Certificate Assistant → Create a Certificate**, use name **WorktreeManager Development**, identity **Self Signed Root**, and type **Code Signing**.
2. Open the certificate's **Trust** settings and set **Code Signing → Always Trust**. Keep its associated private key.
3. Verify it appears in `security find-identity -v -p codesigning`, then run `pnpm run tauri:dev:shared`.

Set `WTM_DEV_SIGNING_IDENTITY` to another certificate's name or SHA-1 hash to override the default. macOS may still ask to authorize signing or access to existing Linear credentials. For Keychain errors, use **Dependencies → Re-check** after authorizing access; certificate trust and credential access are separate permissions.

### Typed linting

Run `pnpm run lint:ts` to check the frontend, Node scripts and Vite/Vitest configuration. Type-aware rules catch unhandled promises, misused async callbacks and unsafe `any` usage.

### Unused code and dependencies pilot

Run `pnpm run knip` to report unused files, exports, types and dependencies. It exits with code 1 when findings exist. For an informational run, use `pnpm run knip --no-exit-code`.

[Knip configuration](knip.json) scopes analysis to frontend sources (including CSS), Node scripts and TypeScript configuration files. Knip's Vite plugin discovers the React entry from [index.html](index.html); its Vitest plugin includes tests and their helpers, and package scripts identify the release entry. ESLint configuration and its dependencies are discovered automatically. Rust sources, generated native bindings, vault templates and Semgrep fixtures are outside this scope. Tauri command names passed to `invoke` are native boundaries, not TypeScript imports.

CI runs the informational command in the existing frontend job whenever its change filter matches. Findings remain visible in the job log without blocking this pilot; execution/configuration errors still fail the step. No automatic cleanup or blanket suppressions are enabled. Review references, test-only use and native integration before removing any reported item. See [Knip's CI guidance](https://knip.dev/guides/using-knip-in-ci).

Initial baseline on 2026-10-06, Node 22.23.2 / Knip 6.40.0: 0 unused files, 2 unused dependencies (`@tauri-apps/plugin-shell` and `@types/uuid`), 12 unused exports and 13 unused exported types. These are candidates for review, not approved deletions. Gate only validated categories after reviewing this baseline.

Reviewed and resolved all 27 findings on 2026-10-06: made module-local exports private, removed unreferenced UI wrappers/helpers and a type, and removed the two unused JavaScript dependencies. The native Rust shell plugin remains registered. The current scan reports no findings; CI remains informational.

Added scan cost measured locally on macOS with dependencies installed, without Knip caching: 0.617 s, 0.532 s and 0.543 s across three runs (median 0.543 s). This excludes dependency installation and does not measure Linux CI runtime. Roll back the CI integration by removing the **Unused code and dependencies (pilot report)** step; local scanning remains available.

Verify the scanner integration with `pnpm exec vitest run scripts/knip.test.ts`.

### Build and verify

```bash
pnpm run tauri build
pnpm run typecheck
pnpm exec vitest run src/services/operations.test.ts
CARGO_TARGET_DIR="$HOME/.cache/worktreemanager-target" cargo test --manifest-path src-tauri/Cargo.toml --lib cli::tests
```

Builds default to `~/.cache/worktreemanager-target`; the app bundle is under `release/bundle/macos/`. Override `CARGO_TARGET_DIR` when needed. Run the specific test files relevant to your changes.

### Frontend coverage pilot

```bash
pnpm run test:coverage
```

Runs only the store, utils and operations test files with Vitest V8. [Coverage configuration](vitest.config.ts) includes all TypeScript/TSX sources, even files these tests never import, and excludes tests and declaration files. Zeros elsewhere describe this focused pilot, not the coverage of the full test suite.

Reports are written to the ignored coverage directory: [HTML with uncovered lines and branches](coverage/index.html), [LCOV](coverage/lcov.info) and [JSON summary](coverage/coverage-summary.json), plus the console table. Coverage is opt-in; normal test runs do not collect it. Upgrade Vitest and its coverage provider together; their versions must match.

CI runs the same pilot in **Frontend coverage (pilot report)** when the existing frontend change filter matches, including fork PRs, with read-only permissions and no secrets or external coverage service. Download the **frontend-coverage** artifact from that workflow run; retention is seven days. Reports are also uploaded after test failures when available. A skipped job has no artifact and means **not measured**, not 0%; failed/cancelled runs without reports also have no measurement. Do not substitute artifacts from older runs.

Baseline on 2026-10-02, Node 22.23.2 / Vitest 4.1.11, 114 tests:

| Source                                   | Lines            | Branches         | Proposed minimum lines | Proposed minimum branches |
| ---------------------------------------- | ---------------- | ---------------- | ---------------------- | ------------------------- |
| [store](src/services/store.ts)           | 90% (117/130)    | 77.77% (56/72)   | 90%                    | 77%                       |
| [utils](src/utils.ts)                    | 97.36% (74/76)   | 88.8% (111/125)  | 97%                    | 88%                       |
| [operations](src/services/operations.ts) | 90.56% (240/265) | 82.84% (140/169) | 90%                    | 82%                       |

Start with reporting only. The proposed ratchet uses per-file lines/branches floors rounded down from this baseline, raised as tests improve. Validate them on Linux CI before enabling `coverage.thresholds` for these three exact paths; the pilot's global percentage is not a suitable gate. Keep the test selection fixed when comparing results.

To preview the utils floor locally:

```bash
pnpm exec vitest run src/services/store.test.ts src/utils.test.ts src/services/operations.test.ts --coverage --coverage.include=src/utils.ts --coverage.thresholds.lines=97 --coverage.thresholds.branches=88 --coverage.reportsDirectory=coverage/ratchet-preview
```

Adding `-t timeAgo` to that preview failed the same thresholds; restoring the full pilot selection passed. Utils reaches this baseline through the combined pilot: its own test file alone measures 93.42% lines / 83.2% branches. Four pilot runs produced identical JSON coverage metrics. On macOS arm64, three alternating timed pairs had median wall times of 1.31 s without coverage and 1.86 s with coverage (+0.55 s, about 42%). This excludes CI provisioning, dependency installation and artifact upload; hosted runner cost remains to be measured.

To roll back the CI integration, remove the `frontend-coverage` job from the [CI workflow](.github/workflows/ci.yml); ordinary tests remain available.

## Releasing

```bash
pnpm run release <version>
git push && git push origin v<version>
```

The release script bumps versions, commits, and tags. Pushing the tag triggers the [release workflow](.github/workflows/release.yml), which builds the universal macOS app, publishes updater artifacts, and updates the Homebrew cask. See the [Homebrew guide](homebrew/README.md) for tap setup.
