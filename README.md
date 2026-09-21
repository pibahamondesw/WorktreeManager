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

| Command                     | Data and credentials             | Signing                                           |
| --------------------------- | -------------------------------- | ------------------------------------------------- |
| `pnpm run tauri:dev:local`  | Isolated; configure Linear again | Default development signature                     |
| `pnpm run tauri:dev:shared` | Shares the installed app's state | Stable certificate; setup below                   |
| `pnpm run tauri dev`        | Shares the installed app's state | Default, unless `WTM_DEV_SIGNING_IDENTITY` is set |

Close the installed app before running shared development: only one instance can own that state.

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

### Build and verify

```bash
pnpm run tauri build
pnpm run typecheck
pnpm exec vitest run src/services/operations.test.ts
CARGO_TARGET_DIR="$HOME/.cache/worktreemanager-target" cargo test --manifest-path src-tauri/Cargo.toml --lib cli::tests
```

Builds default to `~/.cache/worktreemanager-target`; the app bundle is under `release/bundle/macos/`. Override `CARGO_TARGET_DIR` when needed. Run the specific test files relevant to your changes.

## Releasing

```bash
pnpm run release <version>
git push && git push origin v<version>
```

The release script bumps versions, commits, and tags. Pushing the tag triggers the [release workflow](.github/workflows/release.yml), which builds the universal macOS app, publishes updater artifacts, and updates the Homebrew cask. See the [Homebrew guide](homebrew/README.md) for tap setup.
