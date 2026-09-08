# WorktreeManager

![WorktreeManager logo](assets/logo.png)

A lightweight macOS desktop app for managing Git worktrees with Linear integration. Built with Tauri v2, React, and TypeScript.

## Features

- **Project management** — Add and organize your local repositories
- **Linear integration** — Search issues, auto-create branches, set issues as "In Progress"
- **Worktree management** — Create, open, and delete Git worktrees from a clean UI
- **Multi-editor support** — Open worktrees in Cursor, VS Code, Zed, OpenCode, or Claude Code
- **Embedded agent terminal** — With Claude Code as the editor, each task opens inside the app; the session keeps running in the background while you switch tasks
- **PR tracking** — See linked GitHub PRs on worktree cards, or create new PRs in one click
- **Git status** — See ahead/behind counts, dirty state, and worktree age at a glance
- **Obsidian task logs** — Optional per-task note in your vault, created and archived with the task
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
> The app isn't notarized by Apple yet. The Homebrew cask strips the download
> quarantine on install (via a `postflight`), so it launches normally. If you
> instead download the app tarball directly, right-click the app and choose
> **Open** the first time to get past Gatekeeper.

## Building from source

Only needed for development, or if you'd rather not use Homebrew.

### Prerequisites

1. **Rust** (1.77.2+) — Install via [rustup](https://rustup.rs/):

   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   source "$HOME/.cargo/env"
   ```

2. **Node.js** (18+) — Install from [nodejs.org](https://nodejs.org/) or via a version manager like [nvm](https://github.com/nvm-sh/nvm)

   This project uses [pnpm](https://pnpm.io/) as its package manager. The easiest way to get the pinned version is Corepack (bundled with Node):

   ```bash
   corepack enable
   ```

3. **Xcode Command Line Tools** — Required for the Rust/C toolchain and includes Git:

   ```bash
   xcode-select --install
   ```

4. **An editor** (at least one):
   - [Cursor](https://cursor.sh/) — AI-powered code editor
   - [VS Code](https://code.visualstudio.com/) — Microsoft's code editor
   - [Zed](https://zed.dev/) — high-performance code editor
   - [OpenCode](https://opencode.ai/) — AI-native desktop editor
   - [Claude Code](https://docs.anthropic.com/en/docs/claude-code) — Anthropic's CLI coding agent (runs in a terminal embedded in the app, one session per task)

### Build

```bash
git clone https://github.com/pibahamondesw/WorktreeManager.git && cd WorktreeManager
pnpm install
pnpm run tauri build
cp -R src-tauri/target/release/bundle/macos/WorktreeManager.app /Applications/
```

### Development mode

All Tauri development commands start Vite on `localhost:5173` and open the native app with hot reload.

| Command                     | App data and Linear credentials                           | Development signing                                                     |
| --------------------------- | --------------------------------------------------------- | ----------------------------------------------------------------------- |
| `pnpm run tauri:dev:shared` | Shares the installed app's store and Keychain entry       | Signs each launch with a stable certificate; setup below                |
| `pnpm run tauri:dev:local`  | Separate store and Keychain entry; configure Linear again | Default development signature                                           |
| `pnpm run tauri dev`        | Shares the installed app's store and Keychain entry       | Default development signature, unless `WTM_DEV_SIGNING_IDENTITY` is set |

The shared identifier is `com.worktreemanager.dev`; the isolated identifier is `com.worktreemanager.dev.local`. Sharing includes settings, workspaces, tasks, and Linear credentials. Close the installed app before running shared development: both instances can write to the same store.

#### One-time signing setup for shared development

The default development signature can change when the Rust executable is rebuilt. A stable code-signing certificate lets you retain the same signing identity across builds. It does not automatically grant access to an existing Keychain item.

1. Open **Keychain Access** from Terminal:

   ```bash
   open -a "Keychain Access"
   ```

2. In the macOS menu bar at the top of the screen, next to the Apple menu, choose **Keychain Access → Certificate Assistant → Create a Certificate…**. Set:
   - **Name:** `WorktreeManager Development`
   - **Identity Type:** `Self Signed Root`
   - **Certificate Type:** `Code Signing`

   Click **Create**, accept the confirmation if shown, then **Done**. If asked for a keychain, choose **login**; some macOS versions do not show that prompt. Keep this certificate and its private key on your Mac.

3. Find **WorktreeManager Development** in Keychain Access and double-click the certificate. Expand **Trust** and set only **Code Signing → Always Trust**. Close the window and authenticate if prompted.

4. Verify the identity is usable:

   ```bash
   security find-identity -v -p codesigning
   ```

   The output should list **WorktreeManager Development** among the valid identities. If it reports `0 valid identities found`, use the troubleshooting steps below before launching.

#### Run with the installed app's data

```bash
pnpm run tauri:dev:shared
```

The Cargo runner signs and verifies the executable before each launch, including Rust hot reloads. Missing identities or failed verification stop the app from opening. The runner does not copy or export credentials or modify the installed app.

The default certificate name is `WorktreeManager Development`. To select another existing code-signing identity, use its exact name or SHA-1 hash from `security find-identity`:

```bash
WTM_DEV_SIGNING_IDENTITY="Your signing identity" pnpm run tauri:dev:shared
```

This variable selects a certificate; it does not create one. The ordinary command can also use it:

```bash
WTM_DEV_SIGNING_IDENTITY="WorktreeManager Development" pnpm run tauri dev
```

macOS may ask to authorize use of the signing private key and access to the existing Linear credentials. When approving the expected `codesign` or development executable request, choose **Always Allow** to retain that authorization. If Linear credentials cannot be read, open **Dependencies**: Doctor displays the Keychain error and **Re-check** retries access without restarting the app.

#### Troubleshooting signing and Keychain access

- **`no identity found`:** Check that `WTM_DEV_SIGNING_IDENTITY` matches an existing code-signing certificate's name or hash. Use **My Certificates** in Keychain Access to confirm the certificate has an associated private key.
- **`0 valid identities found`:** Run `security find-identity -p codesigning` without `-v` to include invalid identities. If yours shows `CSSMERR_TP_NOT_TRUSTED`, apply **Trust → Code Signing → Always Trust** as described above, then check again with `-v`.
- **No keychain selection during certificate creation:** The missing prompt alone is not an error. Search for the certificate in Keychain Access and verify it using `security find-identity`.
- **Signing succeeds but access to Linear credentials is denied:** Certificate trust and permission to read an existing Keychain item are separate. Inspect Doctor's error, authorize the development executable when macOS prompts, and use **Re-check**. An `ACL partition mismatch` means the item's access policy does not match the requesting executable; changing the certificate name or re-entering the Linear token does not resolve that policy mismatch.

Apple documents [creating self-signed certificates](https://support.apple.com/en-ie/guide/keychain-access/kyca8916/mac), [changing certificate trust](https://support.apple.com/en-au/guide/keychain-access/kyca11871/mac), and [code-signing identity stability](https://developer.apple.com/library/archive/technotes/tn2206/_index.html).

## Releasing

Releases are automated by [`.github/workflows/release.yml`](.github/workflows/release.yml).
To cut a release:

```bash
pnpm run release 0.2.0       # bumps version in all 3 files, commits, tags v0.2.0
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

| Key               | Action                             |
| ----------------- | ---------------------------------- |
| `↑` `↓` / `j` `k` | Navigate worktree cards            |
| `0`–`9`           | Jump to worktree by index          |
| `⌘0`–`⌘9`         | Jump to project by displayed index |
| `⌘+` / `⌘−`       | Zoom in / out (80%–150%)           |
| `Enter`           | Open selected worktree in editor   |
| `⌘[`              | Back to the task list from an open task |
| `N`               | New worktree                       |
| `P`               | Add new project                    |
| `R`               | Refresh Linear info                |
| `L`               | Open selected issue on Linear      |
| `O`               | Open task notes in Obsidian        |
| `⌘B`              | Copy branch name                   |
| `⌘⇧C`             | Copy worktree path                 |
| `⌘D`              | Delete selected worktree           |
| `Esc`             | Clear selection                    |

## Obsidian vault

The app knows more about a unit of work than anything else on your machine: a task is a branch, a Linear issue, and one worktree per member repo. Delete the worktree and all of that context goes with it — including why you took an approach, what you tried and rejected, and what you learned.

Enable the vault (setup wizard, or **sidebar footer → Obsidian vault**) and the app scaffolds a full Obsidian vault at `~/Documents/worktreemanager-vault` and registers it with Obsidian — a guide (`AGENTS.md`), templates, project scripts, a `projects/` layer for work that spans tickets and repos, and a `task-logs/` folder the app keeps in sync:

- **Created** with the task, with the issue, branch, repos, and worktree paths already in frontmatter.
- **Opened** with `O` or **More actions → Open notes**.
- **Archived** when you delete the task or remove the workspace — moved with their task folder to `task-logs/_archive/<task-id>/` with `status: archived`, body untouched.

Every new task gets its own `task-logs/<task-id>/` folder, so repeated branch names have separate notes. All notes are archived, including untouched templates. Existing flat notes keep their paths. Startup installs missing task-note support and appends a skill reference to the vault guide while preserving custom instructions.

To make agents write and read the vault, add the one-liner from the vault settings modal to your AI tool's global instructions — it points at `<vault>/agent-setup.md`, which works with any agent that supports global instructions. Details and the optional `/task-log` Claude Code skill live in [`vault-kit/README.md`](vault-kit/README.md) — the agent wiring is the part that decides whether this survives past week three.

You can skip the vault during initial setup and enable it later from the sidebar. Missing vault folders and the Obsidian registry are created automatically, even before Obsidian has been opened for the first time. If creation or registration fails, settings shows the error and lets you retry.

Leave the vault disabled and the whole feature stays off. Disabling it later touches nothing on disk.

## Tests

Run the unit test suite with:

```bash
pnpm test
```

For watch mode during development:

```bash
pnpm run test:watch
```

Tests cover pure utility functions (time formatting, PR extraction from Linear attachments, keyboard shortcut parsing, and store data migrations). They use [Vitest](https://vitest.dev/) and require no external services or API keys.

## Tech Stack

- **Tauri v2** — Native desktop shell (~5-10 MB app)
- **React 19 + TypeScript** — Frontend UI
- **Tailwind CSS v4** — Dark-mode styling with theme support
- **xterm.js + portable-pty** — Embedded agent terminals
- **@linear/sdk** — Linear API integration (raw GraphQL for performance)

Interface zoom scales text, icons, and spacing, and is remembered across app launches. The titlebar also provides zoom controls; click the percentage to reset. `⌘=` also zooms in, and zoom shortcuts accept `Ctrl` in place of `⌘`. `⌘0`–`⌘9` keep their existing project navigation behavior.
