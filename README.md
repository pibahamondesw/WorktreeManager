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

## Local Installation

### Prerequisites

Install the following before building:

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

### Build & Install

```bash
# 1. Clone the repository
git clone <repo-url> && cd WorktreeManager

# 2. Install Node dependencies
npm install

# 3. Build the production app
npm run tauri build
```

This compiles the Rust backend, bundles the React frontend, and produces a macOS `.app` bundle.

### Install to Applications

After building, copy the app to your Applications folder:

```bash
cp -R src-tauri/target/release/bundle/macos/WorktreeManager.app /Applications/
```

You can then launch it from Spotlight or from `/Applications`.

### Development Mode

To run the app in development with hot-reload:

```bash
npm run tauri dev
```

This starts the Vite dev server on `localhost:5173` and opens the Tauri window pointed at it.

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
