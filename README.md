# WorktreeManager

![WorktreeManager logo](assets/logo.png)

A lightweight macOS desktop app for managing Git worktrees with Linear integration. Built with Tauri v2, React, and TypeScript.

## Features

- **Project management** — Add and organize your local repositories
- **Linear integration** — Search issues, auto-create branches, set issues as "In Progress"
- **Worktree management** — Create, open, and delete Git worktrees from a clean UI
- **Multi-editor support** — Open worktrees in Cursor, VS Code, OpenCode, or Claude Code
- **PR tracking** — See linked GitHub PRs on worktree cards, or create new PRs in one click
- **Git status** — See ahead/behind counts, dirty state, and worktree age at a glance
- **Keyboard-driven** — Navigate with arrows/numbers, shortcuts for all common actions
- **Themes** — 8 built-in color themes (Indigo, Ocean, Forest, Sunset, Rose, Nord, Dracula, Monochrome)

## Installation

Install with [Homebrew](https://brew.sh):

```bash
brew install pibahamondesw/tap/worktreemanager
```

Then launch it from Spotlight or `/Applications`.

> [!NOTE]
> Homebrew 6+ asks you to confirm trust for third-party taps the first time. If
> the install is refused non-interactively, trust the tap once up front:
> `brew trust pibahamondesw/tap`.

**The app keeps itself up to date.** On startup it checks for a newer release and
offers to download and install it, so you don't need to reinstall to upgrade.
(Prefer Homebrew's own flow? `brew upgrade` works too.)

> [!NOTE]
> The app isn't notarized by Apple yet. Homebrew strips the download quarantine
> on install, so it launches normally. If you ever download the app tarball
> directly instead of using Homebrew, right-click the app and choose **Open** the
> first time.

## Building from source

Only needed for development, or if you'd rather not use Homebrew.

### Prerequisites

1. **Rust** (1.77.2+) — Install via [rustup](https://rustup.rs/):

   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   source "$HOME/.cargo/env"
   ```

2. **Node.js** (18+) — Install from [nodejs.org](https://nodejs.org/) or via a version manager like [nvm](https://github.com/nvm-sh/nvm)

3. **Xcode Command Line Tools** — Required for the Rust/C toolchain and includes Git:

   ```bash
   xcode-select --install
   ```

4. **An editor** (at least one):
   - [Cursor](https://cursor.sh/) — AI-powered code editor
   - [VS Code](https://code.visualstudio.com/) — Microsoft's code editor
   - [OpenCode](https://opencode.ai/) — AI-native desktop editor
   - [Claude Code](https://docs.anthropic.com/en/docs/claude-code) — Anthropic's CLI coding agent (runs in Terminal)

### Build

```bash
git clone https://github.com/pibahamondesw/WorktreeManager.git && cd WorktreeManager
npm install
npm run tauri build
cp -R src-tauri/target/release/bundle/macos/WorktreeManager.app /Applications/
```

### Development mode

To run the app with hot-reload:

```bash
npm run tauri dev
```

This starts the Vite dev server on `localhost:5173` and opens the Tauri window pointed at it.

## Releasing

Releases are automated by [`.github/workflows/release.yml`](.github/workflows/release.yml).
To cut a release:

```bash
npm run release 0.2.0       # bumps version in all 3 files, commits, tags v0.2.0
git push && git push origin v0.2.0
```

Pushing the tag builds a universal macOS app on CI, publishes a GitHub Release
(the `.app` tarball + updater signature + `latest.json`), and bumps the Homebrew
cask. See
[`homebrew/README.md`](homebrew/README.md) for the one-time tap setup and the
signing secrets involved.

## First Launch

On first launch, the app shows a setup wizard:

1. **Linear** — Generate a Personal API key at [Linear Settings > API](https://linear.app/settings/api)

Then add a project by providing a name and selecting the local repository folder.

By default, new projects store worktrees in:

`~/Documents/.worktreemanager/worktrees/<project-slug>/<branch-name>`

## Keyboard Shortcuts

| Key               | Action                           |
| ----------------- | -------------------------------- |
| `↑` `↓` / `j` `k` | Navigate worktree cards          |
| `1`–`9`           | Jump to worktree by number       |
| `Enter`           | Open selected worktree in editor |
| `N`               | New worktree                     |
| `P`               | Add new project                  |
| `R`               | Refresh Linear info              |
| `L`               | Open selected issue on Linear    |
| `⌘B`              | Copy branch name                 |
| `⌘⇧C`             | Copy worktree path               |
| `⌘D`              | Delete selected worktree         |
| `Esc`             | Clear selection                  |

## Tests

Run the unit test suite with:

```bash
npm test
```

For watch mode during development:

```bash
npm run test:watch
```

Tests cover pure utility functions (time formatting, PR extraction from Linear attachments, keyboard shortcut parsing, and store data migrations). They use [Vitest](https://vitest.dev/) and require no external services or API keys.

## Tech Stack

- **Tauri v2** — Native desktop shell (~5-10 MB app)
- **React 19 + TypeScript** — Frontend UI
- **Tailwind CSS v4** — Dark-mode styling with theme support
- **@linear/sdk** — Linear API integration (raw GraphQL for performance)
